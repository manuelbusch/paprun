//! Parser für das Java-Subset der PAP-Ausdrücke (rekursiver Abstieg).
//!
//! Grammatik (absteigende Bindungsstärke):
//! ```text
//! expr       -> or
//! or         -> and ( "||" and )*
//! and        -> equality ( "&&" equality )*
//! equality   -> relational ( ("=="|"!=") relational )*
//! relational -> additive ( ("<"|"<="|">"|">=") additive )*
//! additive   -> multiplicative ( ("+"|"-") multiplicative )*
//! multiplicative -> unary ( ("*"|"/"|"%") unary )*
//! unary      -> ("-"|"!") unary | postfix
//! postfix    -> primary ( "." IDENT "(" args? ")" | "[" expr "]" )*
//! primary    -> NUMBER | IDENT | "new" "BigDecimal" "(" args ")" | "(" expr ")"
//!             | "BigDecimal" "." ( "ZERO"|"ONE"|"TEN"|"ROUND_*"|"valueOf" "(" expr ")" )
//! ```

use crate::ast::{BdMethod, BinOp, Expr, VarId};
use crate::error::Error;
use crate::lex::{Tok, tokenize};
use crate::value::RoundMode;
use bigdecimal::BigDecimal;
use std::collections::HashMap;

/// Auflösung von Variablennamen zu Slot-Indizes.
pub trait Scope {
    fn lookup(&self, name: &str) -> Option<VarId>;
}

impl Scope for HashMap<String, VarId> {
    fn lookup(&self, name: &str) -> Option<VarId> {
        self.get(name).copied()
    }
}

/// Leerer Scope für Ausdrücke ohne Variablenbezug (Defaults, Konstanten).
pub struct NoScope;

impl Scope for NoScope {
    fn lookup(&self, _name: &str) -> Option<VarId> {
        None
    }
}

/// Parst einen Ausdruck (`expr`-Attribut, Default- oder Konstantenwert).
pub fn parse_expr(src: &str, scope: &dyn Scope) -> Result<Expr, Error> {
    let mut p = Parser::new(src, scope)?;
    let e = p.expression()?;
    p.expect_end()?;
    Ok(e)
}

/// Parst eine Zuweisung (`exec`-Attribut) zu Zielvariable und Ausdruck.
pub fn parse_assign(src: &str, scope: &dyn Scope) -> Result<(VarId, Expr), Error> {
    let mut p = Parser::new(src, scope)?;
    let name = match p.next().cloned() {
        Some(Tok::Ident(n)) => n,
        other => {
            return Err(Error::load(
                format!("Zuweisung erwartet einen Variablennamen, gefunden {other:?}"),
                src,
            ));
        }
    };
    p.expect_punct("=")?;
    let target = p.resolve(&name)?;
    let expr = p.expression()?;
    p.expect_end()?;
    Ok((target, expr))
}

/// Parst ein Array-Literal `{a, b, c}` (Konstantentabellen).
pub fn parse_array(src: &str, scope: &dyn Scope) -> Result<Vec<Expr>, Error> {
    let mut p = Parser::new(src, scope)?;
    p.expect_punct("{")?;
    let mut items = Vec::new();
    if !p.peek_punct("}") {
        loop {
            items.push(p.expression()?);
            if p.eat_punct(",") {
                continue;
            }
            break;
        }
    }
    p.expect_punct("}")?;
    p.expect_end()?;
    Ok(items)
}

