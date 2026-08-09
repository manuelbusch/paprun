//! Prüft den YAML-Export gegen die amtlichen PAP-Dateien.
//!
//! Kernstück ist [`every_expression_survives_the_round_trip`]: Jeder Ausdruck
//! beider Jahrgänge wird ausgegeben, wieder eingelesen und dann mit identischer
//! Variablenbelegung ausgewertet. Original und Rückübersetzung müssen exakt
//! dasselbe liefern — auch dieselben Fehler. Das prüft die Übersetzung
//! semantisch statt nur strukturell, denn Methodenaufrufe wie `a.add(b)`
//! werden bewusst zu Operatoren (`a + b`) vereinfacht.

use paprun::Pap;
use paprun::ast::{Expr, Stmt};
use paprun::emit::{expr_to_string, to_yaml};
use paprun::eval::eval_expr;
use paprun::parse::parse_expr;
use std::collections::HashMap;
use std::sync::OnceLock;

fn paps() -> &'static Vec<(u16, Pap)> {
    static PAPS: OnceLock<Vec<(u16, Pap)>> = OnceLock::new();
    PAPS.get_or_init(|| {
        [2025u16, 2026]
            .into_iter()
            .map(|year| {
                let xml = std::fs::read_to_string(format!("tests/data/Lohnsteuer{year}.xml"))
                    .expect("PAP-XML nicht lesbar");
                (year, Pap::from_xml(&xml).expect("PAP nicht ladbar"))
            })
            .collect()
    })
}

