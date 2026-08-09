//! Laden eines PAP aus dem XML-Pseudocode des BMF.

use crate::ast::{MethodDef, Pap, Stmt, Ty, VarDecl, VarId, VarKind};
use crate::error::Error;
use crate::eval::const_eval;
use crate::parse::{NoScope, parse_array, parse_assign, parse_expr};
use crate::value::Value;
use bigdecimal::BigDecimal;
use roxmltree::{Document, Node};
use std::collections::HashMap;
use std::sync::Arc;

/// Lädt einen PAP aus einem XML-Dokument.
pub fn load(xml: &str) -> Result<Pap, Error> {
    // Die BMF-Dateien beginnen mit einem UTF-8-BOM.
    let xml = xml.trim_start_matches('\u{feff}');
    let doc = Document::parse(xml).map_err(|e| Error::load(format!("ungültiges XML: {e}"), ""))?;
    let root = doc.root_element();
    if root.tag_name().name() != "PAP" {
        return Err(Error::load(
            format!(
                "Wurzelelement `PAP` erwartet, gefunden `{}`",
                root.tag_name().name()
            ),
            "",
        ));
    }

    let mut vars: Vec<VarDecl> = Vec::new();
    let mut by_name: HashMap<String, VarId> = HashMap::new();

    // 1. Deklarationen einsammeln (Reihenfolge bleibt erhalten).
    for (tag, kind) in [
        ("INPUT", VarKind::Input),
        ("OUTPUT", VarKind::Output),
        ("INTERNAL", VarKind::Internal),
        ("CONSTANT", VarKind::Constant),
    ] {
        for node in root.descendants().filter(|n| n.has_tag_name(tag)) {
            let decl = read_decl(node, kind)?;
            if by_name
                .insert(decl.name.clone(), vars.len() as VarId)
                .is_some()
            {
                return Err(Error::load(
                    format!("Variable `{}` ist mehrfach deklariert", decl.name),
                    "",
                ));
            }
            vars.push(decl);
        }
    }

    // 2. Methoden registrieren, damit EXECUTE vorwärts auflösen kann.
    let methods_node = root
        .descendants()
        .find(|n| n.has_tag_name("METHODS"))
        .ok_or_else(|| Error::load("Abschnitt `METHODS` fehlt", ""))?;
    let method_nodes: Vec<Node> = methods_node
        .children()
        .filter(|n| n.has_tag_name("METHOD"))
        .collect();
    let method_ids: HashMap<String, u32> = method_nodes
        .iter()
        .enumerate()
        .filter_map(|(i, n)| n.attribute("name").map(|name| (name.to_string(), i as u32)))
        .collect();

    // 3. Rümpfe übersetzen.
    let mut methods = Vec::with_capacity(method_nodes.len());
    for node in &method_nodes {
        let name = node
            .attribute("name")
            .ok_or_else(|| Error::load("METHOD ohne `name`", ""))?;
        methods.push(MethodDef {
            name: name.to_string(),
            body: read_block(*node, &by_name, &method_ids)?,
        });
    }

    let main_node = methods_node
        .children()
        .find(|n| n.has_tag_name("MAIN"))
        .ok_or_else(|| Error::load("Abschnitt `MAIN` fehlt", ""))?;
    let main = read_block(main_node, &by_name, &method_ids)?;

    Ok(Pap {
        name: root.attribute("name").unwrap_or_default().to_string(),
        version: root
            .attribute("version")
            .or_else(|| root.attribute("versionNummer"))
            .unwrap_or_default()
            .to_string(),
        vars,
        by_name,
        methods,
        main,
    })
}

