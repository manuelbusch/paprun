//! Grobe Laufzeitmessung: `cargo run --release --example bench`

use paprun::Pap;
use std::time::Instant;

fn main() {
    let xml = std::fs::read_to_string("tests/data/Lohnsteuer2025.xml").unwrap();

    let t = Instant::now();
    let runs = 200;
    for _ in 0..runs {
        std::hint::black_box(Pap::from_xml(&xml).unwrap());
    }
    let load = t.elapsed() / runs;

    let pap = Pap::from_xml(&xml).unwrap();
    let mut inputs = pap.new_inputs();
    inputs.set("LZZ", "1").unwrap();
    inputs.set("STKL", "1").unwrap();
    inputs.set("RE4", "5000000").unwrap();
    inputs.set("KVZ", "2.5").unwrap();
    inputs.set("PVZ", "1").unwrap();

    // Aufwärmen
    for _ in 0..1000 {
        std::hint::black_box(pap.run(&inputs).unwrap());
    }

    let t = Instant::now();
    let runs = 50_000;
    for _ in 0..runs {
        std::hint::black_box(pap.run(&inputs).unwrap());
    }
    let elapsed = t.elapsed();
    let per_run = elapsed / runs;

    println!("Laden:     {load:?} pro PAP");
    println!("Berechnen: {per_run:?} pro Lauf");
    println!(
        "Durchsatz: {:.0} Berechnungen/s (1 Kern)",
        runs as f64 / elapsed.as_secs_f64()
    );

    // Pap ist Send + Sync und kann von mehreren Threads geteilt werden.
    let threads = std::thread::available_parallelism().unwrap().get();
    let per_thread = 20_000u32;
    let t = Instant::now();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                let mut inputs = pap.new_inputs();
                inputs.set("LZZ", "1").unwrap();
                inputs.set("STKL", "1").unwrap();
                inputs.set("RE4", "5000000").unwrap();
                for _ in 0..per_thread {
                    std::hint::black_box(pap.run(&inputs).unwrap());
                }
            });
        }
    });
    let total = threads as u32 * per_thread;
    println!(
        "Durchsatz: {:.0} Berechnungen/s ({threads} Threads)",
        total as f64 / t.elapsed().as_secs_f64()
    );
}
