use paprun::ast::VarKind;
use paprun::batch::{self, CsvOptions};
use paprun::{Error, Pap, format_value};
use std::process::ExitCode;

const USAGE: &str = "\
paprun — wertet die XML-Programmablaufpläne des BMF aus

Aufruf:
  paprun <PAP.xml> [--in NAME=WERT]... [--json] [--vars]
  paprun <PAP.xml> --csv [--delimiter Z] [--passthrough]   < ein.csv > aus.csv

Einzelfall:
  --in NAME=WERT   Eingabevariable setzen (wiederholbar).
                   Geldbeträge in Cent, z. B. --in RE4=5000000 für 50.000,00 EUR
  --all            alle Variablen ausgeben, auch interne Zwischenergebnisse
  --json           Ausgaben als flaches JSON-Objekt

Stapelverarbeitung:
  --csv            CSV von der Standardeingabe lesen, Ergebnisse als CSV auf
                   die Standardausgabe schreiben. Die Kopfzeile benennt die
                   Eingabevariablen, jede Datenzeile ist ein Fall. Fehlende
                   oder leere Felder verwenden den Default der Variablen.
  --delimiter Z    Trennzeichen (Standard `,`; deutsche Exporte oft `;`)
  --passthrough    Spalten, die keine Eingabevariablen sind (etwa
                   Personalnummern), unverändert in die Ausgabe übernehmen
  --template       CSV-Kopfzeile mit allen Eingabevariablen ausgeben

Allgemein:
  --vars           Ein- und Ausgabevariablen mit Typ und Default auflisten
  -h, --help       diese Hilfe";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") || args.is_empty() {
        println!("{USAGE}");
        return if args.is_empty() {
            ExitCode::from(2)
        } else {
            ExitCode::SUCCESS
        };
    }

    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Usage(msg)) => {
            eprintln!("Fehler: {msg}\n\n{USAGE}");
            ExitCode::from(2)
        }
        Err(CliError::Run(e)) => {
            eprintln!("Fehler: {e}");
            ExitCode::FAILURE
        }
    }
}

enum CliError {
    Usage(String),
    Run(Error),
}

impl From<Error> for CliError {
    fn from(e: Error) -> Self {
        CliError::Run(e)
    }
}

fn run(args: &[String]) -> Result<(), CliError> {
    let mut path: Option<&str> = None;
    let mut assignments: Vec<&str> = Vec::new();
    let mut json = false;
    let mut list_vars = false;
    let mut all = false;
    let mut csv_mode = false;
    let mut template = false;
    let mut csv_options = CsvOptions::default();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => json = true,
            "--vars" => list_vars = true,
            "--all" => all = true,
            "--csv" => csv_mode = true,
            "--template" => template = true,
            "--passthrough" => csv_options.passthrough = true,
            "--delimiter" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| CliError::Usage("--delimiter erwartet ein Zeichen".into()))?;
                csv_options.delimiter = single_byte(value)?;
            }
            other if other.starts_with("--delimiter=") => {
                csv_options.delimiter = single_byte(&other["--delimiter=".len()..])?;
            }
            "--in" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| CliError::Usage("--in erwartet NAME=WERT".into()))?;
                assignments.push(value);
            }
            other if other.starts_with("--in=") => assignments.push(&other["--in=".len()..]),
            other if other.starts_with('-') => {
                return Err(CliError::Usage(format!("unbekannte Option `{other}`")));
            }
            other => {
                if path.replace(other).is_some() {
                    return Err(CliError::Usage("mehr als eine XML-Datei angegeben".into()));
                }
            }
        }
        i += 1;
    }

    let path = path.ok_or_else(|| CliError::Usage("keine XML-Datei angegeben".into()))?;
    let xml = std::fs::read_to_string(path)
        .map_err(|e| CliError::Usage(format!("`{path}` nicht lesbar: {e}")))?;
    let pap = Pap::from_xml(&xml)?;

    if list_vars {
        print_vars(&pap);
        return Ok(());
    }

    if template {
        print!("{}", batch::csv_template(&pap, csv_options.delimiter));
        return Ok(());
    }

    if csv_mode {
        // Streamend: Es wird immer nur eine Zeile im Speicher gehalten.
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        let rows = batch::run_csv(
            &pap,
            stdin.lock(),
            std::io::BufWriter::new(stdout.lock()),
            &csv_options,
        )?;
        eprintln!("{rows} Zeilen verarbeitet");
        return Ok(());
    }

    let mut inputs = pap.new_inputs();
    for assignment in assignments {
        let (name, value) = assignment
            .split_once('=')
            .ok_or_else(|| CliError::Usage(format!("`{assignment}` ist kein NAME=WERT")))?;
        inputs.set(name.trim(), value)?;
    }

    let outputs = if all {
        pap.run_all(&inputs)?
    } else {
        pap.run(&inputs)?
    };
    if json {
        let body: Vec<String> = outputs
            .iter()
            .map(|(name, value)| format!("  \"{name}\": \"{value}\""))
            .collect();
        println!("{{\n{}\n}}", body.join(",\n"));
    } else {
        for (name, value) in outputs {
            println!("{name}={value}");
        }
    }
    Ok(())
}

/// Trennzeichen aus einem Argument lesen; erlaubt sind nur Einzelbyte-Zeichen.
fn single_byte(text: &str) -> Result<u8, CliError> {
    let unescaped = match text {
        "\\t" | "tab" => "\t",
        other => other,
    };
    match unescaped.as_bytes() {
        [byte] => Ok(*byte),
        _ => Err(CliError::Usage(format!(
            "`{text}` ist kein einzelnes Trennzeichen"
        ))),
    }
}

fn print_vars(pap: &Pap) {
    println!("{} (Version {})\n", pap.name, pap.version);
    println!("EINGABEN:");
    for (_, decl) in pap.vars_of_kind(VarKind::Input) {
        println!(
            "  {:<10} {:<12} default={}",
            decl.name,
            decl.ty.name(),
            format_value(&decl.default)
        );
    }
    println!("\nAUSGABEN:");
    for (_, decl) in pap.vars_of_kind(VarKind::Output) {
        let group = if decl.group.is_empty() {
            String::new()
        } else {
            format!(" [{}]", decl.group)
        };
        println!("  {:<10} {:<12}{group}", decl.name, decl.ty.name());
    }
}
