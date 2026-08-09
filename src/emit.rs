//! Ausgabe eines geladenen PAP im YAML-Format.
//!
//! Der Ausdrucksteil wird in die Kompaktschreibweise übersetzt, die
//! [`crate::parse`] ebenfalls versteht: Arithmetik als Operatoren, alles
//! Übrige als Funktion mit dem Empfänger als erstem Argument
//! (`div(a, b, 2, down)`, `scale(a, 0, down)`, `cmp(a, b)`).

use crate::ast::{BdMethod, BinOp, Expr, Pap, Stmt, Ty, VarKind};
use crate::value::{RoundMode, to_plain_string};
use crate::{Value, format_value};

/// Bindungsstärken; höher bindet stärker. Wird nur zur Klammersetzung genutzt.
const P_OR: u8 = 1;
const P_AND: u8 = 2;
const P_EQ: u8 = 3;
const P_CMP: u8 = 4;
const P_ADD: u8 = 5;
const P_MUL: u8 = 6;
const P_UNARY: u8 = 7;
const P_ATOM: u8 = 8;

fn precedence(op: BinOp) -> u8 {
    match op {
        BinOp::Or => P_OR,
        BinOp::And => P_AND,
        BinOp::Eq | BinOp::Ne => P_EQ,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => P_CMP,
        BinOp::Add | BinOp::Sub => P_ADD,
        BinOp::Mul | BinOp::Div | BinOp::Rem => P_MUL,
    }
}

fn symbol(op: BinOp) -> &'static str {
    match op {
        BinOp::Or => "||",
        BinOp::And => "&&",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
    }
}

fn round_name(mode: RoundMode) -> &'static str {
    match mode {
        RoundMode::Up => "up",
        RoundMode::Down => "down",
        RoundMode::Ceiling => "ceiling",
        RoundMode::Floor => "floor",
        RoundMode::HalfUp => "half_up",
        RoundMode::HalfDown => "half_down",
        RoundMode::HalfEven => "half_even",
    }
}

/// Schreibt einen Ausdruck in der Kompaktschreibweise.
pub fn expr_to_string(pap: &Pap, expr: &Expr) -> String {
    let mut out = String::new();
    write_expr(&mut out, pap, expr, 0);
    out
}

/// `min_precedence` ist die Stärke, die der Kontext verlangt; bindet der
/// Ausdruck schwächer, wird geklammert.
fn write_expr(out: &mut String, pap: &Pap, expr: &Expr, min_precedence: u8) {
    match expr {
        Expr::IntLit(i) => out.push_str(&i.to_string()),
        // `dec(...)` hält den Typ fest: Ein nacktes `0.006` läse sich sonst
        // als `double` zurück statt als BigDecimal.
        Expr::DecLit(d) => {
            out.push_str("dec(");
            out.push_str(&to_plain_string(d));
            out.push(')');
        }
        Expr::DblLit(f) => {
            let text = f.to_string();
            // Als double erkennbar halten, damit der Round-Trip den Typ behält.
            out.push_str(&text);
            if !text.contains('.') {
                out.push_str(".0");
            }
        }
        Expr::RoundLit(mode) => out.push_str(round_name(*mode)),
        Expr::Var(id) => out.push_str(&pap.var(*id).name),
        Expr::Index { arr, idx } => {
            write_expr(out, pap, arr, P_ATOM);
            out.push('[');
            write_expr(out, pap, idx, 0);
            out.push(']');
        }
        Expr::ValueOf(inner) => write_function(out, pap, "dec", std::slice::from_ref(&**inner)),
        Expr::NewBigDecimal(inner) => {
            write_function(out, pap, "bigdec", std::slice::from_ref(&**inner))
        }
        Expr::Neg(inner) => {
            let needs = P_UNARY < min_precedence;
            if needs {
                out.push('(');
            }
            out.push('-');
            write_expr(out, pap, inner, P_UNARY);
            if needs {
                out.push(')');
            }
        }
        Expr::Not(inner) => {
            let needs = P_UNARY < min_precedence;
            if needs {
                out.push('(');
            }
            out.push('!');
            write_expr(out, pap, inner, P_UNARY);
            if needs {
                out.push(')');
            }
        }
        Expr::Bin { op, lhs, rhs } => {
            // `cmp(a, b) == -1` ist der häufigste Ausdruck des PAP und meint
            // schlicht `a < b`. BigDecimal vergleicht numerisch, exakt wie
            // Javas compareTo — die Umschreibung ist also bedeutungsgleich.
            if let Some(direct) = comparison_operator(*op, lhs, rhs) {
                let p = precedence(direct);
                let needs = p < min_precedence;
                if needs {
                    out.push('(');
                }
                let Expr::Call { recv, args, .. } = &**lhs else {
                    unreachable!("von comparison_operator geprüft")
                };
                write_expr(out, pap, recv, p);
                out.push(' ');
                out.push_str(symbol(direct));
                out.push(' ');
                write_expr(out, pap, &args[0], p + 1);
                if needs {
                    out.push(')');
                }
                return;
            }
            let p = precedence(*op);
            let needs = p < min_precedence;
            if needs {
                out.push('(');
            }
            write_expr(out, pap, lhs, p);
            out.push(' ');
            out.push_str(symbol(*op));
            out.push(' ');
            // Rechts eine Stufe strenger: erhält die Linksassoziativität.
            write_expr(out, pap, rhs, p + 1);
            if needs {
                out.push(')');
            }
        }
        Expr::Call { recv, method, args } => {
            write_call(out, pap, recv, *method, args, min_precedence)
        }
    }
}

