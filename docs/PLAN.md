# paprun — Interpreter für BMF-Programmablaufpläne (Lohnsteuer-PAP) in Rust

## Kontext

Das BMF veröffentlicht den Programmablaufplan (PAP) für die Lohnsteuer maschinenlesbar als XML-Pseudocode (verifiziert: `https://www.bmf-steuerrechner.de/javax.faces.resource/daten/xmls/Lohnsteuer2026.xml.xhtml`, analog 2025). Struktur: `<PAP>` → `<VARIABLES>` (INPUTS/OUTPUTS/INTERNALS), `<CONSTANTS>`, `<METHODS>` mit `<MAIN>`, `<METHOD>`; Statements `<EVAL exec="…">`, `<IF expr="…">`/`<THEN>`/`<ELSE>`, `<EXECUTE method="…"/>`. Ausdrücke sind ein Java-Subset (BigDecimal-Methodenaufrufe, int-Arithmetik, Vergleiche, `&&`/`||`, Array-Indexierung). Keine Schleifen, keine Rekursion, keine Parameter — alle Variablen global.

Ziel: **Interpreter** (kein Codegenerator) — ein neues Steuerjahr ist nur eine neue XML-Datei. Library-Crate + dünne CLI. Ergebnisse müssen centgenau mit Java-`BigDecimal`-Semantik übereinstimmen.

Das Repo ist ein frisches `cargo new` (edition 2024, keine Dependencies).

## Architektur-Entscheidungen

- **Decimal-Crate: `bigdecimal`** (nicht `rust_decimal`): arbitrary precision wie Java, exakte add/subtract/multiply mit Java-Scale-Regeln, `RoundingMode` nach Java-Vorbild. `rust_decimal` (96-bit Mantisse) rundet bei großen Zwischenprodukten still — Korrektheitsrisiko.
- **`divide(d, scale, mode)` selbst implementieren** (`div_scale`): über `as_bigint_and_exponent()` + BigInt-`div_rem`, Rundung aus dem Rest ableiten. Exakte Integer-Arithmetik, bit-identisch zu Java. Nicht die `Div`-Impl von `bigdecimal` nutzen (Default-Precision + Nachrunden ≠ Java).
- **XML: `roxmltree`** (read-only DOM, überspringt Kommentare). CLI ohne clap (nur wenige Flags).
- **Auflösung zur Ladezeit**: Variablen/Methoden → Indizes (`VarId`/`MethodId`); unbekannte Namen, Methoden oder Syntax → Ladefehler mit exec-String, nie Laufzeitüberraschung. `BdMethod` als geschlossenes Enum (Add, Subtract, Multiply, Divide, SetScale, CompareTo, LongValue, …).
- **`double`-Variablen** (z. B. Faktor `f`): eigene `Dbl(f64)`-Variante mit Java-Promotion-Regeln; `BigDecimal.valueOf(f64)` über shortest-roundtrip-Formatierung (entspricht `Double.toString`).
- Dezimal-Literale (`0.07`) direkt aus dem Quelltext als BigDecimal parsen — entspricht Javas `BigDecimal.valueOf(double)` für alle im PAP vorkommenden Literale.

## Modul-Layout

```
src/
├── lib.rs      # Public API: Pap::from_str, pap.inputs(), pap.run()
├── error.rs    # LoadError (mit exec-String-Kontext) / EvalError
├── value.rs    # Value {Int(i64), Dec(BigDecimal), Dbl(f64), Bool, Arr(Rc<Vec>)},
│               # RoundMode (Up/Down/Ceiling/Floor/HalfUp/HalfDown/HalfEven),
│               # div_scale(), set_scale(), long_value(), value_of()
├── lex.rs      # Tokenizer (IDENT, NUMBER, Operatoren, `new`)
├── parse.rs    # Rekursiver Abstieg: || > && > ==/!= > </<= > +/- > */÷/% > unary > postfix
│               # postfix: .methode(...), [index]; BigDecimal.ZERO/ROUND_* zur Parse-Zeit auflösen
│               # Zuweisungen ("X= expr") und Array-Literale ("{...}") für CONSTANTS
├── load.rs     # roxmltree → Pap: Deklarationen (defaults constant-folden), Konstanten,
│               # MAIN + Methoden; OUTPUTS ggf. in Untergruppen (STANDARD/DBA) → über Nachfahren iterieren
├── eval.rs     # Env = Vec<Value> (per VarId indiziert); sequentielle Statement-Ausführung
└── main.rs     # CLI
tests/
├── parse_full.rs   # beide Fixture-XMLs laden: jeder exec/expr/default/value parst & resolved
├── e2e.rs          # Smoke- + Golden-Tests
└── data/           # Lohnsteuer2025.xml, Lohnsteuer2026.xml (mit Quell-URL+Datum), pruef_2025.csv
```

