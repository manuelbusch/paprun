# paprun

Ein Interpreter für die vom Bundesministerium der Finanzen (BMF) veröffentlichten
**Programmablaufpläne (PAP)** zur Lohnsteuerberechnung — geschrieben in Rust.

Das BMF stellt den PAP nicht nur als PDF-Flussdiagramm bereit, sondern auch als
maschinenlesbaren **XML-Pseudocode** (Java-Subset). `paprun` lädt eine solche
XML-Datei, interpretiert sie und berechnet daraus Lohnsteuer, Solidaritätszuschlag
und Bemessungsgrundlage für die Kirchensteuer — centgenau mit den Rundungsregeln
von Java `BigDecimal`.

Ein neues Steuerjahr benötigt keinen neuen Code: Es genügt, die neue XML-Datei des
BMF zu laden.

## Bezugsquelle der PAP-XML-Dateien

Offizielle Downloads des BMF (Übersichtsseite:
<https://www.bmf-steuerrechner.de/interface/pseudocodes.xhtml>):

- <https://www.bmf-steuerrechner.de/javax.faces.resource/daten/xmls/Lohnsteuer2026.xml.xhtml>
- <https://www.bmf-steuerrechner.de/javax.faces.resource/daten/xmls/Lohnsteuer2025.xml.xhtml>

Kopien liegen als Test-Fixtures unter `tests/data/` (Quelle und Abrufdatum siehe
`tests/data/README.md`).

## Nutzung

```
paprun <PAP.xml> --in NAME=WERT [--in NAME=WERT …] [--json] [--vars]
```

- `--in NAME=WERT` — Eingabevariable setzen (wiederholbar). Geldbeträge sind nach
  PAP-Konvention in **Cent** anzugeben, z. B. `--in RE4=5000000` für 50 000,00 €.
- `--vars` — alle Ein- und Ausgabevariablen mit Typ und Default auflisten.
- `--all` — alle Variablen ausgeben, auch interne Zwischenergebnisse.
- `--json` — Ausgaben als flaches JSON-Objekt (Werte als Strings).

Beispiel (Jahreslohn 50 000 €, Steuerklasse 1, Jahres-Lohnzahlungszeitraum):

```
paprun tests/data/Lohnsteuer2025.xml \
  --in LZZ=1 --in STKL=1 --in RE4=5000000 --in KVZ=2.5 --in PVZ=1
```

Ausgegeben wird eine `NAME=WERT`-Zeile pro Ausgabevariable (u. a. `LSTLZZ` =
Lohnsteuer für den Lohnzahlungszeitraum in Cent) — hier `LSTLZZ=692700`, also
6 927,00 € Jahreslohnsteuer.

## Stapelverarbeitung (CSV über Standardein-/ausgabe)

Für viele Fälle liest `--csv` eine CSV-Tabelle von der Standardeingabe und
schreibt die Ergebnisse als CSV auf die Standardausgabe. Die Kopfzeile benennt
die Eingabevariablen, jede Datenzeile ist ein Berechnungsfall:

```
paprun tests/data/Lohnsteuer2025.xml --csv --passthrough < faelle.csv > ergebnisse.csv
```

```csv
Personalnummer,LZZ,STKL,RE4,KVZ,PVZ      →   Personalnummer,BK,BKS,LSTLZZ,SOLZLZZ,…
4711,1,1,5000000,2.5,1                       4711,0,0,692700,0,…
4712,1,3,5000000,2.5,1                       4712,0,0,297000,0,…
```

- Die Verarbeitung ist **streamend**: Es wird immer nur eine Zeile im Speicher
  gehalten, der Bedarf bleibt also auch bei Millionen Zeilen konstant.
- **Nicht aufgeführte oder leere Felder** verwenden den Default der Variablen.
- `--passthrough` übernimmt Spalten, die keine Eingabevariablen sind (etwa
  Personalnummern), unverändert in die Ausgabe. Ohne das Flag sind sie ein
  Fehler — das fängt Tippfehler in Spaltennamen ab, die sonst still ignoriert
  würden.
- `--delimiter ';'` für Exporte aus deutschen Excel-Versionen.
- `--template` gibt eine Kopfzeile mit allen Eingabevariablen als Vorlage aus.
- Bei einem Fehler bricht die Verarbeitung mit der Zeilennummer ab, sodass
  Ein- und Ausgabe immer Zeile für Zeile zusammenpassen.

Durchsatz: rund **42 000 Zeilen/s** (200 000 Zeilen in 4,7 s), davon etwa 82 %
Rechenzeit und 18 % CSV-Verarbeitung.

## Laufzeit

Eine Berechnung dauert rund 19 µs (≈ 51 000/s auf einem Kern), das Laden eines PAP
etwa 1,4 ms. Ein geladener `Pap` ist `Send + Sync` und kann von mehreren Threads
geteilt werden — auf sechs Kernen sind es ~243 000 Berechnungen/s. Für Massenläufe
lohnt es sich also, den PAP einmal zu laden und die Fälle zu parallelisieren.

```
cargo run --release --example bench    # Gesamtdurchsatz
cargo run --release --example micro    # einzelne Rechenbausteine
```

## Stand

Die Jahrgänge 2025 und 2026 laden vollständig und rechnen. Getestet ist der
Grundtarif über alle sechs Steuerklassen; Sonderkonstellationen (sonstige Bezüge,
Versorgungsbezüge, private Krankenversicherung) sind implementiert, aber noch nicht
gezielt geprüft — siehe [docs/PLAN.md](docs/PLAN.md).

## Vorgehen und Architektur

Das geplante Vorgehen, die Architekturentscheidungen (Interpreter statt
Codegenerator, `bigdecimal`-Crate, exakte Nachbildung von Java
`divide(divisor, scale, roundingMode)`) und die Teststrategie sind in
[docs/PLAN.md](docs/PLAN.md) dokumentiert und werden dort fortgeschrieben.

## Tests

```
cargo test
```

Die Tests umfassen die Rundungssemantik (`value.rs`), den Expression-Parser und
das vollständige Laden beider PAP-Jahrgänge. Kernstück ist ein End-to-End-Test,
der den Einkommensteuertarif nach §32a EStG ein zweites Mal — unabhängig vom
Interpreter — implementiert und über 2 000 Fälle (beide Jahrgänge, alle sechs
Steuerklassen, Einkommen von 0 bis 400 000 € samt aller Zonengrenzen) abgleicht.
Details in [docs/PLAN.md](docs/PLAN.md).

## Hinweis

`paprun` ist ein technisches Werkzeug zur Auswertung der amtlichen
Programmablaufpläne und keine steuerliche Beratung. Maßgeblich sind die
Veröffentlichungen des BMF.
