//! Erzeugen von JSON-Ausgaben.
//!
//! Werte bleiben durchgehend Zeichenketten: Ein `BigDecimal` mit
//! Nachkommastellen würde als JSON-Zahl über JavaScripts `Number` laufen und
//! dabei an Genauigkeit verlieren.

/// Flaches JSON-Objekt aus Name-Wert-Paaren, in der übergebenen Reihenfolge.
pub fn object(entries: &[(String, String)]) -> String {
    let body: Vec<String> = entries
        .iter()
        .map(|(name, value)| format!("{}:{}", string(name), string(value)))
        .collect();
    format!("{{{}}}", body.join(","))
}

/// Dasselbe, aber eingerückt und mit Zeilenumbrüchen.
pub fn object_pretty(entries: &[(String, String)]) -> String {
    let body: Vec<String> = entries
        .iter()
        .map(|(name, value)| format!("  {}: {}", string(name), string(value)))
        .collect();
    format!("{{\n{}\n}}", body.join(",\n"))
}

/// JSON-Array aus bereits fertigen JSON-Werten.
pub fn array(items: &[String]) -> String {
    format!("[{}]", items.join(","))
}

/// Zeichenkette als JSON-Literal, mit vollständiger Maskierung.
pub fn string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(name: &str, value: &str) -> (String, String) {
        (name.to_string(), value.to_string())
    }

    #[test]
    fn builds_flat_objects() {
        let entries = [pair("LSTLZZ", "692700"), pair("SOLZLZZ", "0")];
        assert_eq!(object(&entries), r#"{"LSTLZZ":"692700","SOLZLZZ":"0"}"#);
    }

    #[test]
    fn keeps_declaration_order() {
        let entries = [pair("B", "2"), pair("A", "1")];
        assert!(object(&entries).starts_with(r#"{"B""#));
    }

    #[test]
    fn escapes_special_characters() {
        assert_eq!(string(r#"a"b"#), r#""a\"b""#);
        assert_eq!(string(r"a\b"), r#""a\\b""#);
        assert_eq!(string("a\nb"), r#""a\nb""#);
        assert_eq!(string("a\tb"), r#""a\tb""#);
        // Steuerzeichen werden als \uXXXX ausgeschrieben.
        assert_eq!(string("a\u{1}b"), r#""a\u0001b""#);
        // Umlaute bleiben unverändert; JSON ist UTF-8.
        assert_eq!(string("Größe"), r#""Größe""#);
    }

    #[test]
    fn empty_object_is_valid() {
        assert_eq!(object(&[]), "{}");
        assert_eq!(array(&[]), "[]");
    }

    #[test]
    fn pretty_output_is_indented() {
        let json = object_pretty(&[pair("A", "1")]);
        assert_eq!(json, "{\n  \"A\": \"1\"\n}");
    }
}
