//! Schließt den Kreis: XML laden → als YAML ausgeben → wieder laden → rechnen.
//!
//! Beide Wege müssen für jeden geprüften Fall exakt dieselben Ausgabewerte
//! liefern. Erst damit ist das YAML-Format als verlustfreie Darstellung des
//! amtlichen Programmablaufplans belegt.

use paprun::Pap;
use paprun::ast::VarKind;
use paprun::emit::to_yaml;
use std::sync::OnceLock;

/// Jeweils der aus XML geladene PAP und derselbe über YAML zurückgeholt.
fn pair(year: u16) -> &'static (Pap, Pap) {
    static PAIRS: OnceLock<Vec<(u16, (Pap, Pap))>> = OnceLock::new();
    let pairs = PAIRS.get_or_init(|| {
        [2025u16, 2026]
            .into_iter()
            .map(|year| {
                let xml = std::fs::read_to_string(format!("tests/data/Lohnsteuer{year}.xml"))
                    .expect("PAP-XML nicht lesbar");
                let from_xml = Pap::from_xml(&xml).expect("XML nicht ladbar");
                let yaml = to_yaml(&from_xml);
                let from_yaml = Pap::from_yaml(&yaml)
                    .unwrap_or_else(|e| panic!("{year}: YAML nicht ladbar: {e}"));
                (year, (from_xml, from_yaml))
            })
            .collect()
    });
    &pairs
        .iter()
        .find(|(y, _)| *y == year)
        .expect("Jahr fehlt")
        .1
}

fn run(pap: &Pap, stkl: i32, re4: i64, lzz: i32) -> Vec<(String, String)> {
    let mut inputs = pap.new_inputs();
    inputs.set("LZZ", &lzz.to_string()).unwrap();
    inputs.set("STKL", &stkl.to_string()).unwrap();
    inputs.set("RE4", &re4.to_string()).unwrap();
    inputs.set("KVZ", "2.5").unwrap();
    inputs.set("PVZ", "1").unwrap();
    pap.run(&inputs).expect("Lauf fehlgeschlagen")
}

