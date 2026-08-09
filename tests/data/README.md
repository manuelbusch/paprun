# Test-Fixtures

## PAP-XML-Dateien

Amtliche XML-Pseudocodes der Programmablaufpläne, unverändert übernommen:

| Datei | Quelle | Abgerufen |
|---|---|---|
| `Lohnsteuer2025.xml` | <https://www.bmf-steuerrechner.de/javax.faces.resource/daten/xmls/Lohnsteuer2025.xml.xhtml> | 2026-08-09 |
| `Lohnsteuer2026.xml` | <https://www.bmf-steuerrechner.de/javax.faces.resource/daten/xmls/Lohnsteuer2026.xml.xhtml> | 2026-08-09 |

Übersichtsseite: <https://www.bmf-steuerrechner.de/interface/pseudocodes.xhtml>

## Referenzwerte

`pruef_2025.csv` enthält Referenzfälle für den End-to-End-Vergleich
(Format: `RE4;STKL;LZZ;weitere Inputs…;erwartetes LSTLZZ;erwartetes SOLZLZZ`).
Herkunft der Werte ist in der Datei bzw. in `tests/e2e.rs` dokumentiert.
