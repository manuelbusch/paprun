//! Laden eines PAP aus dem YAML-Format (Gegenstück zu [`crate::emit`]).
//!
//! Zahlen werden aus ihrer Textdarstellung übernommen, nie über `f64` —
//! sonst verlöre `932.30` seine Nachkommastelle.

use crate::ast::{MethodDef, Pap, Stmt, Ty, VarDecl, VarId, VarKind};
use crate::error::Error;
use crate::eval::const_eval;
use crate::parse::{NoScope, parse_assign, parse_expr};
use crate::value::Value;
use bigdecimal::BigDecimal;
use std::collections::HashMap;
use std::sync::Arc;
use yaml_rust2::{Yaml, YamlLoader};

/// Lädt einen PAP aus einem YAML-Dokument.
pub fn load(source: &str) -> Result<Pap, Error> {
    let docs = YamlLoader::load_from_str(source)
        .map_err(|e| Error::load(format!("ungültiges YAML: {e}"), ""))?;
    let doc = docs
        .first()
        .ok_or_else(|| Error::load("YAML-Dokument ist leer", ""))?;
    if doc.as_hash().is_none() {
        return Err(Error::load(
            "YAML-Abbildung auf oberster Ebene erwartet",
            "",
        ));
    }

    let mut vars: Vec<VarDecl> = Vec::new();
    let mut by_name: HashMap<String, VarId> = HashMap::new();

    for (section, kind) in [
        ("inputs", VarKind::Input),
        ("outputs", VarKind::Output),
        ("internals", VarKind::Internal),
        ("constants", VarKind::Constant),
    ] {
        let node = &doc[section];
        if node.is_badvalue() {
            continue;
        }
        let entries = node
            .as_hash()
            .ok_or_else(|| Error::load(format!("`{section}` muss eine Abbildung sein"), section))?;
        for (name, spec) in entries {
            let name = scalar_text(name).ok_or_else(|| {
                Error::load(
                    format!("Variablenname in `{section}` ist kein Text"),
                    section,
                )
            })?;
            let decl = read_decl(&name, spec, kind)?;
            if by_name.insert(name.clone(), vars.len() as VarId).is_some() {
                return Err(Error::load(
                    format!("Variable `{name}` ist mehrfach deklariert"),
                    section,
                ));
            }
            vars.push(decl);
        }
    }

    // Methoden zuerst registrieren, damit `call` vorwärts auflösen kann.
    let method_entries: Vec<(String, &Yaml)> = match doc["methods"].as_hash() {
        Some(hash) => hash
            .iter()
            .map(|(name, body)| {
                scalar_text(name)
                    .map(|name| (name, body))
                    .ok_or_else(|| Error::load("Methodenname ist kein Text", "methods"))
            })
            .collect::<Result<_, _>>()?,
        None => Vec::new(),
    };
    let method_ids: HashMap<String, u32> = method_entries
        .iter()
        .enumerate()
        .map(|(index, (name, _))| (name.clone(), index as u32))
        .collect();

    let mut methods = Vec::with_capacity(method_entries.len());
    for (name, body) in &method_entries {
        methods.push(MethodDef {
            name: name.clone(),
            body: read_block(body, &by_name, &method_ids, name)?,
        });
    }

    let main = read_block(&doc["main"], &by_name, &method_ids, "main")?;

    Ok(Pap {
        name: scalar_text(&doc["name"]).unwrap_or_default(),
        version: scalar_text(&doc["version"]).unwrap_or_default(),
        vars,
        by_name,
        methods,
        main,
    })
}

/// Textdarstellung eines Skalars — für Zahlen der unveränderte Quelltext.
fn scalar_text(node: &Yaml) -> Option<String> {
    match node {
        Yaml::String(text) => Some(text.clone()),
        Yaml::Real(text) => Some(text.clone()),
        Yaml::Integer(value) => Some(value.to_string()),
        Yaml::Boolean(value) => Some(value.to_string()),
        _ => None,
    }
}

fn read_decl(name: &str, spec: &Yaml, kind: VarKind) -> Result<VarDecl, Error> {
    let ty_text = scalar_text(&spec["type"]).ok_or_else(|| {
        Error::load(
            format!("`{name}` braucht ein Feld `type`"),
            name.to_string(),
        )
    })?;
    let ty = Ty::from_xml(&ty_text)
        .ok_or_else(|| Error::load(format!("unbekannter Typ `{ty_text}`"), name.to_string()))?;

    // Konstanten tragen ihren Wert unter `value`, alles Übrige unter `default`.
    let value_node = if kind == VarKind::Constant {
        &spec["value"]
    } else {
        &spec["default"]
    };

    let default = if value_node.is_badvalue() {
        if kind == VarKind::Constant {
            return Err(Error::load(
                format!("Konstante `{name}` braucht ein Feld `value`"),
                name.to_string(),
            ));
        }
        zero_of(ty)
    } else if ty == Ty::DecArr {
        let items = value_node.as_vec().ok_or_else(|| {
            Error::load(format!("`{name}` erwartet eine Liste"), name.to_string())
        })?;
        let values: Result<Vec<Value>, Error> = items
            .iter()
            .map(|item| parse_scalar(item, Ty::Dec, name))
            .collect();
        Value::Arr(Arc::new(values?))
    } else {
        parse_scalar(value_node, ty, name)?
    };

    Ok(VarDecl {
        name: name.to_string(),
        kind,
        ty,
        default,
        group: scalar_text(&spec["group"]).unwrap_or_default(),
    })
}

