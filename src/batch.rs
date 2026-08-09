//! Stapelverarbeitung: CSV mit Eingabevariablen rein, CSV mit
//! Ausgabevariablen raus.
//!
//! Die Verarbeitung ist streamend — es wird immer nur eine Zeile im Speicher
//! gehalten, sodass beliebig große Dateien mit konstantem Speicher durchlaufen.
//! [`run_csv`] arbeitet auf `Read`/`Write` und ist damit ohne Dateisystem
//! testbar; die CLI reicht Stdin und Stdout hinein.

use crate::ast::{VarId, VarKind};
use crate::error::Error;
use crate::{Pap, format_value};
use std::io::{Read, Write};

/// Einstellungen der CSV-Verarbeitung.
#[derive(Debug, Clone)]
pub struct CsvOptions {
    /// Trennzeichen; deutsche Exporte nutzen häufig `;`.
    pub delimiter: u8,
    /// Spalten, die keine Eingabevariablen sind, unverändert in die Ausgabe
    /// übernehmen (z. B. Personalnummern). Ohne dieses Flag sind sie ein
    /// Fehler — das fängt Tippfehler in Spaltennamen ab.
    pub passthrough: bool,
    /// Anzahl rechnender Threads. `None` nutzt alle verfügbaren Kerne.
    /// Das Ergebnis ist unabhängig davon immer identisch — die
    /// Zeilenreihenfolge bleibt erhalten.
    pub threads: Option<usize>,
}

impl Default for CsvOptions {
    fn default() -> Self {
        CsvOptions {
            delimiter: b',',
            passthrough: false,
            threads: None,
        }
    }
}

/// Zeilen, die je Block gelesen und gemeinsam gerechnet werden. Groß genug,
/// dass das Starten der Threads (~0,3 ms) nicht ins Gewicht fällt, klein genug
/// für einen konstanten Speicherbedarf von wenigen Megabyte.
const CHUNK_ROWS: usize = 4096;

/// Unterhalb dieser Blockgröße lohnt sich das Verteilen nicht.
const MIN_ROWS_FOR_THREADS: usize = 64;

/// Bedeutung einer Spalte der Eingabedatei.
enum Column {
    /// Eingabevariable des PAP.
    Input(VarId),
    /// Wird unverändert in die Ausgabe übernommen.
    Passthrough,
}

/// Liest CSV von `reader`, rechnet jede Zeile und schreibt CSV nach `writer`.
/// Gibt die Anzahl verarbeiteter Datenzeilen zurück.
///
/// Die Kopfzeile benennt die Spalten; jede Datenzeile ist ein Berechnungsfall.
/// Nicht aufgeführte Eingabevariablen behalten ihren Default. Ausgabespalten
/// sind die durchgereichten Spalten, gefolgt von allen Ausgabevariablen des PAP.
pub fn run_csv(
    pap: &Pap,
    reader: impl Read,
    writer: impl Write,
    options: &CsvOptions,
) -> Result<u64, Error> {
    let mut input = csv::ReaderBuilder::new()
        .delimiter(options.delimiter)
        .flexible(false)
        .from_reader(reader);
    let mut output = csv::WriterBuilder::new()
        .delimiter(options.delimiter)
        .from_writer(writer);

    let header = input
        .headers()
        .map_err(|e| Error::io(format!("Kopfzeile nicht lesbar: {e}")))?
        .clone();
    let columns = resolve_columns(pap, &header, options)?;

    // Kopfzeile: erst die durchgereichten Spalten, dann die Ausgabevariablen.
    let passthrough_names: Vec<&str> = header
        .iter()
        .zip(&columns)
        .filter(|(_, kind)| matches!(kind, Column::Passthrough))
        .map(|(name, _)| name)
        .collect();
    let output_names: Vec<&str> = pap
        .vars_of_kind(VarKind::Output)
        .map(|(_, decl)| decl.name.as_str())
        .collect();
    output
        .write_record(passthrough_names.iter().chain(&output_names))
        .map_err(|e| Error::io(format!("Kopfzeile nicht schreibbar: {e}")))?;

    let threads = options
        .threads
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
        })
        .max(1);

    let mut record = csv::StringRecord::new();
    let mut row_number = 1u64; // Kopfzeile ist Zeile 1
    let mut rows = 0u64;
    let mut chunk: Vec<(u64, csv::StringRecord)> = Vec::with_capacity(CHUNK_ROWS);
    let mut done = false;

    while !done {
        // Einen Block einlesen …
        chunk.clear();
        while chunk.len() < CHUNK_ROWS {
            if !input
                .read_record(&mut record)
                .map_err(|e| Error::io(format!("Zeile {} nicht lesbar: {e}", row_number + 1)))?
            {
                done = true;
                break;
            }
            row_number += 1;
            chunk.push((row_number, record.clone()));
        }

        // … rechnen …
        let results = compute_chunk(pap, &columns, &chunk, threads);

        // … und in Eingabereihenfolge schreiben. Ein Fehler beendet den Lauf
        // an genau der Stelle, an der auch die sequenzielle Verarbeitung
        // abbräche: Alle vorherigen Zeilen sind geschrieben.
        for ((number, _), result) in chunk.iter().zip(results) {
            let fields = result?;
            output
                .write_record(&fields)
                .map_err(|e| Error::io(format!("Zeile {number} nicht schreibbar: {e}")))?;
            rows += 1;
        }
    }

    output
        .flush()
        .map_err(|e| Error::io(format!("Ausgabe nicht abschließbar: {e}")))?;
    Ok(rows)
}