struct Parser<'a> {
    src: &'a str,
    toks: Vec<Tok>,
    pos: usize,
    scope: &'a dyn Scope,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str, scope: &'a dyn Scope) -> Result<Self, Error> {
        Ok(Parser {
            src,
            toks: tokenize(src)?,
            pos: 0,
            scope,
        })
    }

    fn err(&self, msg: impl Into<String>) -> Error {
        Error::load(msg, self.src)
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<&Tok> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn peek_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok::Punct(x)) if *x == p)
    }

    fn eat_punct(&mut self, p: &str) -> bool {
        if self.peek_punct(p) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_punct(&mut self, p: &str) -> Result<(), Error> {
        if self.eat_punct(p) {
            Ok(())
        } else {
            Err(self.err(format!("`{p}` erwartet, gefunden {:?}", self.peek())))
        }
    }

    fn expect_end(&self) -> Result<(), Error> {
        match self.peek() {
            None => Ok(()),
            Some(t) => Err(self.err(format!("überzähliges Token {t:?}"))),
        }
    }

    fn resolve(&self, name: &str) -> Result<VarId, Error> {
        self.scope
            .lookup(name)
            .ok_or_else(|| self.err(format!("unbekannte Variable `{name}`")))
    }

    fn expression(&mut self) -> Result<Expr, Error> {
        self.binary(0)
    }

    /// Binäre Operatoren über Präzedenzebenen (0 = schwächste Bindung).
    fn binary(&mut self, level: usize) -> Result<Expr, Error> {
        const LEVELS: &[&[(&str, BinOp)]] = &[
            &[("||", BinOp::Or)],
            &[("&&", BinOp::And)],
            &[("==", BinOp::Eq), ("!=", BinOp::Ne)],
            &[
                ("<=", BinOp::Le),
                (">=", BinOp::Ge),
                ("<", BinOp::Lt),
                (">", BinOp::Gt),
            ],
            &[("+", BinOp::Add), ("-", BinOp::Sub)],
            &[("*", BinOp::Mul), ("/", BinOp::Div), ("%", BinOp::Rem)],
        ];

        if level == LEVELS.len() {
            return self.unary();
        }

        let mut lhs = self.binary(level + 1)?;
        loop {
            let Some(&(_, op)) = LEVELS[level].iter().find(|(sym, _)| self.peek_punct(sym)) else {
                return Ok(lhs);
            };
            self.pos += 1;
            let rhs = self.binary(level + 1)?;
            lhs = Expr::Bin {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
    }

    fn unary(&mut self) -> Result<Expr, Error> {
        if self.eat_punct("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.eat_punct("!") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr, Error> {
        let mut e = self.primary()?;
        loop {
            if self.eat_punct(".") {
                let name = match self.next().cloned() {
                    Some(Tok::Ident(n)) => n,
                    other => {
                        return Err(self.err(format!("Methodenname erwartet, gefunden {other:?}")));
                    }
                };
                let method = BdMethod::from_name(&name)
                    .ok_or_else(|| self.err(format!("nicht unterstützte Methode `{name}`")))?;
                let args = self.call_args()?;
                e = Expr::Call {
                    recv: Box::new(e),
                    method,
                    args,
                };
            } else if self.eat_punct("[") {
                let idx = self.expression()?;
                self.expect_punct("]")?;
                e = Expr::Index {
                    arr: Box::new(e),
                    idx: Box::new(idx),
                };
            } else {
                return Ok(e);
            }
        }
    }

    fn call_args(&mut self) -> Result<Vec<Expr>, Error> {
        self.expect_punct("(")?;
        let mut args = Vec::new();
        if !self.peek_punct(")") {
            loop {
                args.push(self.expression()?);
                if self.eat_punct(",") {
                    continue;
                }
                break;
            }
        }
        self.expect_punct(")")?;
        Ok(args)
    }

    fn primary(&mut self) -> Result<Expr, Error> {
        match self.next().cloned() {
            Some(Tok::Num(text)) => Ok(number_literal(&text, self.src)?),
            Some(Tok::New) => {
                match self.next().cloned() {
                    Some(Tok::Ident(n)) if n == "BigDecimal" => {}
                    other => {
                        return Err(
                            self.err(format!("`new BigDecimal` erwartet, gefunden {other:?}"))
                        );
                    }
                }
                let args = self.call_args()?;
                match <[Expr; 1]>::try_from(args) {
                    Ok([arg]) => Ok(Expr::NewBigDecimal(Box::new(arg))),
                    Err(args) => Err(self.err(format!(
                        "new BigDecimal erwartet genau ein Argument, erhielt {}",
                        args.len()
                    ))),
                }
            }
            Some(Tok::Punct("(")) => {
                let e = self.expression()?;
                self.expect_punct(")")?;
                Ok(e)
            }
            Some(Tok::Ident(name)) => {
                if (name == "BigDecimal" || name == "RoundingMode") && self.peek_punct(".") {
                    return self.static_member(&name);
                }
                // Kompaktschreibweise des YAML-Formats: `div(a, b, 2, down)`
                // und `scale(a, 0, down)`.
                if self.peek_punct("(") {
                    if let Some(method) = compact_function(&name) {
                        return self.compact_call(&name, method);
                    }
                    // `dec(x)` = BigDecimal.valueOf(x) (Dezimaldarstellung),
                    // `bigdec(x)` = new BigDecimal(x) (bei double exakt binär).
                    if name == "dec" || name == "bigdec" {
                        // `dec(1015.13)` mit reinem Zahlliteral wird direkt zum
                        // Dezimalwert — ohne Umweg über `double`, der
                        // Nachkommastellen verfälschen könnte.
                        if name == "dec"
                            && let Some(literal) = self.decimal_literal_argument()?
                        {
                            return Ok(Expr::DecLit(literal));
                        }
                        let args = self.call_args()?;
                        return match <[Expr; 1]>::try_from(args) {
                            Ok([arg]) if name == "dec" => Ok(Expr::ValueOf(Box::new(arg))),
                            Ok([arg]) => Ok(Expr::NewBigDecimal(Box::new(arg))),
                            Err(args) => Err(self.err(format!(
                                "`{name}` erwartet genau ein Argument, erhielt {}",
                                args.len()
                            ))),
                        };
                    }
                }
                if let Some(id) = self.scope.lookup(&name) {
                    return Ok(Expr::Var(id));
                }
                // Rundungsmodi dürfen auch ohne Klassenpräfix stehen
                // (`down` statt `BigDecimal.ROUND_DOWN`).
                RoundMode::from_java_name(&name.to_uppercase())
                    .map(Expr::RoundLit)
                    .ok_or_else(|| self.err(format!("unbekannte Variable `{name}`")))
            }
            other => Err(self.err(format!("Ausdruck erwartet, gefunden {other:?}"))),
        }
    }

    /// Erkennt `(<Zahl>)` als vollständige Argumentliste und liefert den Wert
    /// exakt aus dem Quelltext. Nur dann wird die Position vorgerückt.
    fn decimal_literal_argument(&mut self) -> Result<Option<BigDecimal>, Error> {
        let (Some(Tok::Punct("(")), Some(Tok::Num(text)), Some(Tok::Punct(")"))) = (
            self.toks.get(self.pos),
            self.toks.get(self.pos + 1),
            self.toks.get(self.pos + 2),
        ) else {
            return Ok(None);
        };
        let value = text
            .parse::<BigDecimal>()
            .map_err(|e| self.err(format!("ungültige Dezimalzahl `{text}`: {e}")))?;
        self.pos += 3;
        Ok(Some(value))
    }

    /// `div(a, b, scale, modus)` bzw. `scale(a, stellen, modus)` — der erste
    /// Parameter ist der Empfänger des entsprechenden Methodenaufrufs.
    fn compact_call(&mut self, name: &str, method: BdMethod) -> Result<Expr, Error> {
        let mut args = self.call_args()?;
        if args.is_empty() {
            return Err(self.err(format!("`{name}` erwartet mindestens ein Argument")));
        }
        let recv = args.remove(0);
        Ok(Expr::Call {
            recv: Box::new(recv),
            method,
            args,
        })
    }

    /// `BigDecimal.ZERO`, `BigDecimal.ROUND_DOWN`, `BigDecimal.valueOf(x)`,
    /// `RoundingMode.HALF_UP`.
    fn static_member(&mut self, class: &str) -> Result<Expr, Error> {
        self.expect_punct(".")?;
        let member = match self.next().cloned() {
            Some(Tok::Ident(n)) => n,
            other => return Err(self.err(format!("Membername erwartet, gefunden {other:?}"))),
        };

        if class == "BigDecimal" && member == "valueOf" {
            let args = self.call_args()?;
            return match <[Expr; 1]>::try_from(args) {
                Ok([arg]) => Ok(Expr::ValueOf(Box::new(arg))),
                Err(args) => Err(self.err(format!(
                    "BigDecimal.valueOf erwartet genau ein Argument, erhielt {}",
                    args.len()
                ))),
            };
        }

        if class == "BigDecimal" {
            match member.as_str() {
                "ZERO" => return Ok(Expr::DecLit(BigDecimal::from(0))),
                "ONE" => return Ok(Expr::DecLit(BigDecimal::from(1))),
                "TEN" => return Ok(Expr::DecLit(BigDecimal::from(10))),
                _ => {}
            }
        }

        RoundMode::from_java_name(&member)
            .map(Expr::RoundLit)
            .ok_or_else(|| self.err(format!("nicht unterstütztes Member `{class}.{member}`")))
    }
}

/// Funktionsschreibweisen des YAML-Formats auf die jeweilige Methode abbilden.
/// Der erste Parameter ist jeweils der Empfänger.
fn compact_function(name: &str) -> Option<BdMethod> {
    Some(match name {
        "div" => BdMethod::Divide,
        "scale" => BdMethod::SetScale,
        "cmp" => BdMethod::CompareTo,
        "abs" => BdMethod::Abs,
        "neg" => BdMethod::Negate,
        "long" => BdMethod::LongValue,
        "int" => BdMethod::IntValue,
        _ => return None,
    })
}

/// Java-Literalsemantik: mit Punkt ist es ein `double`-Literal, sonst `int`.
fn number_literal(text: &str, src: &str) -> Result<Expr, Error> {
    if text.contains('.') {
        text.parse::<f64>()
            .map(Expr::DblLit)
            .map_err(|e| Error::load(format!("ungültiges double-Literal `{text}`: {e}"), src))
    } else {
        text.parse::<i64>()
            .map(Expr::IntLit)
            .map_err(|e| Error::load(format!("ungültiges int-Literal `{text}`: {e}"), src))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(names: &[&str]) -> HashMap<String, VarId> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.to_string(), i as VarId))
            .collect()
    }

    #[test]
    fn parses_real_eval_snippet() {
        let s = scope(&["ZRE4J", "RE4", "ZAHL100"]);
        let (target, expr) =
            parse_assign("ZRE4J= RE4.divide (ZAHL100, 2, BigDecimal.ROUND_DOWN)", &s).unwrap();
        assert_eq!(target, 0);
        match expr {
            Expr::Call { method, args, .. } => {
                assert_eq!(method, BdMethod::Divide);
                assert_eq!(args.len(), 3);
                assert!(matches!(args[2], Expr::RoundLit(RoundMode::Down)));
            }
            other => panic!("unerwarteter AST: {other:?}"),
        }
    }

    #[test]
    fn parses_chained_and_nested_calls() {
        let s = scope(&["STS", "LSTSO", "LSTOSO", "f", "ZAHL100"]);
        parse_assign(
            "STS = LSTSO.subtract(LSTOSO).multiply(BigDecimal.valueOf(f)).divide(ZAHL100, 0, BigDecimal.ROUND_DOWN).multiply(ZAHL100)",
            &s,
        )
        .unwrap();
    }

    #[test]
    fn parses_conditions() {
        let s = scope(&["SONSTB", "MBV", "ZVBEZ", "FVBZ", "STKL"]);
        parse_expr(
            "SONSTB.compareTo (BigDecimal.ZERO) == 0 && MBV.compareTo (BigDecimal.ZERO) == 0",
            &s,
        )
        .unwrap();
        parse_expr(
            "(ZVBEZ.subtract (FVBZ)).compareTo (BigDecimal.valueOf (102)) == -1",
            &s,
        )
        .unwrap();
        parse_expr("STKL < 5", &s).unwrap();
    }

    #[test]
    fn parses_int_arithmetic_and_index() {
        let s = scope(&["J", "VJAHR", "TAB2", "FVBSO"]);
        parse_assign("J= VJAHR - 2004", &s).unwrap();
        match parse_assign("FVBSO = TAB2[J]", &s).unwrap().1 {
            Expr::Index { .. } => {}
            other => panic!("Index erwartet, erhielt {other:?}"),
        }
    }

    #[test]
    fn parses_array_literal() {
        let items = parse_array(
            "{BigDecimal.valueOf(0), BigDecimal.valueOf( 0.4),\n BigDecimal.valueOf( 0.384) }",
            &NoScope,
        )
        .unwrap();
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn parses_new_bigdecimal_and_precedence() {
        let s = scope(&["A", "B", "C"]);
        parse_expr("new BigDecimal(100)", &NoScope).unwrap();
        // A + B * C muss als A + (B * C) gruppieren
        match parse_expr("A + B * C", &s).unwrap() {
            Expr::Bin {
                op: BinOp::Add,
                rhs,
                ..
            } => {
                assert!(matches!(*rhs, Expr::Bin { op: BinOp::Mul, .. }));
            }
            other => panic!("unerwarteter AST: {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_names_and_methods() {
        let s = scope(&["A"]);
        assert!(parse_expr("UNBEKANNT + 1", &s).is_err());
        assert!(parse_expr("A.sqrt()", &s).is_err());
        assert!(parse_expr("Math.max(A, 1)", &s).is_err());
        assert!(parse_expr("A +", &s).is_err());
    }
}
