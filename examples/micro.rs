use bigdecimal::BigDecimal;
use paprun::Pap;
use paprun::value::{RoundMode, div_scale, set_scale};
use std::hint::black_box;
use std::str::FromStr;
use std::time::Instant;

fn time<T>(name: &str, iters: u32, mut f: impl FnMut() -> T) {
    for _ in 0..iters / 10 {
        black_box(f());
    }
    let t = Instant::now();
    for _ in 0..iters {
        black_box(f());
    }
    println!(
        "{name:<34} {:>8.1} ns",
        t.elapsed().as_nanos() as f64 / iters as f64
    );
}

fn main() {
    let xml = std::fs::read_to_string("tests/data/Lohnsteuer2025.xml").unwrap();
    let pap = Pap::from_xml(&xml).unwrap();

    time("Env-Init (133 Value-Klone)", 200_000, || {
        pap.vars
            .iter()
            .map(|v| v.default.clone())
            .collect::<Vec<_>>()
    });

    let a = BigDecimal::from_str("38759.00").unwrap();
    let b = BigDecimal::from_str("10000").unwrap();
    let z = BigDecimal::from(0);
    time("BigDecimal::clone (Wert 0)", 2_000_000, || z.clone());
    time("BigDecimal::clone (38759.00)", 2_000_000, || a.clone());
    time("add", 2_000_000, || &a + &b);
    time("multiply", 2_000_000, || &a * &b);
    time("div_scale(.., 6, Down)", 2_000_000, || {
        div_scale(&a, &b, 6, RoundMode::Down).unwrap()
    });
    time("set_scale(.., 0, Down)", 2_000_000, || {
        set_scale(&a, 0, RoundMode::Down).unwrap()
    });
    time("compareTo", 2_000_000, || a.cmp(&b));
}
