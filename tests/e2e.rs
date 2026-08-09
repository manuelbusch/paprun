//! End-to-End-Prüfung des Interpreters.
//!
//! Kernstück ist [`tariff_matches_independent_reference`]: Der Einkommensteuer-
//! tarif nach §32a EStG (und das Sonderverfahren der Steuerklassen 5/6) ist hier
//! **unabhängig** vom Interpreter direkt aus den Gesetzesformeln implementiert.
//! Stimmen beide Wege über den gesamten Einkommensbereich und alle Steuerklassen
//! überein, ist die BigDecimal-Kette des Interpreters — Rundungsmodi, Scale-
//! Semantik, Verzweigungslogik — belastbar geprüft.

use bigdecimal::BigDecimal;
use paprun::Pap;
use paprun::value::{RoundMode, div_scale, set_scale};
use std::str::FromStr;
use std::sync::OnceLock;

fn pap(year: u16) -> &'static Pap {
    static PAPS: OnceLock<Vec<(u16, Pap)>> = OnceLock::new();
    let paps = PAPS.get_or_init(|| {
        [2025u16, 2026]
            .into_iter()
            .map(|y| {
                let xml = std::fs::read_to_string(format!("tests/data/Lohnsteuer{y}.xml"))
                    .expect("PAP-XML nicht lesbar");
                (y, Pap::from_xml(&xml).expect("PAP nicht ladbar"))
            })
            .collect()
    });
    &paps.iter().find(|(y, _)| *y == year).expect("Jahr fehlt").1
}

fn dec(s: &str) -> BigDecimal {
    BigDecimal::from_str(s).expect("Dezimalzahl")
}

/// Lauf mit typischen Eingaben; liefert alle Variablen als Name→Wert.
fn run(year: u16, stkl: i32, re4_cent: i64, extra: &[(&str, &str)]) -> Vec<(String, String)> {
    let pap = pap(year);
    let mut inputs = pap.new_inputs();
    inputs.set("LZZ", "1").unwrap();
    inputs.set("STKL", &stkl.to_string()).unwrap();
    inputs.set("RE4", &re4_cent.to_string()).unwrap();
    inputs.set("KVZ", "2.5").unwrap();
    inputs.set("PVZ", "1").unwrap();
    for (name, value) in extra {
        inputs.set(name, value).unwrap();
    }
    pap.run_all(&inputs).expect("Lauf fehlgeschlagen")
}

fn value_of(vars: &[(String, String)], name: &str) -> BigDecimal {
    let text = vars
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("Variable `{name}` fehlt"))
        .1
        .as_str();
    dec(text)
}

// ---------------------------------------------------------------------------
// Unabhängige Referenzimplementierung des Tarifs
// ---------------------------------------------------------------------------

/// Zonenparameter eines Jahrestarifs nach §32a EStG.
struct Tariff {
    grundfreibetrag: i64,
    zone2_end: i64,
    zone2_faktor: &'static str,
    zone3_start: i64,
    zone3_end: i64,
    zone3_faktor: &'static str,
    zone3_konstante: &'static str,
    zone4_end: i64,
    zone4_abzug: &'static str,
    zone5_abzug: &'static str,
    /// Grenzwerte W1/W2/W3 des Sonderverfahrens für Steuerklasse 5/6.
    w: [i64; 3],
    /// Nachkommastellen, auf die `UP5_6` die Stützstellen abschneidet
    /// (2025: 2, ab 2026: 0).
    up5_6_scale: i64,
}

const TARIFF_2025: Tariff = Tariff {
    grundfreibetrag: 12096,
    zone2_end: 17444,
    zone2_faktor: "932.30",
    zone3_start: 17443,
    zone3_end: 68481,
    zone3_faktor: "176.64",
    zone3_konstante: "1015.13",
    zone4_end: 277826,
    zone4_abzug: "10911.92",
    zone5_abzug: "19246.67",
    w: [13785, 34240, 222260],
    up5_6_scale: 2,
};

const TARIFF_2026: Tariff = Tariff {
    grundfreibetrag: 12348,
    zone2_end: 17800,
    zone2_faktor: "914.51",
    zone3_start: 17799,
    zone3_end: 69879,
    zone3_faktor: "173.1",
    zone3_konstante: "1034.87",
    zone4_end: 277826,
    zone4_abzug: "11135.63",
    zone5_abzug: "19470.38",
    w: [14071, 34939, 222260],
    up5_6_scale: 0,
};

