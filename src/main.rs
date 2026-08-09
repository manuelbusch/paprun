use paprun::ast::VarKind;
use paprun::{Error, Pap, format_value};
use std::process::ExitCode;

const USAGE: &str = "\
paprun — wertet die XML-Programmablaufpläne des BMF aus

Aufruf:
  paprun <PAP.xml> [--in NAME=WERT]... [--json] [--vars]

Optionen:
  --in NAME=WERT   Eingabevariable setzen (wiederholbar).
                   Geldbeträge in Cent, z. B. --in RE4=5000000 für 50.000,00 EUR
  --vars           Ein- und Ausgabevariablen mit Typ und Default auflisten
  --all            alle Variablen ausgeben, auch interne Zwischenergebnisse
  --json           Ausgaben als flaches JSON-Objekt
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

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => json = true,
            "--vars" => list_vars = true,
            "--all" => all = true,
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