/// Wert eines Skalars gemäß deklariertem Typ; erlaubt sind auch Ausdrücke,
/// damit sich Defaults wie `dec(0)` schreiben lassen.
fn parse_scalar(node: &Yaml, ty: Ty, name: &str) -> Result<Value, Error> {
    let text = scalar_text(node).ok_or_else(|| {
        Error::load(
            format!("`{name}`: Zahl oder Text erwartet"),
            name.to_string(),
        )
    })?;
    let direct = match ty {
        Ty::Int => text.trim().parse::<i64>().ok().map(Value::Int),
        Ty::Dec => text.trim().parse::<BigDecimal>().ok().map(Value::Dec),
        Ty::Dbl => text.trim().parse::<f64>().ok().map(Value::Dbl),
        Ty::DecArr => None,
    };
    if let Some(value) = direct {
        return Ok(value);
    }
    // Kein einfacher Zahlwert — als Ausdruck auswerten.
    let expr = parse_expr(&text, &NoScope)?;
    crate::eval::coerce(const_eval(&expr)?, ty, name).map_err(|e| Error::load(e.to_string(), text))
}

fn zero_of(ty: Ty) -> Value {
    match ty {
        Ty::Int => Value::Int(0),
        Ty::Dec => Value::Dec(BigDecimal::from(0)),
        Ty::Dbl => Value::Dbl(0.0),
        Ty::DecArr => Value::Arr(Arc::new(Vec::new())),
    }
}

/// Liest eine Anweisungsliste. `elif` und `else` beziehen sich jeweils auf das
/// vorangehende `if` und werden zu verschachtelten Verzweigungen zusammengefügt.
fn read_block(
    node: &Yaml,
    scope: &HashMap<String, VarId>,
    method_ids: &HashMap<String, u32>,
    context: &str,
) -> Result<Vec<Stmt>, Error> {
    if node.is_badvalue() || node.is_null() {
        return Ok(Vec::new());
    }
    let items = node.as_vec().ok_or_else(|| {
        Error::load(
            format!("`{context}` muss eine Liste von Anweisungen sein"),
            context.to_string(),
        )
    })?;

    let mut out = Vec::new();
    let mut index = 0;
    while index < items.len() {
        let item = &items[index];
        if !item["if"].is_badvalue() {
            // Die Kette aus if + folgenden elif einsammeln.
            let mut chain = vec![item];
            let mut next = index + 1;
            while next < items.len() && !items[next]["elif"].is_badvalue() {
                chain.push(&items[next]);
                next += 1;
            }
            out.push(build_chain(&chain, scope, method_ids, context)?);
            index = next;
            continue;
        }
        if !item["elif"].is_badvalue() {
            return Err(Error::load(
                "`elif` ohne vorangehendes `if`",
                context.to_string(),
            ));
        }
        out.push(read_statement(item, scope, method_ids, context)?);
        index += 1;
    }
    Ok(out)
}

/// Baut aus `if` + `elif`… + `else` die verschachtelte Verzweigung.
fn build_chain(
    chain: &[&Yaml],
    scope: &HashMap<String, VarId>,
    method_ids: &HashMap<String, u32>,
    context: &str,
) -> Result<Stmt, Error> {
    let (item, rest) = chain.split_first().expect("Kette ist nie leer");
    let key = if item["if"].is_badvalue() {
        "elif"
    } else {
        "if"
    };
    let condition = scalar_text(&item[key]).ok_or_else(|| {
        Error::load(
            format!("`{key}` braucht eine Bedingung"),
            context.to_string(),
        )
    })?;
    let cond = parse_expr(&condition, scope)?;
    let then_ = read_block(&item["then"], scope, method_ids, context)?;

    let else_ = if rest.is_empty() {
        read_block(&item["else"], scope, method_ids, context)?
    } else {
        // Der `else`-Zweig ist das nächste Glied der Kette.
        vec![build_chain(rest, scope, method_ids, context)?]
    };

    Ok(Stmt::If { cond, then_, else_ })
}

fn read_statement(
    item: &Yaml,
    scope: &HashMap<String, VarId>,
    method_ids: &HashMap<String, u32>,
    context: &str,
) -> Result<Stmt, Error> {
    if let Some(assignment) = scalar_text(&item["eval"]) {
        let (target, expr) = parse_assign(&assignment, scope)?;
        return Ok(Stmt::Eval { target, expr });
    }
    if let Some(name) = scalar_text(&item["call"]) {
        let id = method_ids.get(&name).copied().ok_or_else(|| {
            Error::load(format!("unbekannte Methode `{name}`"), context.to_string())
        })?;
        return Ok(Stmt::Execute(id));
    }
    Err(Error::load(
        "Anweisung braucht eines der Felder `eval`, `call` oder `if`",
        context.to_string(),
    ))
}