fn read_decl(node: Node, kind: VarKind) -> Result<VarDecl, Error> {
    let name = node
        .attribute("name")
        .ok_or_else(|| Error::load(format!("{kind:?}-Deklaration ohne `name`"), ""))?;
    let ty_text = node
        .attribute("type")
        .ok_or_else(|| Error::load(format!("Deklaration `{name}` ohne `type`"), ""))?;
    let ty = Ty::from_xml(ty_text)
        .ok_or_else(|| Error::load(format!("unbekannter Typ `{ty_text}` für `{name}`"), ""))?;

    // Konstanten stehen in `value`, alles andere in `default`; fehlt der
    // Default (kommt in den BMF-Dateien vor), gilt der Nullwert des Typs.
    let source = if kind == VarKind::Constant {
        node.attribute("value")
            .ok_or_else(|| Error::load(format!("CONSTANT `{name}` ohne `value`"), ""))?
    } else {
        match node.attribute("default") {
            Some(text) => text,
            None => {
                return Ok(VarDecl {
                    name: name.to_string(),
                    kind,
                    ty,
                    default: zero_of(ty),
                    group: output_group(node),
                });
            }
        }
    };

    let default = if ty == Ty::DecArr {
        let items = parse_array(source, &NoScope)?;
        let values: Result<Vec<Value>, Error> = items.iter().map(const_eval).collect();
        Value::Arr(Arc::new(values?))
    } else {
        let expr = parse_expr(source, &NoScope)?;
        crate::eval::coerce(const_eval(&expr)?, ty, name)
            .map_err(|e| Error::load(e.to_string(), source))?
    };

    Ok(VarDecl {
        name: name.to_string(),
        kind,
        ty,
        default,
        group: output_group(node),
    })
}

/// `<OUTPUTS type="STANDARD">` bzw. `"DBA"` als Gruppenkennzeichen.
fn output_group(node: Node) -> String {
    node.parent()
        .filter(|p| p.has_tag_name("OUTPUTS"))
        .and_then(|p| p.attribute("type"))
        .unwrap_or_default()
        .to_string()
}

fn zero_of(ty: Ty) -> Value {
    match ty {
        Ty::Int => Value::Int(0),
        Ty::Dec => Value::Dec(BigDecimal::from(0)),
        Ty::Dbl => Value::Dbl(0.0),
        Ty::DecArr => Value::Arr(Arc::new(Vec::new())),
    }
}

fn read_block(
    parent: Node,
    scope: &HashMap<String, VarId>,
    method_ids: &HashMap<String, u32>,
) -> Result<Vec<Stmt>, Error> {
    let mut out = Vec::new();
    for node in parent.children().filter(|n| n.is_element()) {
        match node.tag_name().name() {
            "EVAL" => {
                let exec = node
                    .attribute("exec")
                    .ok_or_else(|| Error::load("EVAL ohne `exec`", ""))?;
                let (target, expr) = parse_assign(exec, scope)?;
                out.push(Stmt::Eval { target, expr });
            }
            "EXECUTE" => {
                let name = node
                    .attribute("method")
                    .ok_or_else(|| Error::load("EXECUTE ohne `method`", ""))?;
                let id = method_ids
                    .get(name)
                    .copied()
                    .ok_or_else(|| Error::load(format!("unbekannte Methode `{name}`"), ""))?;
                out.push(Stmt::Execute(id));
            }
            "IF" => {
                let expr_src = node
                    .attribute("expr")
                    .ok_or_else(|| Error::load("IF ohne `expr`", ""))?;
                let cond = parse_expr(expr_src, scope)?;
                let branch = |tag: &str| -> Result<Vec<Stmt>, Error> {
                    match node.children().find(|n| n.has_tag_name(tag)) {
                        Some(n) => read_block(n, scope, method_ids),
                        None => Ok(Vec::new()),
                    }
                };
                out.push(Stmt::If {
                    cond,
                    then_: branch("THEN")?,
                    else_: branch("ELSE")?,
                });
            }
            other => {
                return Err(Error::load(
                    format!("unerwartetes Element `{other}` in einem Anweisungsblock"),
                    "",
                ));
            }
        }
    }
    Ok(out)
}