Dependencies: `bigdecimal`, `num-bigint`, `num-traits`, `roxmltree`.

## CLI

```
paprun <PAP.xml> --in NAME=WERT [--in …] [--json] [--vars]
```
- Ausgabe: eine `NAME=WERT`-Zeile pro OUTPUT (Deklarationsreihenfolge), nie wissenschaftliche Notation; `--json` als flaches Objekt (Werte als Strings); `--vars` listet Inputs/Outputs mit Typ und Default.
- Beträge in **Cent** (PAP-Konvention) dokumentieren. Exit-Codes: 0 ok, 1 Fehler, 2 Usage.

## Implementierungsreihenfolge

- [x] **0. Dokumentation im Repo**: `docs/PLAN.md` und `README.md` angelegt.
- [x] **1. `value.rs`** + Cargo.toml — `div_scale`/`set_scale`/`long_value` mit voller Unit-Test-Batterie (riskantester Code zuerst, isoliert): Vorzeichen-/Tie-Fälle wie `div_scale(-7, 2, 0, Down) = -3` vs. `Floor = -4`, `set_scale("2.005", 2, HalfUp) = 2.01`, `set_scale(-7.5, 0, HalfUp) = -8`.
- [x] **2. `lex.rs` + `ast.rs` + `parse.rs`** — Unit-Tests mit Original-Snippets (inkl. `ZRE4J= RE4.divide (ZAHL100, 2, BigDecimal.ROUND_DOWN)` mit Leerzeichen-Eigenheiten).
- [x] **3. `load.rs`** + beide XMLs als Fixtures → `parse_full.rs` grün.
- [x] **4. `eval.rs`** → Smoke-Test: `RE4=0` → `LSTLZZ=0`.
- [x] **5. CLI** → `--in`/`--vars`/`--json`/`--all`, Golden-Tests festgeschrieben.
- [x] **6. Verifikation** → unabhängige Tarif-Referenz statt Prüftabellen-CSV (siehe unten).
- [x] **7. Politur** → `cargo fmt`, Fehlermeldungen mit exec-Kontext, lib-Doku.

### Abweichungen vom ursprünglichen Plan

| Geplant | Tatsächlich | Grund |
|---|---|---|
| Cross-Check gegen LstGen (Python) | Entfällt | Auf diesem System sind weder `pip` noch `ensurepip` verfügbar; LstGen ließ sich nicht installieren. |
| Golden-Wert manuell über den BMF-Online-Rechner | Entfällt als Primärquelle | Die externe BMF-Testschnittstelle liegt hinter einer Zustimmungsseite; ihre Nutzungsbedingungen wurden nicht umgangen. |
| Prüftabellen-CSV mit ~40 transkribierten Zeilen | Ersetzt durch **unabhängige Referenzimplementierung** in `tests/e2e.rs` | Deutlich stärker: prüft >2.000 Fälle statt 40, ohne Transkriptionsrisiko. |
| `Rc<Vec<Value>>` für Konstantentabellen | `Arc<Vec<Value>>` | Macht `Pap` `Send + Sync`, sodass ein geladener PAP zwischen Threads geteilt werden kann. |
| — (nicht geplant) | CLI-Flag `--all` ergänzt | Nötig, um interne Zwischenergebnisse (`ZVE`, `X`, `ST`) für die Verifikation auszulesen; auch beim Debuggen nützlich. |

## Risiken & Gegenmaßnahmen (Stand nach Umsetzung)