/// Erkennt `cmp(a, b) == k` mit k ∈ {-1, 0, 1} und liefert den gleichwertigen
/// Vergleichsoperator.
fn comparison_operator(op: BinOp, lhs: &Expr, rhs: &Expr) -> Option<BinOp> {
    if !matches!(op, BinOp::Eq | BinOp::Ne) {
        return None;
    }
    let Expr::Call {
        method: BdMethod::CompareTo,
        args,
        ..
    } = lhs
    else {
        return None;
    };
    if args.len() != 1 {
        return None;
    }
    // Je nachdem, ob die Konstantenfaltung schon gelaufen ist, steht hier
    // `IntLit(-1)` oder `Neg(IntLit(1))`.
    let k = match rhs {
        Expr::IntLit(k) => *k,
        Expr::Neg(inner) => match &**inner {
            Expr::IntLit(k) => -k,
            _ => return None,
        },
        _ => return None,
    };
    Some(match (op, &k) {
        (BinOp::Eq, -1) => BinOp::Lt,
        (BinOp::Eq, 0) => BinOp::Eq,
        (BinOp::Eq, 1) => BinOp::Gt,
        (BinOp::Ne, -1) => BinOp::Ge,
        (BinOp::Ne, 0) => BinOp::Ne,
        (BinOp::Ne, 1) => BinOp::Le,
        _ => return None,
    })
}

/// Grundrechenarten werden zu Operatoren, alles Übrige zu Funktionen.
fn write_call(
    out: &mut String,
    pap: &Pap,
    recv: &Expr,
    method: BdMethod,
    args: &[Expr],
    min_precedence: u8,
) {
    let binary = match (method, args.len()) {
        (BdMethod::Add, 1) => Some(BinOp::Add),
        (BdMethod::Subtract, 1) => Some(BinOp::Sub),
        (BdMethod::Multiply, 1) => Some(BinOp::Mul),
        // Nur die einargumentige Division entspricht dem Operator; mit
        // Rundungsangabe bleibt es `div(...)`.
        (BdMethod::Divide, 1) => Some(BinOp::Div),
        _ => None,
    };

    if let Some(op) = binary {
        let p = precedence(op);
        let needs = p < min_precedence;
        if needs {
            out.push('(');
        }
        write_expr(out, pap, recv, p);
        out.push(' ');
        out.push_str(symbol(op));
        out.push(' ');
        write_expr(out, pap, &args[0], p + 1);
        if needs {
            out.push(')');
        }
        return;
    }

    let name = match method {
        BdMethod::Add => "add",
        BdMethod::Subtract => "subtract",
        BdMethod::Multiply => "multiply",
        BdMethod::Divide => "div",
        BdMethod::SetScale => "scale",
        BdMethod::CompareTo => "cmp",
        BdMethod::LongValue => "long",
        BdMethod::IntValue => "int",
        BdMethod::Negate => "neg",
        BdMethod::Abs => "abs",
    };
    let mut all = Vec::with_capacity(args.len() + 1);
    all.push(recv.clone());
    all.extend(args.iter().cloned());
    write_function(out, pap, name, &all);
}

fn write_function(out: &mut String, pap: &Pap, name: &str, args: &[Expr]) {
    out.push_str(name);
    out.push('(');
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        write_expr(out, pap, arg, 0);
    }
    out.push(')');
}

// ---------------------------------------------------------------------------
// YAML
// ---------------------------------------------------------------------------

