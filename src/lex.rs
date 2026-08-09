//! Tokenizer für das Java-Subset der PAP-Ausdrücke.

use crate::error::Error;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    /// Zahlliteral im Originaltext (z. B. `2004`, `0.07`).
    Num(String),
    New,
    Punct(&'static str),
}

const OPERATORS: &[&str] = &[
    "&&", "||", "==", "!=", "<=", ">=", "<", ">", "+", "-", "*", "/", "%", "!", "(", ")", "[", "]",
    "{", "}", ",", ".", "=",
];

pub fn tokenize(src: &str) -> Result<Vec<Tok>, Error> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i] as char;

        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        if c.is_ascii_digit() {
            let start = i;
            while i < bytes.len() && ((bytes[i] as char).is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            let text = &src[start..i];
            if text.matches('.').count() > 1 {
                return Err(Error::load(format!("ungültiges Zahlliteral `{text}`"), src));
            }
            out.push(Tok::Num(text.to_string()));
            continue;
        }

        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < bytes.len()
                && ((bytes[i] as char).is_ascii_alphanumeric() || bytes[i] == b'_')
            {
                i += 1;
            }
            let word = &src[start..i];
            out.push(if word == "new" {
                Tok::New
            } else {
                Tok::Ident(word.to_string())
            });
            continue;
        }

        // "==" vor "=" prüfen: OPERATORS ist absteigend nach Länge sortiert.
        match OPERATORS.iter().find(|op| src[i..].starts_with(**op)) {
            Some(op) => {
                out.push(Tok::Punct(op));
                i += op.len();
            }
            None => {
                return Err(Error::load(format!("unerwartetes Zeichen `{c}`"), src));
            }
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_method_call() {
        let toks = tokenize("RE4.divide (ZAHL100, 2, BigDecimal.ROUND_DOWN)").unwrap();
        assert_eq!(toks[0], Tok::Ident("RE4".into()));
        assert_eq!(toks[1], Tok::Punct("."));
        assert_eq!(toks[2], Tok::Ident("divide".into()));
        assert_eq!(toks[3], Tok::Punct("("));
        assert_eq!(toks[5], Tok::Punct(","));
        assert_eq!(toks[6], Tok::Num("2".into()));
    }

    #[test]
    fn distinguishes_assign_and_equals() {
        assert_eq!(tokenize("A= 1").unwrap()[1], Tok::Punct("="));
        assert_eq!(tokenize("A == 1").unwrap()[1], Tok::Punct("=="));
        assert_eq!(tokenize("A != 1").unwrap()[1], Tok::Punct("!="));
        assert_eq!(tokenize("A <= 1").unwrap()[1], Tok::Punct("<="));
    }

    #[test]
    fn decimal_and_new() {
        let toks = tokenize("new BigDecimal(0.07)").unwrap();
        assert_eq!(toks[0], Tok::New);
        assert_eq!(toks[3], Tok::Num("0.07".into()));
    }

    #[test]
    fn rejects_garbage() {
        assert!(tokenize("A @ B").is_err());
        assert!(tokenize("1.2.3").is_err());
    }
}