1. **Exakte `divide`-Semantik** → BigInt-basiertes `div_scale` ohne Precision-Knopf, dazu Tie-/Negativ-Tests. Zusätzlich durch die Tarif-Referenz über >2.000 Fälle bestätigt. *Risiko ausgeräumt.*
2. **Grammatik-Drift zwischen Jahrgängen** → fail-loud beim Laden; beide Jahrgänge parsen vollständig. Ältere PAPs (vor 2025) sind ungetestet und können zusätzliche Konstrukte enthalten (Casts, `Math.*`); diese schlagen dann sichtbar beim Laden fehl, statt still falsch zu rechnen. *Restrisiko bewusst offen.*
3. **`double`-Pfad** → `Dbl`-Variante mit Java-Promotionsregeln; in beiden Jahrgängen betrifft das nur den Faktor `f`, dessen Ergebnis sofort durch ein explizites `setScale` läuft. *Risiko gering.*
4. **Verifikationsquelle** → statt transkribierter Prüftabellen eine unabhängig implementierte Tarifreferenz (siehe unten). *Transkriptionsrisiko entfällt.*

## Verifikation

Der Kern ist der Test `tariff_matches_independent_reference` in `tests/e2e.rs`: Der
Einkommensteuertarif nach §32a EStG **und** das Sonderverfahren der Steuerklassen 5/6
sind dort ein zweites Mal implementiert — direkt aus den Gesetzes- bzw. PAP-Formeln,
ohne den Interpreter zu benutzen. Verglichen werden beide Wege über:

- die Jahrgänge 2025 und 2026,
- alle sechs Steuerklassen,
- Jahresbruttolöhne von 0 bis 400.000 € im 2.500-€-Raster, plus alle Zonengrenzen
  (Grundfreibetrag, Zonenübergänge, W1/W2/W3 des Sonderverfahrens).

Das sind über 2.000 Vergleiche pro Lauf, die die gesamte BigDecimal-Kette abdecken:
Rundungsmodi, Scale-Semantik von `divide`/`setScale`, Verzweigungslogik und
Methodenaufrufe. Dieser Test hat sich bereits bewährt — er deckte auf, dass sich
`UP5_6` zwischen 2025 und 2026 unterscheidet (Abschneiden auf 2 statt 0 Nachkommastellen).

Ergänzend prüfen strukturelle Tests Monotonie der Steuer, die Ordnung der
Steuerklassen (3 < 1 < 5 ≤ 6), Steuerfreiheit unterhalb des Grundfreibetrags sowie
die Konsistenz von Jahres- und Monatsberechnung. `known_values_stay_stable` sichert
konkrete Werte gegen Regressionen ab.

Die amtlichen Tarifkonstanten wurden gegen §32a EStG abgeglichen (2025:
Grundfreibetrag 12.096 €, Zonengrenzen 17.444/68.481/277.826, Faktoren
932,30/176,64/0,42/0,45 — alle deckungsgleich).

**Nicht genutzte Quellen:** Die externe BMF-Programmierschnittstelle
(`einganginterface.xhtml`) liegt hinter einer Zustimmungsseite und ist ausdrücklich
nur für Programmtests freigegeben; sie wurde nicht automatisiert angesprochen. Ein
manueller Abgleich einzelner Fälle mit dem BMF-Online-Rechner bleibt als zusätzliche
Bestätigung sinnvoll — insbesondere für Konstellationen, die die Tarifreferenz nicht
abdeckt (Versorgungsbezüge, sonstige Bezüge, private Krankenversicherung).

## Laufzeit

Messung mit `cargo run --release --example bench` (Lohnsteuer2025, Jahreslauf,
Steuerklasse 1). `examples/micro.rs` misst die einzelnen Bausteine.

| | vorher | nachher |
|---|---|---|
| Laden eines PAP | ~1,4 ms | ~1,4 ms |
| Eine Berechnung | 25,7 µs | **19,4 µs** |
| Durchsatz (1 Kern) | 39.000/s | **51.400/s** |
| Durchsatz (6 Threads) | — | **242.700/s** |
| Allokationen pro Lauf | 106 | **66** |

Ein Lauf führt rund 124 Anweisungen, 1.138 Ausdrucksknoten und 80
BigDecimal-Methodenaufrufe aus.