/// Rechnet einen Block von Zeilen und liefert die Ergebnisse in Eingabe-
/// reihenfolge. Bei mehreren Threads bearbeitet jeder einen zusammenhängenden
/// Abschnitt, sodass die Reihenfolge ohne Sortieren erhalten bleibt.
fn compute_chunk(
    pap: &Pap,
    columns: &[Column],
    chunk: &[(u64, csv::StringRecord)],
    threads: usize,
) -> Vec<Result<Vec<String>, Error>> {
    if threads <= 1 || chunk.len() < MIN_ROWS_FOR_THREADS {
        return chunk
            .iter()
            .map(|(number, record)| compute_row(pap, columns, record, *number))
            .collect();
    }

    let per_thread = chunk.len().div_ceil(threads);
    std::thread::scope(|scope| {
        let handles: Vec<_> = chunk
            .chunks(per_thread)
            .map(|slice| {
                scope.spawn(move || {
                    slice
                        .iter()
                        .map(|(number, record)| compute_row(pap, columns, record, *number))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("Rechen-Thread ist abgestürzt"))
            .collect()
    })
}

/// Wertet eine einzelne Zeile aus: durchgereichte Spalten zuerst, dann die
/// Ausgabevariablen des PAP.
fn compute_row(
    pap: &Pap,
    columns: &[Column],
    record: &csv::StringRecord,
    row_number: u64,
) -> Result<Vec<String>, Error> {
    let mut inputs = pap.new_inputs();
    let mut fields = Vec::new();
    for (value, kind) in record.iter().zip(columns) {
        match kind {
            Column::Passthrough => fields.push(value.to_string()),
            // Leere Felder bedeuten "Default verwenden".
            Column::Input(_) if value.trim().is_empty() => {}
            Column::Input(id) => {
                inputs
                    .set_by_id(*id, value)
                    .map_err(|e| Error::eval(format!("Zeile {row_number}: {e}")))?;
            }
        }
    }

    let results = pap
        .run(&inputs)
        .map_err(|e| Error::eval(format!("Zeile {row_number}: {e}")))?;
    fields.extend(results.into_iter().map(|(_, value)| value));
    Ok(fields)
}

/// Ordnet jeder Spalte der Kopfzeile ihre Bedeutung zu.
fn resolve_columns(
    pap: &Pap,
    header: &csv::StringRecord,
    options: &CsvOptions,
) -> Result<Vec<Column>, Error> {
    let mut columns = Vec::with_capacity(header.len());
    for name in header {
        let name = name.trim();
        match pap.var_id(name) {
            Some(id) if pap.var(id).kind == VarKind::Input => columns.push(Column::Input(id)),
            // Der Name existiert im PAP, ist aber keine Eingabe. Das ist fast
            // immer ein Missverständnis und daher auch mit --passthrough ein
            // Fehler.
            Some(id) => {
                return Err(Error::eval(format!(
                    "Spalte `{name}` ist keine Eingabevariable, sondern {:?}",
                    pap.var(id).kind
                )));
            }
            None if options.passthrough => columns.push(Column::Passthrough),
            None => {
                return Err(Error::eval(format!(
                    "unbekannte Spalte `{name}` — mit --passthrough wird sie \
                     unverändert in die Ausgabe übernommen"
                )));
            }
        }
    }
    Ok(columns)
}

/// Kopfzeile mit allen Eingabevariablen als Vorlage für eigene Dateien.
pub fn csv_template(pap: &Pap, delimiter: u8) -> String {
    let names: Vec<&str> = pap
        .vars_of_kind(VarKind::Input)
        .map(|(_, decl)| decl.name.as_str())
        .collect();
    let defaults: Vec<String> = pap
        .vars_of_kind(VarKind::Input)
        .map(|(_, decl)| format_value(&decl.default))
        .collect();
    let sep = delimiter as char;
    format!(
        "{}\n{}\n",
        names.join(&sep.to_string()),
        defaults.join(&sep.to_string())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn pap() -> &'static Pap {
        static PAP: OnceLock<Pap> = OnceLock::new();
        PAP.get_or_init(|| {
            let xml = std::fs::read_to_string("tests/data/Lohnsteuer2025.xml").unwrap();
            Pap::from_xml(&xml).unwrap()
        })
    }

    fn run(csv: &str, options: &CsvOptions) -> Result<String, Error> {
        let mut out = Vec::new();
        run_csv(pap(), csv.as_bytes(), &mut out, options)?;
        Ok(String::from_utf8(out).unwrap())
    }

    /// Wert einer Spalte in der ersten Datenzeile.
    fn field(csv: &str, column: &str) -> String {
        let mut lines = csv.lines();
        let header: Vec<&str> = lines.next().unwrap().split(',').collect();
        let row: Vec<&str> = lines.next().unwrap().split(',').collect();
        let index = header.iter().position(|c| *c == column).unwrap();
        row[index].to_string()
    }

    #[test]
    fn computes_each_row() {
        let out = run(
            "LZZ,STKL,RE4,KVZ,PVZ\n1,1,5000000,2.5,1\n1,3,5000000,2.5,1\n",
            &CsvOptions::default(),
        )
        .unwrap();
        let mut lines = out.lines();
        let header = lines.next().unwrap();
        assert!(header.starts_with("BK,BKS,LSTLZZ"), "Kopfzeile: {header}");
        let rows: Vec<&str> = lines.collect();
        assert_eq!(rows.len(), 2);
        // Steuerklasse 1 bzw. 3 bei gleichem Bruttolohn.
        assert!(rows[0].contains("692700"), "{}", rows[0]);
        assert!(rows[1].contains("297000"), "{}", rows[1]);
    }

    #[test]
    fn omitted_and_empty_columns_use_defaults() {
        // RE4 fehlt komplett, KVZ ist leer.
        let out = run("LZZ,STKL,KVZ\n1,1,\n", &CsvOptions::default()).unwrap();
        assert_eq!(field(&out, "LSTLZZ"), "0");
    }

    #[test]
    fn passthrough_keeps_extra_columns_in_order() {
        let options = CsvOptions {
            passthrough: true,
            ..CsvOptions::default()
        };
        let out = run(
            "Personalnummer,LZZ,STKL,RE4,KVZ,PVZ,Name\n4711,1,1,5000000,2.5,1,Muster\n",
            &options,
        )
        .unwrap();
        let header = out.lines().next().unwrap();
        assert!(
            header.starts_with("Personalnummer,Name,BK"),
            "Kopfzeile: {header}"
        );
        assert_eq!(field(&out, "Personalnummer"), "4711");
        assert_eq!(field(&out, "Name"), "Muster");
        assert_eq!(field(&out, "LSTLZZ"), "692700");
    }

    #[test]
    fn unknown_column_is_an_error_without_passthrough() {
        let err = run(
            "LZZ,STKL,Personalnummer\n1,1,4711\n",
            &CsvOptions::default(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("--passthrough"),
            "Meldung sollte auf --passthrough hinweisen: {err}"
        );
    }

    #[test]
    fn output_column_as_input_is_rejected() {
        for passthrough in [false, true] {
            let options = CsvOptions {
                passthrough,
                ..CsvOptions::default()
            };
            let err = run("LZZ,LSTLZZ\n1,5\n", &options).unwrap_err();
            assert!(err.to_string().contains("keine Eingabevariable"), "{err}");
        }
    }

    #[test]
    fn reports_the_offending_row_number() {
        let err = run("LZZ,STKL\n1,1\n1,1\n1,keinezahl\n", &CsvOptions::default()).unwrap_err();
        assert!(err.to_string().contains("Zeile 4"), "Meldung: {err}");
    }

    #[test]
    fn semicolon_delimiter_and_quoting() {
        let options = CsvOptions {
            delimiter: b';',
            passthrough: true,
            ..CsvOptions::default()
        };
        let out = run(
            "Name;LZZ;STKL;RE4;KVZ;PVZ\n\"Muster; GmbH\";1;1;5000000;2.5;1\n",
            &options,
        )
        .unwrap();
        assert!(out.contains("\"Muster; GmbH\""), "Quoting fehlt: {out}");
        assert!(out.contains("692700"), "{out}");
    }

    #[test]
    fn header_only_input_yields_header_only_output() {
        let mut out = Vec::new();
        let rows = run_csv(
            pap(),
            "LZZ,STKL\n".as_bytes(),
            &mut out,
            &CsvOptions::default(),
        )
        .unwrap();
        assert_eq!(rows, 0);
        assert_eq!(String::from_utf8(out).unwrap().lines().count(), 1);
    }

    /// Erzeugt `rows` Fälle mit fortlaufender Nummer und wechselnder
    /// Steuerklasse — genug Zeilen, um die Parallelverarbeitung auszulösen.
    fn many_rows(rows: u64) -> String {
        let mut csv = String::from("Nr,LZZ,STKL,RE4,KVZ,PVZ\n");
        for i in 0..rows {
            let stkl = i % 6 + 1;
            let re4 = 1_000_000 + i * 1_000;
            csv.push_str(&format!("{i},1,{stkl},{re4},2.5,1\n"));
        }
        csv
    }

    fn options_with(threads: usize) -> CsvOptions {
        CsvOptions {
            passthrough: true,
            threads: Some(threads),
            ..CsvOptions::default()
        }
    }

    #[test]
    fn parallel_result_is_identical_to_sequential() {
        let csv = many_rows(1000);
        let sequential = run(&csv, &options_with(1)).unwrap();
        for threads in [2, 3, 8, 16] {
            assert_eq!(
                run(&csv, &options_with(threads)).unwrap(),
                sequential,
                "Ergebnis weicht bei {threads} Threads ab"
            );
        }
        // Auch der Standard (alle Kerne) muss dasselbe liefern.
        let options = CsvOptions {
            passthrough: true,
            ..CsvOptions::default()
        };
        assert_eq!(run(&csv, &options).unwrap(), sequential);
    }

    #[test]
    fn preserves_row_order_across_chunks() {
        // Mehr als CHUNK_ROWS, damit mehrere Blöcke durchlaufen.
        let rows = CHUNK_ROWS as u64 + 500;
        let out = run(&many_rows(rows), &options_with(8)).unwrap();
        let numbers: Vec<u64> = out
            .lines()
            .skip(1)
            .map(|line| line.split(',').next().unwrap().parse().unwrap())
            .collect();
        assert_eq!(numbers.len(), rows as usize);
        assert!(
            numbers.iter().enumerate().all(|(i, n)| *n == i as u64),
            "Zeilenreihenfolge ist nicht erhalten"
        );
    }

    #[test]
    fn parallel_reports_same_error_row_as_sequential() {
        let mut csv = many_rows(500);
        // Zeile 300 der Daten (Dateizeile 301) unbrauchbar machen.
        let mut lines: Vec<&str> = csv.lines().collect();
        lines[300] = "300,1,keinezahl,1000000,2.5,1";
        csv = format!("{}\n", lines.join("\n"));

        let sequential = run(&csv, &options_with(1)).unwrap_err().to_string();
        assert!(sequential.contains("Zeile 301"), "{sequential}");
        for threads in [2, 8] {
            let parallel = run(&csv, &options_with(threads)).unwrap_err().to_string();
            assert_eq!(parallel, sequential, "bei {threads} Threads");
        }
    }

    #[test]
    fn rows_before_an_error_are_written() {
        let mut lines: Vec<&str> = Vec::new();
        let csv = many_rows(200);
        lines.extend(csv.lines());
        lines[100] = "100,1,keinezahl,1000000,2.5,1";
        let csv = format!("{}\n", lines.join("\n"));

        let mut out = Vec::new();
        let error = run_csv(pap(), csv.as_bytes(), &mut out, &options_with(8)).unwrap_err();
        assert!(error.to_string().contains("Zeile 101"));
        // Kopfzeile plus die 99 fehlerfreien Zeilen davor.
        let written = String::from_utf8(out).unwrap();
        assert_eq!(
            written.lines().count(),
            100,
            "Teilausgabe: {}",
            written.lines().count()
        );
    }

    #[test]
    fn template_round_trips() {
        let template = csv_template(pap(), b',');
        let out = run(&template, &CsvOptions::default()).unwrap();
        assert_eq!(field(&out, "LSTLZZ"), "0");
    }
}