/// Sammelt jeden Ausdruck eines PAP — Bedingungen wie Zuweisungen.
fn collect_expressions(pap: &Pap) -> Vec<&Expr> {
    fn walk<'a>(block: &'a [Stmt], out: &mut Vec<&'a Expr>) {
        for stmt in block {
            match stmt {
                Stmt::Eval { expr, .. } => out.push(expr),
                Stmt::Execute(_) => {}
                Stmt::If { cond, then_, else_ } => {
                    out.push(cond);
                    walk(then_, out);
                    walk(else_, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&pap.main, &mut out);
    for method in &pap.methods {
        walk(&method.body, &mut out);
    }
    out
}

/// Variablenbelegung für die Auswertung: Defaults, aber mit Werten, die
/// typische Zweige erreichbar machen.
fn test_slots(pap: &Pap, seed: i64) -> Vec<paprun::Value> {
    let mut slots: Vec<paprun::Value> = pap.vars.iter().map(|v| v.default.clone()).collect();
    for (index, decl) in pap.vars.iter().enumerate() {
        slots[index] = match decl.ty {
            paprun::ast::Ty::Int => paprun::Value::Int(seed % 7),
            paprun::ast::Ty::Dec => {
                paprun::Value::Dec(bigdecimal::BigDecimal::from(seed * 1000 + 17))
            }
            paprun::ast::Ty::Dbl => paprun::Value::Dbl(1.0),
            paprun::ast::Ty::DecArr => decl.default.clone(),
        };
    }
    slots
}

#[test]
fn every_expression_survives_the_round_trip() {
    let mut checked = 0;
    for (year, pap) in paps() {
        // Namensauflösung für das Wiedereinlesen.
        let scope: HashMap<String, u32> = pap
            .vars
            .iter()
            .enumerate()
            .map(|(index, decl)| (decl.name.clone(), index as u32))
            .collect();

        for expr in collect_expressions(pap) {
            let printed = expr_to_string(pap, expr);
            let reparsed = parse_expr(&printed, &scope)
                .unwrap_or_else(|e| panic!("{year}: `{printed}` nicht wieder lesbar: {e}"));

            // Ausgabe muss stabil sein: erneutes Schreiben ändert nichts.
            assert_eq!(
                expr_to_string(pap, &reparsed),
                printed,
                "{year}: Ausgabe ist nicht idempotent"
            );

            // Und beide müssen gleich rechnen — inklusive gleicher Fehler.
            for seed in [0i64, 3, 11] {
                let slots = test_slots(pap, seed);
                let original = eval_expr(expr, &slots);
                let round_trip = eval_expr(&reparsed, &slots);
                assert_eq!(
                    original.is_ok(),
                    round_trip.is_ok(),
                    "{year}: `{printed}` verhält sich anders (seed {seed})"
                );
                if let (Ok(a), Ok(b)) = (original, round_trip) {
                    assert_eq!(
                        paprun::format_value(&a),
                        paprun::format_value(&b),
                        "{year}: `{printed}` liefert ein anderes Ergebnis (seed {seed})"
                    );
                }
            }
            checked += 1;
        }
    }
    assert!(checked > 500, "zu wenige Ausdrücke geprüft: {checked}");
}

#[test]
fn yaml_is_well_formed_and_complete() {
    for (year, pap) in paps() {
        let yaml = to_yaml(pap);

        // Alle Abschnitte vorhanden.
        for section in [
            "name:",
            "inputs:",
            "outputs:",
            "internals:",
            "constants:",
            "main:",
            "methods:",
        ] {
            assert!(yaml.contains(section), "{year}: `{section}` fehlt");
        }
        assert!(yaml.contains(&format!("name: \"Lohnsteuer{year}\"")));

        // Jede Methode taucht als Schlüssel auf.
        for method in &pap.methods {
            assert!(
                yaml.contains(&format!("  {}:\n", method.name)),
                "{year}: Methode {} fehlt",
                method.name
            );
        }

        // Einrückung: Nur Vielfache von zwei Leerzeichen, keine Tabulatoren.
        for (number, line) in yaml.lines().enumerate() {
            assert!(
                !line.contains('\t'),
                "{year}: Tabulator in Zeile {}",
                number + 1
            );
            let indent = line.len() - line.trim_start().len();
            assert_eq!(
                indent % 2,
                0,
                "{year}: ungerade Einrückung in Zeile {}",
                number + 1
            );
        }
    }
}

#[test]
fn nested_conditionals_are_folded_into_elif() {
    let (_, pap) = &paps()[0];
    let yaml = to_yaml(pap);
    assert!(yaml.contains("- elif:"), "keine elif-Faltung erzeugt");

    // Der Tarifteil hat fünf Zweige; ohne Faltung würde er tief einrücken.
    let deepest = yaml
        .lines()
        .map(|line| line.len() - line.trim_start().len())
        .max()
        .unwrap();
    assert!(
        deepest <= 20,
        "Einrückung wird zu tief: {deepest} Leerzeichen"
    );
}

#[test]
fn compare_to_becomes_a_comparison_operator() {
    let (_, pap) = &paps()[0];
    let scope: HashMap<String, u32> = pap
        .vars
        .iter()
        .enumerate()
        .map(|(index, decl)| (decl.name.clone(), index as u32))
        .collect();

    for (java, expected) in [
        ("X.compareTo(GFB) == -1", "X < GFB"),
        ("X.compareTo(GFB) == 0", "X == GFB"),
        ("X.compareTo(GFB) == 1", "X > GFB"),
        ("X.compareTo(GFB) != -1", "X >= GFB"),
        ("X.compareTo(GFB) != 0", "X != GFB"),
        ("X.compareTo(GFB) != 1", "X <= GFB"),
        // Andere Konstanten bleiben unangetastet.
        ("X.compareTo(GFB) == 2", "cmp(X, GFB) == 2"),
    ] {
        let expr = parse_expr(java, &scope).unwrap();
        assert_eq!(expr_to_string(pap, &expr), expected, "für `{java}`");
    }
}

#[test]
fn expressions_are_readable() {
    let (_, pap) = &paps()[0];
    let yaml = to_yaml(pap);
    // Methodenaufrufe der Grundrechenarten werden zu Operatoren.
    assert!(
        yaml.contains(" / "),
        "Divisionen sollten als Operator erscheinen"
    );
    assert!(
        yaml.contains("div("),
        "gerundete Division sollte `div(` nutzen"
    );
    assert!(yaml.contains("scale("), "setScale sollte `scale(` nutzen");
    assert!(
        !yaml.contains("BigDecimal.ROUND_"),
        "Rundungsmodi sollten kurz sein"
    );
    assert!(
        !yaml.contains(".divide("),
        "Java-Methodensyntax sollte verschwinden"
    );
}
