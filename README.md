# VectorLume

Moderne Software zum Gestalten und Lasern für GRBL-Diodenlaser (zuerst für den **Sculpfun SF-A9 40 W**),
geschrieben in Rust.

> Privates Projekt – noch keine Lizenz vergeben, alle Rechte vorbehalten.

## Funktionen

- **Gestalten:** Rechteck (mit Eckenradius), Ellipse, Linie, Vieleck, Stern, Text mit installierten Schriften
  (bearbeitbar, entlang eines Pfades, variabel mit Seriennummer `{n}`, `{datum}`, `{zeit}`);
  Auswählen, Verschieben, Skalieren, Drehen, Spiegeln, Ausrichten, Verteilen, Gruppieren, Rückgängig/Wiederholen
- **Bearbeiten:** Knotenwerkzeug, Boolesche Operationen und Verschweißen, Versatz, Reihen-Kopien (Raster, Kreis,
  entlang Pfad), Bild nachzeichnen, Objektfang mit Ausrichtungslinien, Ebenen sperren
- **Import/Export:** SVG (inkl. Text), DXF (inkl. Blöcke, Bögen, Splines), PDF/AI, HPGL/PLT,
  PNG/JPG/BMP/GIF/WEBP/TIFF; Export als G-Code, SVG und DXF; eigenes Projektformat `.vlume`; Autosicherung
- **30 Farbebenen** mit den Modi *Linie* (Schnittbreite, Laschen), *Füllen* (Winkel, Kreuzschraffur, Overscan),
  *Versatz-Füllen* und *Bild* (8 Dithering-Verfahren oder Graustufen, Helligkeit/Kontrast/Gamma)
- **Material:** Materialtest-Generator und Materialbibliothek; Z-Versatz je Durchgang für Geräte mit Z-Achse
- **Laser:** USB-Verbindung, Status, Konsole, Referenzfahrt, Verfahren, Rahmen, Start/Pause/Stopp,
  Job-Ursprung (absolut, Benutzer-Ursprung, aktuelle Position, 9 Ankerpunkte)
- **Vorschau** mit Laufzeitschätzung, Leerfahrten und Abspielregler
- **Optimierung:** kürzeste Wege, innen vor außen, Gravieren vor Schneiden
- **Sicherheit:** Prüfung des Laser-Modus (`$32=1`), Not-Stopp bei Abbruch/Verbindungsverlust, Bereichsprüfung
- **Simulator:** alles ohne angeschlossenen Laser testen (Anschluss „Simulator“)

## Erste Schritte

1. Installieren mit `powershell -ExecutionPolicy Bypass -File tools\install.ps1` – danach über die
   Verknüpfung **VectorLume** auf dem Desktop bzw. im Startmenü starten (oder `cargo run --release`).
2. *Hilfe → Beispielprojekt laden* zeigt alle Ebenenarten.
3. Rechts unter **Laser** den Anschluss wählen (USB-Port des SF-A9 oder „Simulator“) und **Verbinden**.
4. *Laser → Maschineneinstellungen → Werte vom Gerät* übernimmt Arbeitsfläche und S-Max aus der Steuerung.
5. **Vorschau** (Alt+P) prüfen, **Rahmen** fahren, dann **Job starten**.

> **Sicherheit:** Laserschutzbrille tragen, nie unbeaufsichtigt lasern, neue Einstellungen zuerst mit geringer Leistung testen.

## Entwickeln

```
cargo run -p vectorlume          # Programm starten
cargo test --workspace           # alle Tests
cargo clippy --workspace --all-targets
cargo build --release            # target/release/vectorlume.exe
```

Hilfreiche Startschalter: `VECTORLUME_DEMO=1` (Beispielprojekt), `VECTORLUME_PREVIEW=1` (Vorschau an),
`VECTORLUME_SIMULATOR=1` (mit Simulator verbinden). Dateien können als Argument übergeben werden.

## Aufbau

| Ordner | Inhalt |
|---|---|
| `crates/vl-core` | Geometrie, Pfade, Ebenen, Dokument, Projektdatei |
| `crates/vl-raster` | Bildaufbereitung, Dithering, Nachzeichnen |
| `crates/vl-cam` | Werkzeugwege, Optimierung, Zeitschätzung, G-Code |
| `crates/vl-device` | GRBL-Treiber, Streaming, Simulator |
| `crates/vl-io` | Import (SVG, DXF, PDF/AI, HPGL, Bilder, Schrift), SVG-/DXF-Export |
| `app` | Oberfläche (egui) |
| `docs` | [Lastenheft](docs/Lastenheft.md), [Pflichtenheft](docs/Pflichtenheft.md) mit Umsetzungsstand und Roadmap |