fn tariff(year: u16) -> &'static Tariff {
    match year {
        2025 => &TARIFF_2025,
        2026 => &TARIFF_2026,
        other => panic!("kein Referenztarif für {other}"),
    }
}

fn trunc(x: &BigDecimal, scale: i64) -> BigDecimal {
    set_scale(x, scale, RoundMode::Down).expect("setScale")
}

/// Einkommensteuer auf das zu versteuernde Einkommen `x` (ohne Splittingfaktor).
fn est(t: &Tariff, x: &BigDecimal) -> BigDecimal {
    let ten_k = BigDecimal::from(10000);
    if *x < BigDecimal::from(t.grundfreibetrag + 1) {
        BigDecimal::from(0)
    } else if *x < BigDecimal::from(t.zone2_end) {
        let y = div_scale(
            &(x - BigDecimal::from(t.grundfreibetrag)),
            &ten_k,
            6,
            RoundMode::Down,
        )
        .unwrap();
        trunc(
            &((&y * dec(t.zone2_faktor) + BigDecimal::from(1400)) * &y),
            0,
        )
    } else if *x < BigDecimal::from(t.zone3_end) {
        let y = div_scale(
            &(x - BigDecimal::from(t.zone3_start)),
            &ten_k,
            6,
            RoundMode::Down,
        )
        .unwrap();
        let rw = (&y * dec(t.zone3_faktor) + BigDecimal::from(2397)) * &y;
        trunc(&(rw + dec(t.zone3_konstante)), 0)
    } else if *x < BigDecimal::from(t.zone4_end) {
        trunc(&(x * dec("0.42") - dec(t.zone4_abzug)), 0)
    } else {
        trunc(&(x * dec("0.45") - dec(t.zone5_abzug)), 0)
    }
}

/// Zwischenschritt des Sonderverfahrens (PAP-Methode `UP5_6`).
fn up5_6(t: &Tariff, zx: &BigDecimal) -> BigDecimal {
    let st1 = est(t, &trunc(&(zx * dec("1.25")), t.up5_6_scale));
    let st2 = est(t, &trunc(&(zx * dec("0.75")), t.up5_6_scale));
    let diff = (st1 - st2) * BigDecimal::from(2);
    let mist = trunc(&(zx * dec("0.14")), 0);
    if mist > diff { mist } else { diff }
}