/// Gibt den vollständigen PAP als YAML aus.
pub fn to_yaml(pap: &Pap) -> String {
    let mut out = String::new();
    out.push_str(&format!("name: {}\n", quote(&pap.name)));
    out.push_str(&format!("version: {}\n", quote(&pap.version)));

    for (section, kind) in [
        ("inputs", VarKind::Input),
        ("outputs", VarKind::Output),
        ("internals", VarKind::Internal),
    ] {
        let vars: Vec<_> = pap.vars_of_kind(kind).collect();
        if vars.is_empty() {
            continue;
        }
        out.push_str(&format!("\n{section}:\n"));
        for (_, decl) in vars {
            let mut fields = format!("type: {}", decl.ty.name());
            if !is_zero(&decl.default) {
                fields.push_str(&format!(
                    ", default: {}",
                    quote(&format_value(&decl.default))
                ));
            }
            if !decl.group.is_empty() {
                fields.push_str(&format!(", group: {}", quote(&decl.group)));
            }
            out.push_str(&format!("  {}: {{ {fields} }}\n", quote_key(&decl.name)));
        }
    }

    let constants: Vec<_> = pap.vars_of_kind(VarKind::Constant).collect();
    if !constants.is_empty() {
        out.push_str("\nconstants:\n");
        for (_, decl) in constants {
            match &decl.default {
                Value::Arr(items) => {
                    let inner: Vec<String> = items.iter().map(format_value).collect();
                    out.push_str(&format!(
                        "  {}: [{}]\n",
                        quote_key(&decl.name),
                        inner.join(", ")
                    ));
                }
                value => out.push_str(&format!(
                    "  {}: {}\n",
                    quote_key(&decl.name),
                    quote(&format_value(value))
                )),
            }
        }
    }

    out.push_str("\nmain:\n");
    write_block(&mut out, pap, &pap.main, 1);

    if !pap.methods.is_empty() {
        out.push_str("\nmethods:\n");
        for method in &pap.methods {
            out.push_str(&format!("  {}:\n", quote_key(&method.name)));
            write_block(&mut out, pap, &method.body, 2);
        }
    }
    out
}

/// Schreibt einen Anweisungsblock als YAML-Liste. `indent` zählt in Schritten
/// von zwei Leerzeichen.
fn write_block(out: &mut String, pap: &Pap, block: &[Stmt], indent: usize) {
    let pad = "  ".repeat(indent);
    if block.is_empty() {
        out.push_str(&format!("{pad}[]\n"));
        return;
    }
    for stmt in block {
        match stmt {
            Stmt::Eval { target, expr } => {
                let line = format!("{} = {}", pap.var(*target).name, expr_to_string(pap, expr));
                out.push_str(&format!("{pad}- eval: {}\n", quote(&line)));
            }
            Stmt::Execute(id) => {
                out.push_str(&format!(
                    "{pad}- call: {}\n",
                    quote(&pap.methods[*id as usize].name)
                ));
            }
            Stmt::If { cond, then_, else_ } => {
                out.push_str(&format!(
                    "{pad}- if: {}\n",
                    quote(&expr_to_string(pap, cond))
                ));
                out.push_str(&format!("{pad}  then:\n"));
                write_block(out, pap, then_, indent + 2);
                write_else(out, pap, else_, indent, &pad);
            }
        }
    }
}

/// Ein `else`-Zweig, der nur aus einem weiteren `if` besteht, wird zu `elif`.
/// Ohne diese Faltung würde der Tarifteil zwanzig Ebenen tief einrücken.
fn write_else(out: &mut String, pap: &Pap, else_: &[Stmt], indent: usize, pad: &str) {
    if else_.is_empty() {
        return;
    }
    if let [
        Stmt::If {
            cond,
            then_,
            else_: inner,
        },
    ] = else_
    {
        out.push_str(&format!(
            "{pad}- elif: {}\n",
            quote(&expr_to_string(pap, cond))
        ));
        out.push_str(&format!("{pad}  then:\n"));
        write_block(out, pap, then_, indent + 2);
        write_else(out, pap, inner, indent, pad);
        return;
    }
    out.push_str(&format!("{pad}  else:\n"));
    write_block(out, pap, else_, indent + 2);
}

fn is_zero(value: &Value) -> bool {
    match value {
        Value::Int(i) => *i == 0,
        Value::Dbl(f) => *f == 0.0,
        Value::Dec(d) => num_traits::Zero::is_zero(d),
        Value::Arr(items) => items.is_empty(),
        Value::Bool(b) => !*b,
    }
}

/// Schlüssel bleiben unzitiert, solange sie eindeutig sind. Zitiert wird nur,
/// was YAML sonst umdeuten würde — etwa `NO` zu `false` (YAML 1.1) oder einen
/// Namen, der wie eine Zahl aussieht.
fn quote_key(name: &str) -> String {
    const RESERVED: &[&str] = &[
        "y", "n", "yes", "no", "true", "false", "on", "off", "null", "~",
    ];
    let plain = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && !RESERVED.contains(&name.to_ascii_lowercase().as_str());
    if plain { name.to_string() } else { quote(name) }
}

/// YAML-Skalar in doppelten Anführungszeichen. Alle Werte werden zitiert:
/// Das hält Zahlen vom Fließkomma-Parser fern und entschärft YAML-Eigenheiten
/// wie `NO` → `false`.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Typname für die Ausgabe.
impl Ty {
    pub fn yaml_name(self) -> &'static str {
        self.name()
    }
}