### Was gemessen wurde

`perf` ist auf dem Entwicklungsrechner gesperrt, daher wurde über Mikro-Benchmarks
der Bausteine, einen zählenden `GlobalAlloc` und temporäre Operationszähler
eingegrenzt. Ergebnis: Weder Allokationen (~12 % der Zeit) noch die Env-Initialisierung
(~3 %) dominieren — der Aufwand verteilt sich auf die schiere Zahl ausgewerteter
Ausdrucksknoten.

### Umgesetzte Optimierungen

1. **`set_scale` ohne Division** — skaliert die Mantisse direkt (Zehnerpotenz statt
   Division durch 1): 124 → 43 ns. Getestet wurde auch `bigdecimal::with_scale_round`;
   es ist mit 140 ns langsamer, weil es intern in Dezimalziffern konvertiert.
2. **`div_scale` ohne Klone** — `as_bigint_and_scale()` (liefert `Cow`) statt
   `as_bigint_and_exponent()` (klont), `div_rem` statt getrennter Division und
   Modulo, `magnitude()` statt `abs()` für den allokationsfreien Betragsvergleich,
   plus ein Cache vorberechneter Zehnerpotenzen: 136 → 55 ns.
3. **Konstantenfaltung beim Laden** (`fold_constants` in `load.rs`) — der PAP ist
   voll von Ausdrücken wie `BigDecimal.valueOf(17444)`, die sonst bei jedem Lauf neu
   aufgebaut würden. Größter Einzelgewinn: 23,2 → 20,2 µs und 40 Allokationen weniger.
4. **LTO + `codegen-units = 1`** im Release-Profil.

Alle vier sind semantikerhaltend; der Tarif-Vergleich über >2.000 Fälle lief nach
jedem Schritt unverändert durch.

### Bewusst nicht umgesetzt

- **Eigene Festkomma-Darstellung für kleine Beträge** statt `BigDecimal`. Das wäre
  der größte verbleibende Hebel, verlagert aber die Rundungs- und Scale-Semantik in
  selbstgeschriebenen Code — bei einem Steuerberechner ein schlechter Tausch gegen
  Faktor 2.
- **Auswertung über Referenzen/`Cow` statt `Value`-Rückgaben**: tiefer Umbau des
  Interpreters für geschätzt 10–15 %.
- **Wiederverwendbarer Env-Puffer**: ~3 % für zusätzliche API-Komplexität.

Für Massenläufe ist Parallelisierung der weitaus größere Hebel: `Pap` ist
`Send + Sync`, ein geladener PAP kann von beliebig vielen Threads geteilt werden
(4,7× auf 6 Kernen).

## Stapelverarbeitung

`--csv` liest eine CSV-Tabelle von der Standardeingabe und schreibt die
Ergebnisse als CSV auf die Standardausgabe (`src/batch.rs`). Die Kernfunktion
`run_csv` arbeitet auf `Read`/`Write` und ist damit ohne Dateisystem testbar;
die CLI reicht nur Stdin/Stdout hinein.

Entwurfsentscheidungen:

- **Streamend statt sammelnd** — immer nur eine Zeile im Speicher, konstanter
  Bedarf auch bei Millionen Zeilen. Ein zunächst angedachter Modus mit je einer
  JSON-Datei pro Fall wurde verworfen: Er skaliert schlecht (ein Dateisystem-
  Zugriff pro Fall) und passt nicht in Pipelines.
- **Spaltenauflösung einmal pro Datei**, nicht pro Zeile — dafür gibt es
  `InputBuilder::set_by_id` neben dem namensbasierten `set`.
- **Unbekannte Spalten sind ein Fehler**, außer mit `--passthrough`. Ein
  stillschweigend ignoriertes `STKl` statt `STKL` würde sonst falsche Ergebnisse
  liefern, ohne aufzufallen.
- **Abbruch mit Zeilennummer** statt Überspringen fehlerhafter Zeilen, damit
  Ein- und Ausgabe immer 1:1 zusammenpassen.
- Das `csv`-Crate übernimmt Quoting und Trennzeichen — selbstgeschriebenes
  CSV-Parsing ist eine klassische Fehlerquelle (Anführungszeichen, eingebettete
  Zeilenumbrüche).