/// Sonderverfahren der Steuerklassen 5 und 6 (PAP-Methode `MST5_6`).
fn mst5_6(t: &Tariff, zzx: &BigDecimal) -> BigDecimal {
    let [w1, w2, w3] = t.w.map(BigDecimal::from);
    if *zzx > w2 {
        let st = up5_6(t, &w2);
        return if *zzx > w3 {
            let st = trunc(&(st + (&w3 - &w2) * dec("0.42")), 0);
            trunc(&(st + (zzx - &w3) * dec("0.45")), 0)
        } else {
            trunc(&(st + (zzx - &w2) * dec("0.42")), 0)
        };
    }
    let vergl = up5_6(t, zzx);
    if *zzx > w1 {
        let hoch = trunc(&(up5_6(t, &w1) + (zzx - &w1) * dec("0.42")), 0);
        if hoch < vergl { hoch } else { vergl }
    } else {
        vergl
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn tariff_matches_independent_reference() {
    let mut checked = 0;
    for year in [2025u16, 2026] {
        let t = tariff(year);
        // Raster über den relevanten Einkommensbereich plus alle Zonengrenzen.
        let mut incomes: Vec<i64> = (0..=400_000).step_by(2_500).collect();
        incomes.extend([
            t.grundfreibetrag,
            t.grundfreibetrag + 1,
            t.zone2_end - 1,
            t.zone2_end,
            t.zone3_end - 1,
            t.zone3_end,
            t.zone4_end - 1,
            t.zone4_end,
            t.w[0],
            t.w[1],
        ]);

        for stkl in 1..=6 {
            for euro in &incomes {
                let vars = run(year, stkl, euro * 100, &[]);
                let zve = value_of(&vars, "ZVE");
                let kztab = value_of(&vars, "KZTAB");
                let actual = value_of(&vars, "ST");

                let expected = if stkl >= 5 {
                    mst5_6(t, &trunc(&zve, 0))
                } else {
                    // KZTAB=2 ist das Splittingverfahren der Steuerklasse 3.
                    let x = trunc(&(&zve / &kztab), 0);
                    est(t, &x) * &kztab
                };

                assert_eq!(
                    actual, expected,
                    "{year}, StKl {stkl}, RE4={euro} EUR: ZVE={zve}, KZTAB={kztab}"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 1_500, "zu wenige Fälle geprüft: {checked}");
}

#[test]
fn zero_income_yields_zero_tax() {
    for year in [2025u16, 2026] {
        let vars = run(year, 1, 0, &[]);
        assert_eq!(value_of(&vars, "LSTLZZ"), BigDecimal::from(0), "{year}");
        assert_eq!(value_of(&vars, "SOLZLZZ"), BigDecimal::from(0), "{year}");
    }
}

#[test]
fn income_below_basic_allowance_is_tax_free() {
    // 10.000 EUR liegen in jeder Klasse außer 5/6 unter dem Grundfreibetrag.
    for stkl in 1..=4 {
        let vars = run(2025, stkl, 1_000_000, &[]);
        assert_eq!(
            value_of(&vars, "LSTLZZ"),
            BigDecimal::from(0),
            "StKl {stkl} bei 10.000 EUR"
        );
    }
}

#[test]
fn tax_increases_monotonically_with_income() {
    for stkl in 1..=6 {
        let mut previous = BigDecimal::from(-1);
        for euro in (0..=200_000).step_by(5_000) {
            let current = value_of(&run(2025, stkl, euro * 100, &[]), "LSTLZZ");
            assert!(
                current >= previous,
                "StKl {stkl}: Lohnsteuer sinkt bei {euro} EUR ({previous} → {current})"
            );
            previous = current;
        }
    }
}

#[test]
fn tax_classes_are_ordered_as_expected() {
    // Bei gleichem Bruttolohn: Klasse 3 günstiger als 1, Klasse 1 günstiger als 5.
    let tax = |stkl| value_of(&run(2025, stkl, 5_000_000, &[]), "LSTLZZ");
    assert!(tax(3) < tax(1), "StKl 3 muss günstiger sein als StKl 1");
    assert!(tax(1) < tax(5), "StKl 1 muss günstiger sein als StKl 5");
    assert!(
        tax(5) <= tax(6),
        "StKl 6 darf nicht günstiger sein als StKl 5"
    );
}

#[test]
fn monthly_and_yearly_periods_are_consistent() {
    // LZZ=1 (Jahr) gegen LZZ=2 (Monat): Zwölffaches des Monatswerts muss dem
    // Jahreswert entsprechen (Rundungsdifferenzen im Cent-Bereich sind normal).
    let pap = pap(2025);
    for euro in [24_000i64, 50_000, 90_000] {
        let yearly = value_of(&run(2025, 1, euro * 100, &[]), "LSTLZZ");

        let mut inputs = pap.new_inputs();
        inputs.set("LZZ", "2").unwrap();
        inputs.set("STKL", "1").unwrap();
        inputs.set("RE4", &(euro * 100 / 12).to_string()).unwrap();
        inputs.set("KVZ", "2.5").unwrap();
        inputs.set("PVZ", "1").unwrap();
        let monthly = value_of(&pap.run_all(&inputs).unwrap(), "LSTLZZ");

        let difference = (&yearly - monthly * BigDecimal::from(12)).abs();
        assert!(
            difference < BigDecimal::from(20_000),
            "{euro} EUR: Jahres- und Monatsberechnung weichen um {difference} Cent ab"
        );
    }
}

/// Regressionsschutz: Werte aus einem Lauf, der gegen die unabhängige
/// Tarifreferenz oben geprüft wurde.
#[test]
fn known_values_stay_stable() {
    let cases = [
        // (Jahr, Steuerklasse, Bruttojahreslohn in Cent, LSTLZZ in Cent)
        (2025u16, 1, 5_000_000i64, "692700"),
        (2025, 3, 5_000_000, "297000"),
        (2025, 5, 5_000_000, "1213500"),
        (2026, 1, 5_000_000, "681900"),
        (2026, 3, 5_000_000, "283400"),
        (2026, 5, 5_000_000, "1205200"),
    ];
    for (year, stkl, re4, expected) in cases {
        let vars = run(year, stkl, re4, &[]);
        assert_eq!(
            value_of(&vars, "LSTLZZ"),
            dec(expected),
            "{year}, StKl {stkl}, RE4={re4}"
        );
    }
}

#[test]
fn rejects_invalid_inputs() {
    let pap = pap(2025);
    let mut inputs = pap.new_inputs();
    assert!(inputs.set("GIBTESNICHT", "1").is_err());
    assert!(
        inputs.set("LSTLZZ", "1").is_err(),
        "OUTPUT darf nicht gesetzt werden"
    );
    assert!(inputs.set("STKL", "eins").is_err());
}
