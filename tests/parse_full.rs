//! Lädt die amtlichen PAP-XML-Dateien vollständig: jedes `exec`, `expr`,
//! `default` und `value` muss parsen und auflösbar sein.

use paprun::Pap;
use paprun::ast::VarKind;

fn load(file: &str) -> Pap {
    let xml = std::fs::read_to_string(format!("tests/data/{file}"))
        .unwrap_or_else(|e| panic!("{file} nicht lesbar: {e}"));
    Pap::from_xml(&xml).unwrap_or_else(|e| panic!("{file} konnte nicht geladen werden: {e}"))
}

#[test]
fn loads_2025() {
    let pap = load("Lohnsteuer2025.xml");
    assert_eq!(pap.name, "Lohnsteuer2025");
    assert!(!pap.main.is_empty());
    assert!(pap.methods.len() >= 20);
    assert!(pap.var_id("LSTLZZ").is_some());
    assert!(pap.var_id("RE4").is_some());
}

#[test]
fn loads_2026() {
    let pap = load("Lohnsteuer2026.xml");
    assert_eq!(pap.name, "Lohnsteuer2026");
    assert!(!pap.main.is_empty());
    assert!(pap.var_id("LSTLZZ").is_some());
}

#[test]
fn declares_expected_variable_kinds() {
    for file in ["Lohnsteuer2025.xml", "Lohnsteuer2026.xml"] {
        let pap = load(file);
        let count = |kind| pap.vars_of_kind(kind).count();
        assert!(count(VarKind::Input) >= 30, "{file}: zu wenige INPUTS");
        assert!(count(VarKind::Output) >= 10, "{file}: zu wenige OUTPUTS");
        assert!(
            count(VarKind::Internal) >= 50,
            "{file}: zu wenige INTERNALS"
        );
        assert!(
            count(VarKind::Constant) >= 10,
            "{file}: zu wenige CONSTANTS"
        );

        // Die Alterstabellen sind die einzigen Arrays und haben 55 Einträge.
        let tab1 = pap.var(pap.var_id("TAB1").expect("TAB1 fehlt"));
        match &tab1.default {
            paprun::Value::Arr(items) => assert_eq!(items.len(), 55, "{file}: TAB1-Länge"),
            other => panic!("{file}: TAB1 ist kein Array, sondern {other:?}"),
        }
    }
}