### Durchsatz und die Frage nach Apache Arrow

200 000 Zeilen laufen in 4,74 s durch (≈ 42 200 Zeilen/s, 23,7 µs pro Zeile).
Die reine Berechnung kostet davon 19,4 µs — **CSV-Ein-/Ausgabe macht nur rund
18 % der Laufzeit aus, die Interpretation 82 %.**

Damit beantwortet sich die Arrow-Frage von selbst: Selbst ein hypothetisch
kostenloses Einlesen brächte höchstens 22 % — Arrow kann an den 82 % Rechenzeit
nichts ändern. Dem stünde ein erheblicher Zuwachs an Abhängigkeiten gegenüber
(`arrow-rs` zieht Dutzende Crates nach; das Projekt hat derzeit fünf).

Arrow lohnt sich hier erst, wenn ein **Integrations**bedarf besteht, nicht aus
Geschwindigkeitsgründen: wenn die Fälle ohnehin als Parquet oder Arrow-IPC aus
einer Datenpipeline (Polars, DuckDB, Spark) kommen und das Hin- und
Herkonvertieren über CSV entfällt. Dann wäre ein optionales Cargo-Feature der
richtige Weg, damit die schlanke Standardvariante erhalten bleibt.

### Parallelisierung (umgesetzt)

Weil die Berechnung 82 % der Laufzeit ausmacht und `Pap` `Send + Sync` ist, war
Parallelisierung der größte Hebel. Umgesetzt blockweise mit `std::thread::scope`,
ohne zusätzliche Abhängigkeit:

- Es werden `CHUNK_ROWS` (4096) Zeilen eingelesen, auf die Threads verteilt und
  danach **in Eingabereihenfolge** geschrieben. Jeder Thread bekommt einen
  zusammenhängenden Abschnitt, sodass die Reihenfolge ohne Sortieren erhalten
  bleibt.
- Der Speicherbedarf bleibt konstant (ein Block, wenige MB).
- Unter 64 Zeilen pro Block wird sequenziell gerechnet — darunter überwiegt der
  Aufwand fürs Starten der Threads.
- **Fehlerverhalten ist identisch zum sequenziellen Lauf:** Die Zeilen vor der
  fehlerhaften werden geschrieben, dann bricht der Lauf mit derselben
  Zeilennummer ab. Tests vergleichen Ausgabe und Fehlermeldung zwischen 1, 2, 3,
  8 und 16 Threads.

| Threads | Zeilen/s | Dauer für 200 000 Zeilen | Beschleunigung |
|---|---|---|---|
| 1 | 42 400 | 4,71 s | 1,0× |
| 2 | 75 100 | 2,66 s | 1,8× |
| 4 | 131 000 | 1,53 s | 3,1× |
| 6 | 159 500 | 1,25 s | **3,8×** |

Die Skalierung flacht ab, weil Lesen und Schreiben im Hauptthread seriell
bleiben (Amdahl). Bei sechs Kernen entfallen von 6,3 µs pro Zeile nur noch rund
3,2 µs auf die Berechnung — **der CSV-Durchsatz ist jetzt der Flaschenhals.**
Damit gewinnt die Arrow-Frage an Gewicht: Ein spaltenbasiertes Eingabeformat
würde nun an der Hälfte der Wandzeit ansetzen statt an 18 %. Vor Arrow lägen
allerdings billigere Schritte: das Einlesen in einen eigenen Thread verlagern
(Lesen, Rechnen und Schreiben überlappen) oder die Ausgabe direkt aus den
Threads puffern.

## Nächste sinnvolle Schritte

- Ältere Jahrgänge (2024 und früher) laden und den Parser bei Bedarf erweitern.
- Fälle jenseits des Grundtarifs prüfen: sonstige Bezüge (`SONSTB`), Versorgungsbezüge
  (`VBEZ`), private Krankenversicherung (`PKV`), Kinderfreibeträge (`ZKF`).
- Solidaritätszuschlag und Kirchensteuer-Bemessungsgrundlage gezielt testen (die
  bisherigen Testfälle liegen unter der SolZ-Freigrenze, `SOLZLZZ` ist dort immer 0).
