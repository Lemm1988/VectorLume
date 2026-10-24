# VectorLume

Moderne Software zum Gestalten und Lasern für GRBL-Diodenlaser (zuerst für den **Sculpfun SF-A9 40 W**),
geschrieben in Rust. Marlin und Smoothieware werden experimentell unterstützt.

> Privates Projekt – noch keine Lizenz vergeben, alle Rechte vorbehalten.

## Funktionen

- **Gestalten:** Rechteck (mit Eckenradius), Ellipse, Linie, Vieleck, Stern, Text mit installierten Schriften
  (bearbeitbar, entlang eines Pfades, variabel mit Seriennummer `{n}`, `{datum}`, `{zeit}` und Datenliste aus CSV);
  Auswählen, Verschieben, Skalieren, Drehen, Spiegeln, Ausrichten, Verteilen, Gruppieren, Rückgängig/Wiederholen
- **Bearbeiten:** Knotenwerkzeug, Boolesche Operationen und Verschweißen, Versatz, Reihen-Kopien (Raster, Kreis,
  entlang Pfad), Bild nachzeichnen, Objektfang mit Ausrichtungslinien, Ebenen sperren
- **Import/Export:** SVG (inkl. Text), DXF (inkl. Blöcke, Bögen, Splines), PDF/AI, EPS (mit installiertem
  Ghostscript), HPGL/PLT,
  PNG/JPG/BMP/GIF/WEBP/TIFF; LightBurn-Projekte (`.lbrn2`) öffnen; Export als G-Code, SVG und DXF; eigenes
  Projektformat `.vlume`; Autosicherung
- **30 Farbebenen** mit den Modi *Linie* (Schnittbreite, Laschen), *Füllen* (Winkel, Kreuzschraffur, Overscan),
  *Versatz-Füllen* und *Bild* (8 Dithering-Verfahren oder Graustufen, Helligkeit/Kontrast/Gamma)
- **Material:** Materialtest-Generator und Materialbibliothek; Z-Versatz je Durchgang für Geräte mit Z-Achse
- **Kamera:** Webcam oder Foto der Arbeitsfläche, über vier Passpunkte entzerrt und halbtransparent unter dem
  Entwurf angezeigt (*Laser → Kamera…*)
- **Laser:** USB- oder Netzwerkverbindung (TCP/Telnet, z. B. FluidNC), Status, Konsole, Referenzfahrt, Verfahren, Rahmen, Start/Pause/Stopp,
  Job-Ursprung (absolut, Benutzer-Ursprung, aktuelle Position, 9 Ankerpunkte), Drehachse (Rollen/Futter, Y oder A)
- **Vorschau** mit Laufzeitschätzung, Leerfahrten und Abspielregler
- **Optimierung:** kürzeste Wege, innen vor außen, Gravieren vor Schneiden
- **Sicherheit:** Prüfung des Laser-Modus (`$32=1`), Not-Stopp bei Abbruch/Verbindungsverlust, Bereichsprüfung
- **Simulator:** alles ohne angeschlossenen Laser testen (Anschluss „Simulator“)

## Erste Schritte

1. Installieren mit `powershell -ExecutionPolicy Bypass -File tools\install.ps1` – danach über die
   Verknüpfung **VectorLume** auf dem Desktop bzw. im Startmenü starten (oder `cargo run --release`).
   `.vlume`-Projekte öffnen sich per Doppelklick; LightBurn-Projekte (`.lbrn2`) über *Öffnen mit → VectorLume*
   oder *Datei → Öffnen* (die Zuordnung zu LightBurn bleibt). Entfernen mit `… install.ps1 -Uninstall`.
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
`VECTORLUME_SIMULATOR=1` (mit Simulator verbinden), `VECTORLUME_CONFIG_DIR=<Ordner>` (eigener Ordner für
Einstellungen, Autosicherung und Materialbibliothek). Dateien können als Argument übergeben werden.

## Aufbau

| Ordner | Inhalt |
|---|---|
| `crates/vl-core` | Geometrie, Pfade, Ebenen, Dokument, Projektdatei |
| `crates/vl-raster` | Bildaufbereitung, Dithering, Nachzeichnen |
| `crates/vl-cam` | Werkzeugwege, Optimierung, Zeitschätzung, G-Code |
| `crates/vl-device` | Treiber für GRBL, Marlin und Smoothieware, Streaming, Simulator |
| `crates/vl-io` | Import (SVG, DXF, PDF/AI, EPS, HPGL, Bilder, Schrift, CSV, LightBurn-Projekte), SVG-/DXF-Export |
| `app` | Oberfläche (egui) |
| `docs` | [Lastenheft](docs/Lastenheft.md), [Pflichtenheft](docs/Pflichtenheft.md) mit Umsetzungsstand und Roadmap |