#[test]
fn yaml_computes_exactly_like_the_original_xml() {
    let mut checked = 0;
    for year in [2025u16, 2026] {
        let (from_xml, from_yaml) = pair(year);
        for stkl in 1..=6 {
            for euro in (0..=250_000).step_by(2_500) {
                for lzz in [1, 2] {
                    let expected = run(from_xml, stkl, euro * 100, lzz);
                    let actual = run(from_yaml, stkl, euro * 100, lzz);
                    assert_eq!(
                        actual, expected,
                        "{year}, StKl {stkl}, RE4={euro} EUR, LZZ={lzz}"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 2_000, "zu wenige Fälle geprüft: {checked}");
}

#[test]
fn declarations_survive_the_round_trip() {
    for year in [2025u16, 2026] {
        let (from_xml, from_yaml) = pair(year);
        assert_eq!(from_yaml.name, from_xml.name, "{year}: Name");
        assert_eq!(from_yaml.version, from_xml.version, "{year}: Version");
        assert_eq!(
            from_yaml.vars.len(),
            from_xml.vars.len(),
            "{year}: Variablenzahl"
        );
        assert_eq!(
            from_yaml.methods.len(),
            from_xml.methods.len(),
            "{year}: Methodenzahl"
        );

        for (expected, actual) in from_xml.vars.iter().zip(&from_yaml.vars) {
            assert_eq!(actual.name, expected.name, "{year}: Variablenreihenfolge");
            assert_eq!(actual.ty, expected.ty, "{year}: Typ von {}", expected.name);
            assert_eq!(
                actual.kind, expected.kind,
                "{year}: Art von {}",
                expected.name
            );
            assert_eq!(
                paprun::format_value(&actual.default),
                paprun::format_value(&expected.default),
                "{year}: Default von {}",
                expected.name
            );
            assert_eq!(
                actual.group, expected.group,
                "{year}: Gruppe von {}",
                expected.name
            );
        }
    }
}

#[test]
fn exporting_the_reimported_pap_is_stable() {
    // Zweiter Durchlauf darf nichts mehr verändern.
    for year in [2025u16, 2026] {
        let (from_xml, from_yaml) = pair(year);
        assert_eq!(to_yaml(from_yaml), to_yaml(from_xml), "{year}");
    }
}

#[test]
fn decimal_places_are_preserved() {
    // Ein Wert mit nachlaufender Null darf nicht über f64 laufen.
    let yaml = r#"
name: Skalen
inputs:
  BETRAG: { type: "BigDecimal" }
outputs:
  ERGEBNIS: { type: "BigDecimal" }
constants:
  FAKTOR: { type: "BigDecimal", value: "0.10" }
main:
  - eval: "ERGEBNIS = BETRAG * FAKTOR"
"#;
    let pap = Pap::from_yaml(yaml).expect("YAML nicht ladbar");
    let mut inputs = pap.new_inputs();
    inputs.set("BETRAG", "3").unwrap();
    let outputs = pap.run(&inputs).unwrap();
    // 3 * 0.10 = 0.30 — der Scale von zwei Stellen muss erhalten bleiben.
    assert_eq!(outputs[0], ("ERGEBNIS".into(), "0.30".into()));
}

#[test]
fn a_handwritten_pap_runs() {
    let yaml = r#"
name: Beispielabgabe
version: "1"

inputs:
  BRUTTO:  { type: "BigDecimal" }
  KINDER:  { type: int }
outputs:
  ABGABE:  { type: "BigDecimal" }
internals:
  BEMESSUNG: { type: "BigDecimal" }
constants:
  FREIBETRAG:    { type: "BigDecimal", value: "1000" }
  SATZ:          { type: "BigDecimal", value: "0.2" }
  KINDERBONUS:   { type: "BigDecimal", value: "250" }

main:
  - eval: "BEMESSUNG = BRUTTO - FREIBETRAG - dec(KINDER) * KINDERBONUS"
  - if: "BEMESSUNG > dec(0)"
    then:
      - call: BERECHNE
    else:
      - eval: "ABGABE = dec(0)"

methods:
  BERECHNE:
    - eval: "ABGABE = scale(BEMESSUNG * SATZ, 2, half_up)"
"#;
    let pap = Pap::from_yaml(yaml).expect("YAML nicht ladbar");

    let abgabe = |brutto: &str, kinder: &str| {
        let mut inputs = pap.new_inputs();
        inputs.set("BRUTTO", brutto).unwrap();
        inputs.set("KINDER", kinder).unwrap();
        pap.run(&inputs).unwrap()[0].1.clone()
    };

    // (5000 - 1000 - 0) * 0,2
    assert_eq!(abgabe("5000", "0"), "800.00");
    // (5000 - 1000 - 500) * 0,2
    assert_eq!(abgabe("5000", "2"), "700.00");
    // Unter dem Freibetrag bleibt es bei null.
    assert_eq!(abgabe("800", "0"), "0");
}

#[test]
fn elif_chains_are_read_back_correctly() {
    let yaml = r#"
name: Stufen
inputs:
  X: { type: int }
outputs:
  STUFE: { type: int }
main:
  - if: "X < 10"
    then:
      - eval: "STUFE = 1"
  - elif: "X < 20"
    then:
      - eval: "STUFE = 2"
  - elif: "X < 30"
    then:
      - eval: "STUFE = 3"
    else:
      - eval: "STUFE = 4"
"#;
    let pap = Pap::from_yaml(yaml).unwrap();
    for (x, expected) in [(5, "1"), (15, "2"), (25, "3"), (99, "4")] {
        let mut inputs = pap.new_inputs();
        inputs.set("X", &x.to_string()).unwrap();
        assert_eq!(pap.run(&inputs).unwrap()[0].1, expected, "für X={x}");
    }
}

#[test]
fn format_is_detected_automatically() {
    let xml = std::fs::read_to_string("tests/data/Lohnsteuer2025.xml").unwrap();
    assert_eq!(Pap::from_source(&xml).unwrap().name, "Lohnsteuer2025");

    let yaml = to_yaml(&Pap::from_xml(&xml).unwrap());
    assert_eq!(Pap::from_source(&yaml).unwrap().name, "Lohnsteuer2025");
}

#[test]
fn errors_are_reported_clearly() {
    let cases = [
        ("name: X\nmain: 5\n", "Liste"),
        ("name: X\ninputs: 3\n", "Abbildung"),
        (
            "name: X\ninputs:\n  A: { type: Unsinn }\n",
            "unbekannter Typ",
        ),
        (
            "name: X\nconstants:\n  A: { type: \"BigDecimal\" }\n",
            "value",
        ),
        (
            "name: X\nmain:\n  - elif: \"1 == 1\"\n    then: []\n",
            "ohne vorangehendes",
        ),
        ("name: X\nmain:\n  - unsinn: 1\n", "eval"),
        ("name: X\nmain:\n  - call: FEHLT\n", "unbekannte Methode"),
    ];
    for (yaml, expected) in cases {
        let error = Pap::from_yaml(yaml)
            .err()
            .unwrap_or_else(|| panic!("kein Fehler für:\n{yaml}"));
        assert!(
            error.to_string().contains(expected),
            "Meldung `{error}` sollte `{expected}` enthalten (für {yaml:?})"
        );
    }
}

#[test]
fn output_variables_keep_their_group() {
    let (from_xml, from_yaml) = pair(2025);
    let groups = |pap: &Pap| -> Vec<String> {
        pap.vars_of_kind(VarKind::Output)
            .map(|(_, decl)| format!("{}:{}", decl.name, decl.group))
            .collect()
    };
    assert_eq!(groups(from_yaml), groups(from_xml));
}
