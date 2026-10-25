# Pflichtenheft – VectorLume

| | |
|---|---|
| Bezug | [Lastenheft](Lastenheft.md) vom 24.10.2026 |
| Stand | 25.10.2026, Version 0.7.0 (Phasen 1–3 abgeschlossen bis auf Ruida/Galvo, Phase 4 in Arbeit) |
| Status | privat |

Legende: ✅ umgesetzt und getestet · 🟡 teilweise · ⏳ geplant (spätere Phase)

## 1. Zielbestimmung

### 1.1 Musskriterien (Phase 1, erste nutzbare Version)
Alle Anforderungen mit Priorität **M** aus dem Lastenheft: Projektverwaltung, Import, Zeichnen und Bearbeiten,
Farbebenen mit Linie/Füllen/Bild, GRBL-Steuerung mit Streaming, Rahmen, Job-Ursprung, Vorschau mit Laufzeit,
Wegoptimierung, Bereichsprüfung, Simulator.

### 1.2 Wunschkriterien (Phasen 2–3)
Alle Anforderungen mit Priorität **S**: Knotenbearbeitung, Boolesche Operationen, Offset, Arrays, Bild nachzeichnen,
Materialbibliothek, weitere Steuerungen (Ruida, Marlin, Smoothieware, Galvo), Kamera, Rotary.

### 1.3 Abgrenzung
- Kein Nachbau von Code, Grafiken, Texten oder Markenzeichen fremder Programme (siehe Lastenheft Kap. 3).
- Keine Cloud-Funktionen, keine Telemetrie.
- Keine Steuerung der Laser-Hardware ohne die Sicherheitsmechanismen aus Kapitel 7.

## 2. Produkteinsatz und Umgebung

| Bereich | Festlegung |
|---|---|
| Betriebssystem | Windows 10/11 (64 bit); Linux/macOS technisch möglich (plattformunabhängige Bibliotheken) |
| Grafik | wgpu, unter Windows standardmäßig DirectX 12 (Umgebungsvariable `WGPU_BACKEND` überschreibt) |
| Steuerungen | GRBL 1.1 (z. B. Sculpfun SF-A9), USB-seriell, 115 200 Baud |
| Entwicklung | Rust (Edition 2024, Toolchain ≥ 1.88), Cargo-Workspace, Git/GitHub (privat) |

## 3. Architektur

```
                         ┌───────────────────────────────┐
                         │  app (vectorlume.exe)          │  Oberfläche: egui/eframe
                         │  Arbeitsfläche, Panels, Dialoge│
                         └──┬─────────┬─────────┬────────┘
                            │         │         │
             ┌──────────────▼──┐ ┌────▼─────┐ ┌─▼──────────────┐
             │ vl-io           │ │ vl-cam   │ │ vl-device       │
             │ SVG, DXF, Bilder│ │ Wege,    │ │ GRBL-Protokoll, │
             │ Schrift, Export │ │ G-Code,  │ │ Streaming,      │
             └───────┬─────────┘ │ Zeit     │ │ Simulator       │
                     │           └──┬───┬───┘ └─────────────────┘
                     │              │   │
                     │      ┌───────▼┐  │
                     │      │vl-raster│ │   Dithering, Bildaufbereitung
                     │      └───┬────┘  │
                   ┌─▼──────────▼───────▼─┐
                   │ vl-core               │  Geometrie, Pfade, Ebenen, Dokument, Projektdatei
                   └───────────────────────┘
```

| Crate | Aufgabe | Wichtige Typen/Funktionen |
|---|---|---|
| `vl-core` | Geometrie (Punkt, Rechteck, affine Transformation), Bézierpfade und Abflachen, Ebenen/Laser-Einstellungen, Dokument, Projektformat | `Document`, `Object`, `Shape`, `VectorPath`, `CutSettings`, `save_project` |
| `vl-raster` | Bild auf Gravurraster abtasten (auch gedreht/skaliert), Helligkeit/Kontrast/Gamma, 8 Dithering-Verfahren + Graustufen | `engrave_image`, `dither_gray` |
| `vl-cam` | Werkzeugwege (Linie, Schraffur mit Winkel/Kreuzschraffur/Overscan, Raster), Reihenfolge (nächster Nachbar, innen vor außen), Zeitschätzung, G-Code, Rahmen, Job-Ursprung | `build_job`, `frame_gcode`, `MachineProfile`, `JobSettings` |
| `vl-device` | Treiber-Thread: GRBL mit Zeichenzähl-Streaming und Echtzeitbefehlen, Marlin/Smoothieware mit Senden und Warten; Statusabfrage, Not-Stopp, Simulator für alle drei | `Device`, `Command`, `Event`, `SimTransport` |
| `vl-io` | Import SVG (inkl. Text), DXF (inkl. Blöcke, Bulges, Splines), PDF/AI, HPGL, Bilder; Schrift → Pfad; SVG-/DXF-Export | `import_file`, `text_to_path`, `export_svg`, `export_dxf` |
| `app` | Oberfläche, Bedienlogik, Rückgängig, Einstellungen | `VectorLumeApp` |

Die Oberfläche blockiert nie: Werkzeugwege werden in Hintergrund-Threads berechnet, die Maschinensteuerung läuft in einem
eigenen Treiber-Thread und kommuniziert über Kanäle.

## 4. Funktionen und Umsetzungsstand

### 4.1 Projekt und Dateien
| LF | Funktion | Umsetzung | Stand |
|---|---|---|---|
| LF-01 | Neu/Öffnen/Speichern/Speichern unter | Projektformat `.vlume` (JSON, Bilder als PNG eingebettet), sicheres Speichern über Temp-Datei, Nachfrage bei ungespeicherten Änderungen; **Projektnotizen** (*Ansicht → Projektnotizen*, werden mitgespeichert, wahlweise beim Öffnen angezeigt, sonst Hinweis; nicht Teil von Rückgängig) | ✅ |
| LF-02 | Import SVG/DXF/Bilder | SVG über `usvg` (Einheiten, Transformationen, Text → Pfade, Farbe → Ebene, eingebettete PNG/JPEG/GIF/WebP-Bilder mit Lage), DXF (Linie, Kreis, Bogen, Ellipse, LW-/Polylinie mit Bulge, Spline, Blockreferenzen, Einheiten), PNG/JPG/BMP/GIF/WEBP/TIFF; Drag & Drop; Befehlszeile | ✅ |
| LF-03 | PDF/AI/EPS/HPGL | PDF und PDF-kompatible AI (erste Seite, Formular-XObjekte, Farben → Ebenen; Text/Bilder mit Hinweis übersprungen), HPGL/PLT (PU/PD/PA/PR, CI, AA/AR, Stift → Ebene); EPS (auch mit DOS-Binärkopf) über ein installiertes Ghostscript (Suchpfad, `Programme\gs`, PDF24 oder `VECTORLUME_GHOSTSCRIPT`; wird nur als Programm aufgerufen): Umwandlung in PDF mit Texten als Umrissen, ohne Ghostscript ein klarer Hinweis | ✅ |
| LF-04 | LightBurn-Projekte lesen | `.lbrn2` über „Öffnen“ oder Ziehen auf das Fenster: Ebenen (Modus Linie/Füllen/Versatz-Füllen/Bild, Geschwindigkeit mm/s → mm/min, Leistung max./min., Durchgänge, Zeilenabstand/dpi, Winkel, Füllreihenfolge, Kreuzschraffur, bidirektional, Luft, Ausgabe, ausgeblendet, eigener Name, Priorität → Reihenfolge, Dithering), Rechteck mit Eckenradius, Ellipse, Pfade (Linien und Bézier, gemeinsame Punktlisten), Gruppen (auch verschachtelt, mit Transformation), Texte über die mitgespeicherten Umrisse, eingebettete Bilder samt Kopien und Helligkeit/Kontrast/Gamma, Leistung je Objekt, Projektnotizen samt „beim Öffnen anzeigen“; Werkzeugebenen T1/T2 auf freie Ebenen ohne Ausgabe; getrennte Bild-Einstellungen einer Ebene auf eigene Ebene; Y nach oben → Y nach unten über die Betthöhe. Ergebnis ist ein neues, ungespeichertes Projekt; nicht Übernommenes wird gemeldet. Mit 19 echten Projekten geprüft. Älteres `.lbrn` ⏳ | 🟡 |
| LF-05 | Export | G-Code, SVG (Bilder als PNG eingebettet mit Lage, Bildanpassung und Zuschnitt als `clipPath` – z. B. als Druckvorlage), DXF (R12, Ebenen mit ACI-Farben) | ✅ |
| LF-06 | Rückgängig, Zwischenablage | 200 Schritte, Ausschneiden/Kopieren/Einfügen/Duplizieren | ✅ |
| LF-07 | Zuletzt geöffnet, Autosicherung | Liste der letzten 10 Dateien; Autosicherung alle 90 s im Hintergrund, Wiederherstellen nach Absturz (die Sicherung wird beim Start beiseitegelegt, damit eine per „Öffnen mit …“ übergebene Datei sie nicht ersetzt) | ✅ |

### 4.2 Gestalten und Bearbeiten
| LF | Funktion | Umsetzung | Stand |
|---|---|---|---|
| LF-10 | Arbeitsfläche | Bett mit adaptivem Raster, Lineale in Maschinenkoordinaten, Zoom am Mauszeiger, Verschieben (Mitte/Leertaste); Messwerkzeug (M): Strecke mit Objektfang ziehen, Abstand, ΔX/ΔY und Winkel auf der Arbeitsfläche und in der Werkzeugleiste | ✅ |
| LF-11 | Auswählen und Transformieren | Klick, Umschalt-Klick, Rahmenauswahl (links→rechts: vollständig, rechts→links: berührend), Verschieben, 8 Griffe zum Skalieren, Drehgriff (Umschalt = 15°); Auswahl umkehren, alles auf der aktuellen Ebene, offene Formen auswählen | ✅ |
| LF-12 | Zahleneingabe | X/Y/B/H in Maschinenkoordinaten, Seitenverhältnis sperren, Drehen um Winkel | ✅ |
| LF-13 | Grundformen | Rechteck mit Eckenradius, Ellipse/Kreis, Linie (Umschalt = 45°), Vieleck, Stern | ✅ |
| LF-14 | Text | Installierte Schriften und eingebaute **Einlinienschriften** (Hershey Sans/Serif/Script/Cursive/Gothic aus offenen Strichen zum schnellen Gravieren oder Ritzen, fette und kursive Schnitte bzw. Neigung, Umlaute aus Grundbuchstabe und i-Punkt, ß → ss; Danksagung in Lizenzliste und Info), fett/kursiv, Höhe, Zeichen-/Zeilenabstand, Ausrichtung, Kerning; Text bleibt bearbeitbar (Doppelklick); Text auf Pfad (Versatz, Abstand, Seite); variabler Text mit `{n}` (zählt nach jedem fertigen Job weiter), `{datum}`, `{zeit}`, `{jahr}`; Datenliste aus CSV (Spalten als Platzhalter, `{zeile}`, nächste Zeile nach jedem fertigen Job; `,` `;` Tab, UTF-8/Windows-1252); Umwandeln in Kurven; **Verschweißen** überlappender Buchstaben (Nicht-Null-Regel der Schrift, Löcher bleiben) | ✅ |
| LF-15 | Spiegeln, Drehen, Ausrichten, Verteilen | inkl. **An Linie spiegeln** (Achse mit dem Messwerkzeug ziehen, Auswahl spiegeln oder gespiegelte Kopie anlegen), „Auf Arbeitsfläche zentrieren“, gleiche Breite/Höhe (Maß der zuerst gewählten Form), Gruppen werden als Einheit behandelt | ✅ |
| LF-16 | Gruppen, Zerlegen, Ausblenden | Gruppieren/Lösen, Zerlegen (ein Objekt je Teilpfad, auch Texte), Gummiband-Umriss (konvexe Hülle der Auswahl als neue Form), Ebenen ausblenden und sperren | ✅ |
| LF-17 | Knotenbearbeitung | Knotenwerkzeug (N): Knoten/Kurvengriffe ziehen, Doppelklick fügt ein, Entf löscht, Segment → Linie/Kurve, Pfad öffnen/schließen; **Trimmen** (Schere): Klick entfernt das Linienstück bis zu den nächsten Schnittpunkten mit allem Sichtbaren, auch anderen Teilen desselben Objekts und sich selbst; rote Vorschau unter dem Mauszeiger, ohne Schnittpunkt fällt der ganze Teilpfad weg, übrige Teilpfade behalten ihre Kurven, leere Objekte werden gelöscht | ✅ |
| LF-18 | Boolesche Operationen | Vereinigen, Differenz, Schnittmenge, Exklusiv-Oder, Verschweißen (i_overlay) | ✅ |
| LF-19 | Versatz | nach außen/innen, Ecken rund/spitz/abgeschrägt, offene Pfade werden umrandet | ✅ |
| LF-20 | Reihen-Kopien | Raster, Kreis, entlang eines Pfades (gleichmäßig, an der Richtung ausgerichtet) | ✅ |
| LF-21 | Bild nachzeichnen | Schwellwert, Invertieren, Fleckenfilter, Vereinfachen, Glätten mit Eckenerkennung, Live-Vorschau (eigener Algorithmus) | ✅ |
| LF-22 | Fangen | Raster 1 mm; Objektfang an Knoten, Mitten und Kanten mit Ausrichtungslinien (Alt = aus) | ✅ |
| LF-23 | Laschen/Stege | Anzahl und Breite je Kontur, optional nur Außenkonturen | ✅ |
| LF-24 | QR- und Strichcodes | Werkzeug *QR-/Barcode* (Q): QR-Code (Fehlerkorrektur L/M/Q/H, WLAN-Vorlage), Data Matrix, Code 128 (Zeichensatz B/C automatisch), Code 39, EAN-13, EAN-8, UPC-A (Prüfziffer ergänzt bzw. geprüft); Modulgröße, Strichhöhe, Klartext unter 1D-Codes; Module werden zu Umrissen zusammengefasst (lückenlose Füllung); **zusätzlich:** Strichbreiten-Ausgleich für den Laserpunkt und Invertieren mit Ruhezone für Material, das hell graviert (eloxiertes Aluminium, Schiefer); Inhalt mit denselben Platzhaltern wie variabler Text (Seriennummern, Datenliste), bleibt bearbeitbar; Größe und Fehler werden im Dialog sofort angezeigt | ✅ |
| LF-25 | Bereinigen | *Anordnen → Bereinigen* (Auswahl oder alle sichtbaren Pfade): offene Pfadstücke je Ebene verbinden (Enden innerhalb einer Toleranz, Richtung wird angepasst, Kurven bleiben erhalten) und entstandene Ringe schließen; doppelte Linien löschen (auch gegenläufig und über Objekte hinweg, Pfade werden an entfernten Stellen aufgetrennt, leere Objekte gelöscht); **Formen vereinfachen** (*Anordnen → Formen vereinfachen…*: Knoten reduzieren nach Douglas-Peucker mit Toleranz in mm, wahlweise Glätten zu Kurven, Ecken über 65° bleiben; gleicher Code wie beim Nachzeichnen, Meldung „vorher → nachher Knoten“) | ✅ |
| LF-26 | In Bitmap umwandeln | *Anordnen → In Bitmap umwandeln…*: Auswahl mit wählbarer Auflösung (dpi) in ein Graustufenbild rechnen – Formen gefüllt (Gerade-Ungerade je Objekt, Objekte vereinigt, Kanten geglättet über 4 Unterzeilen und anteilige Randpixel) oder als Kontur, offene Pfade mit Strichbreite, Bilder (samt Anpassung und Zuschnitt) abdunkelnd überlagert; Ergebnis auf einer Bildebene, Originale wahlweise behalten, Größenanzeige und Grenze 60 Mio. Pixel | ✅ |
| LF-27 | Motiv-Bibliothek | Reiter *Bibliothek* rechts: Auswahl samt Ebenen-Einstellungen als Motiv ablegen (`.vlume` im Einstellungsordner `library`), Vorschaubilder, Suche, Klick fügt in der Mitte der Ansicht ein (unbenutzte Ebenen übernehmen die Einstellungen des Motivs, Texte bleiben bearbeitbar), Umbenennen, Löschen mit Rückfrage, Ordner öffnen (Motive lassen sich als Dateien weitergeben) | ✅ |
| LF-28 | Verschachteln | *Anordnen → Verschachteln…*: ausgewählte Teile (Gruppen bleiben zusammen) platzsparend auf die Arbeitsfläche oder die größte ausgewählte geschlossene Form (Reststück) legen; Mindestabstand, Rand, Drehung (keine/180°/90°/45°), kleine Teile in Löcher größerer; gepackt an der Nullpunkt-Ecke, größte Teile zuerst; eigenes Rasterverfahren (berührte Zellen zählen, dadurch keine Überlappung), rechnet im Hintergrund, nicht passende Teile bleiben liegen und werden gemeldet | ✅ |
| LF-29 | Schlitze anpassen | *Anordnen → Schlitze anpassen…*: Kerben und Langlöcher einer Breite (± Toleranz) symmetrisch auf eine neue Breite bringen, z. B. bei anderer Materialstärke; erkannt werden U-Formen aus geraden Kanten mit parallelen Randkanten, deren Inneres leer ist (Materialseite über Umlaufsinn und Verschachtelungstiefe), gleich breite Zapfen bleiben; Kurven und Objekttransformation bleiben erhalten | ✅ |

### 4.3 Ebenen und Laser-Einstellungen
| LF | Funktion | Umsetzung | Stand |
|---|---|---|---|
| LF-30 | 30 Farbebenen | eigene Palette, Klick weist Auswahl zu | ✅ |
| LF-31 | Modi | Linie, Füllen, Versatz-Füllen, Bild; **Unterebenen**: beliebig viele weitere Bearbeitungen derselben Objekte je Ebene (z. B. Füllen, dann Kontur schneiden) mit eigenen Werten und eigenem Ausgabe-Schalter, Reiter in den Ebeneneinstellungen, „Gravieren vor Schneiden“ gilt je Bearbeitung; eine Unterebene im Modus Bild rastert nur die Bilder; LightBurn „Füllen + Linie“ wird als Füllen mit Linien-Unterebene übernommen | ✅ |
| LF-32 | Basiswerte | Geschwindigkeit, Leistung max./min., Durchgänge, Ausgabe, Luft (M8/M9); **Leistung je Objekt** in Prozent der Ebenenleistung (Eigenschaftenleiste, z. B. für Materialtests auf einer Ebene) | ✅ |
| LF-33 | Füllen | Zeilenabstand (mit dpi-Anzeige), Winkel, bidirektional, Kreuzschraffur, Overscan; Gerade-Ungerade-Regel (Löcher bleiben frei); Füllreihenfolge alle zusammen / Gruppen einzeln / Formen einzeln (Außenkontur mit ihren Löchern, nächstgelegenes Teil zuerst, gemeinsames Zeilenraster; aus LightBurn `scanOpt`); **Stempel-Rampe** (Leistung steigt mit dem Abstand zum nächsten Rand von Mindest- auf Höchstleistung, Stufen 0,5 %, wahlweise nur an der äußersten Kontur; Kantenabfrage über ein Raster) | ✅ |
| LF-34 | Bildmodi | Schwellwert, Floyd-Steinberg, Jarvis, Stucki, Atkinson, Burkes, Sierra, Bayer 8×8, Graustufen; Negativ; **Halbton** (runde Punkte, deren Fläche dem Grauwert entspricht, Rasterweite in mm und Winkel einstellbar) und **zusätzlich Linienraster** (Linien mit grauwertabhängiger Breite, Kupferstich-Optik); **Punktbreiten-Korrektur** (gebrannte Strecken an Übergängen zu „aus“ um die halbe Punktbreite gekürzt, Einzelpunkte bleiben als kurzer Punkt); **Bearbeitetes Bild speichern** (Rechtsklick auf ein Bild: PNG genau so, wie es graviert wird – ein Pixel je Rasterpunkt, Werte der Bild-Bearbeitung seiner Ebene, dpi im Dateinamen) | ✅ |
| LF-35 | Bildanpassung | Helligkeit, Kontrast, Gamma, live in der Ansicht; **Bild mit Form zuschneiden** (Bild + geschlossene Formen auswählen, Zuschnitt wird im Bild gespeichert, außerhalb weder angezeigt noch graviert – auch im Negativ-Modus – noch nachgezeichnet; „Zuschnitt aufheben“ stellt das ganze Bild wieder her) | ✅ |
| LF-36 | Reihenfolge | Ebenen verschieben; **zusätzlich „Gravieren vor Schneiden“** (Bild/Füllen vor Schnitten) | ✅ |
| LF-37 | Materialbibliothek | Material/Stärke/Titel/Notiz, Suche, Import/Export (`.vlmat`), Übernehmen auf Ebene | ✅ |
| LF-38 | Z/Kerf | Schnittbreitenausgleich; Z-Versatz und Absenkung je Durchgang (nur mit Z-Achse im Profil); **Überschnitt** (geschlossene Konturen laufen über den Startpunkt hinaus, höchstens eine Runde) und **Perforieren** (abwechselnd Schnitt- und Lückenlänge, Lücken im Vorschub mit Laser aus, läuft über Ecken weiter); **Ein-/Auslauf** an geschlossenen Konturen (Länge je mm, Winkel 0–90°; setzt im Abfall an – außen bei Teilen, innen bei Löchern, ermittelt über die Verschachtelungstiefe) | ✅ |
| LF-39 | Testmuster | Materialtest: zwei Achsen aus Geschwindigkeit, Leistung, Durchgänge, Zeilenabstand, Fokus; beschriftet, Vorschau, Feld als Material speichern | ✅ |

### 4.4 Maschinensteuerung
| LF | Funktion | Umsetzung | Stand |
|---|---|---|---|
| LF-40 | Verbindung | Anschlussliste mit USB-Kennung, Baudrate aus Profil, automatische Abfrage `$I` und `$$` | ✅ |
| LF-41 | Status und Konsole | Zustand, Position (WPos/MPos+WCO) alle 200 ms, Fehler- und Alarmcodes mit deutschem Text, Konsole mit Schnellbefehlen und eigenen **Makro-Knöpfen** (Name + G-Code-Zeilen, in den Einstellungen gespeichert, Rechtsklick bearbeitet/entfernt, nur verbunden und ohne laufenden Job) | ✅ |
| LF-42 | Referenzfahrt, Entsperren, Jog | `$H`, `$X`, `$J=` mit Schrittweite und Geschwindigkeit, Richtungen passend zur Nullpunkt-Ecke, Jog-Abbruch, „Laser zur Auswahl fahren“ (Job-Ankerpunkt, absolute Koordinaten) | ✅ |
| LF-43 | Rahmen | Umriss der Ausgabe abfahren (Laser aus oder ≤ 5 % laut Profil), Anfahrt immer im Eilgang; als Rechteck oder als **Umriss-Rahmen** (konvexe Hülle aller gelaserten Strecken, folgt runden und schrägen Formen) | ✅ |
| LF-44 | Job-Steuerung | Start, Pause (`!`), Fortsetzen (`~`), Stopp (Feed-Hold + Soft-Reset), Fortschritt, Restzeit; **fremde G-Code-Datei senden** (*Laser → G-Code-Datei senden…*: Kommentare, Leerzeilen, `%` und BOM entfernt, Bestätigung mit Warnhinweis, Inhalt ungeprüft, Restzeit aus dem Fortschritt hochgerechnet) | ✅ |
| LF-45 | Job-Ursprung | Absolut, Benutzer-Ursprung, aktuelle Position (G91 mit fehlerfreier Rundung), 9 Ankerpunkte | ✅ |
| LF-46 | Maschinenprofil | **Mehrere Geräte:** Liste in den Maschineneinstellungen (Neu, Duplizieren, Löschen mit Rückfrage, Import/Export als `.vldevice`), Umschalten auch im Laser-Bereich (nur getrennt), ältere Einstellungen werden zum ersten Gerät; je Gerät Fläche, Nullpunkt-Ecke, S-Max, M3/M4, Eilgang, Beschleunigung, Luft-Befehle, Empfangspuffer, Rahmen; „Werte vom Gerät“ (`$30`, `$130/$131`, `$110`, `$120`); **Scan-Versatz:** Tabelle Geschwindigkeit → gemessener Versatz zwischen hin- und zurückgefahrenen Zeilen, jede Füll- und Bildzeile (auch im Materialtest) wird um die Hälfte gegen ihre Fahrtrichtung verschoben; linear interpoliert, außerhalb proportional zur Geschwindigkeit, leer = aus | ✅ |
| LF-47 | Simulator | GRBL-Nachbildung mit 128-Byte-Puffer, Planer (15), Echtzeitbefehlen, Alarm nach Reset | ✅ |
| LF-48 | weitere Steuerungen | Marlin 2.1 (Laser im Inline-Modus `M3/M4 I`, S bis 255, Senden und Warten auf `ok`, Jobende per `M400`, Position per `M114`, Pause im Programm, Stopp `M410`+`M5`, Referenzfahrt `G28`), Smoothieware (S 0…1, Statusabfrage `?`, Stopp Strg-X mit `M112` als Rückfall, Freigabe `M999`); beide experimentell, da ohne Hardwaretest; Ruida, Trocen, TopWisdom, Galvo ⏳ | 🟡 |
| LF-49 | Netzwerk | TCP/Telnet für FluidNC, grblHAL, ESP3D: Eintrag „Netzwerk“ mit Adresse (Standard-Port 23), alle Steuerungsarten | ✅ |
| LF-50 | Drehachse, Kamera, Drucken & Schneiden | Drehachse: Rollen oder Futter, statt Y-Motor oder als A-Achse, Weg je Umdrehung, Rollen-/Werkstückdurchmesser (auch als Umfang), Richtung; Y wird um den Ankerpunkt auf den Achsweg umgerechnet (Job, Rahmen, Verfahren), Prüfung Motivhöhe ≤ Umfang, Testdrehungen; **Kamera:** Webcam (Einzelbild oder live alle 0,3 s) oder Foto der Arbeitsfläche, Ausrichten über vier Passpunkte (Bettlage in Maschinenkoordinaten, Klick ins Bild; „Ausrichtmarken lasern“ brennt Kreuze an diesen Stellen), perspektivische Entzerrung (Homographie) und halbtransparente Anzeige unter dem Raster; Objektivverzerrung (Fischauge) wird nicht ausgeglichen; **Drucken & Schneiden** (*Laser → Drucken & Schneiden…*): zwei Passermarken (Kreuz mit Kreis, eigene Ebene ohne Ausgabe) einfügen oder im Entwurf wählen (Mitte der Auswahl oder Klick mit Objektfang), dieselben Marken auf dem Material per Laserposition, Klick (z. B. ins Kamerabild) oder Zahleneingabe; „Hinfahren“ zur Entwurfsmarke als Startpunkt fürs Nachjustieren; Ausrichten der Auswahl bzw. des ganzen Entwurfs durch Drehen, Verschieben und wahlweise gleichmäßiges Skalieren (ohne Skalieren treffen sich die Mitten), Anzeige von Verschiebung/Drehung/Maßstab mit Warnung ab 2 % Maßstabsabweichung, Startposition wird auf absolute Koordinaten gestellt | ✅ |

### 4.5 Vorschau und Optimierung
| LF | Funktion | Umsetzung | Stand |
|---|---|---|---|
| LF-60 | Vorschau | Werkzeugweg, Rasterbilder, Leerfahrten, Laserweg, Laufzeit (Beschleunigungsmodell), Zeilenzahl | ✅ |
| LF-61 | Optimierung | nächster Nachbar mit Bounding-Box-Vorfilter, Startpunkt auf geschlossenen Konturen frei wählbar, offene Pfade werden umgedreht, „innen vor außen“ als Abhängigkeitsgraph | ✅ |
| LF-62 | Bereichsprüfung | Job inkl. Overscan gegen Arbeitsfläche, Bestätigung bei Überschreitung | ✅ |
| LF-63 | Abspielbare Vorschau | Schieberegler mit Kopfmarkierung | ✅ |

## 5. Daten

### 5.1 Koordinatensysteme
- **Dokument:** mm, X nach rechts, Y nach unten, (0,0) = hintere linke Ecke (Bildschirmansicht).
- **Maschine:** abhängig von der Nullpunkt-Ecke im Profil (z. B. vorne links: `x = X`, `y = Bett − Y`). Die Abbildung ist
  ihre eigene Umkehrung und wird nur an einer Stelle definiert (`MachineProfile::doc_to_machine`).
- **Ausgabe:** absolut, relativ zum Benutzer-Ursprung oder relativ zur Kopfposition (G91). Relative Bewegungen werden aus
  auf Mikrometer gerundeten Absolutwerten gebildet, dadurch summieren sich keine Rundungsfehler.

### 5.2 Projektdatei `.vlume`
JSON mit `format`, `version`, `generator` und dem Dokument (Objekte mit Transformation, Pfadsegmenten bzw. Bild als
Base64-PNG, 30 Ebenen, Ebenenreihenfolge). Ältere Dateien werden beim Laden repariert (`Document::repair`), neuere
Formatversionen werden mit Hinweis abgelehnt.

### 5.3 Einstellungen
`%APPDATA%\VectorLume\settings.json`: Maschinenprofil (inkl. Steuerung und Drehachse), Job-Optionen, Design, letzte
Dateien, Netzwerkadresse, Jog-Schritt, Sicherheitshinweis bestätigt, Kamera, Sprache. Mit `VECTORLUME_CONFIG_DIR` lässt
sich ein anderer Ordner verwenden (Tests).

### 5.4 Sprache der Oberfläche
Deutsch und Englisch, umschaltbar unter *Ansicht → Sprache* ohne Neustart; beim ersten Start gilt die Sprache des
Systems (`sys-locale`). Die Texte stehen im Quelltext auf Deutsch und werden zur Laufzeit nachgeschlagen
(`app/src/i18n.rs`, Tabelle `app/src/i18n_en.rs`): `tr("…")` für feste Texte, `trs(&format!(…))` für zusammengesetzte.
Zusammengesetzte Meldungen – auch Fehlermeldungen der Bibliotheken, die zentral bei Hinweisen und Dialogen übersetzt
werden – erkennt eine Vorlage mit `{}`; die eingesetzten Werte (Dateinamen, Zahlen, Modusnamen) bleiben erhalten und
werden ihrerseits übersetzt, wenn sie bekannt sind. Fehlt eine Übersetzung, erscheint der deutsche Text. Ein Test
prüft, dass jeder `tr`/`trs`-Text im Quelltext einen englischen Eintrag hat. Die Beschriftung des Materialtests wird in
der eingestellten Sprache gelasert; Namen von Ebenen und Objekten sind Benutzerdaten und werden nicht übersetzt.
Weitere Sprachen: eine Tabelle wie `i18n_en.rs` und ein Eintrag in `Language`.

## 6. Schnittstellen

### 6.1 GRBL
- **Streaming:** Zeichenzähl-Verfahren – es sind nie mehr Bytes unterwegs als der Empfangspuffer fasst (Standard 127).
- **Echtzeitbefehle:** `?` (alle 200 ms), `!`, `~`, `0x18` (Reset), `0x85` (Jog-Abbruch).
- **Start:** nach dem Öffnen Warten auf die Begrüßung oder 2,5 s (Arduino-Boards starten beim Öffnen neu).
- **Fehler im Job:** sofortiger Stopp mit Meldung der Zeile; Alarm beendet den Job.
- **Job fertig:** alle Zeilen bestätigt und Maschine wieder im Zustand „Idle“.

### 6.2 G-Code (GRBL-Laser-Modus)
`G21`, `G90`/`G91`, einmalig `M4 S0` (oder `M3`), danach Leistung über `S` je Bewegung, `G0` für Eilgänge,
modale Befehle und unveränderte Achsen werden weggelassen (weniger Bytes → flüssigeres Rastern). Am Ende `M5`,
Luft aus, optional Rückfahrt, im Relativmodus zurück auf `G90`. Kommentare nur ASCII, beim Senden entfernt.

### 6.3 Marlin und Smoothieware (experimentell)
- **Streaming:** Senden und Warten – eine Zeile, dann auf `ok` (auch `ok …` mit Anhang) warten. Außerhalb eines Jobs
  gilt eine Zeile nach 3 s ohne jede Antwort als beantwortet, damit die Verbindung nach einem Stopp nicht hängen bleibt.
- **Job fertig:** der Treiber hängt `M400` an; dessen `ok` kommt erst, wenn alle Bewegungen ausgeführt sind.
- **Status:** Smoothieware über `?` (auch älteres Format `<Idle,MPos:…>`); Marlin im Ruhezustand über `M114` alle
  500 ms (ohne Konsolenausgabe), während des Jobs wird der Zustand aus dem Treiber gemeldet.
- **Pause:** im Programm – es werden keine weiteren Zeilen gesendet, die Steuerung hält nach ihrem Puffer an.
- **Stopp:** Marlin `M410` (sofort mit `EMERGENCY_PARSER`) und `M5`; Smoothieware Strg-X, `M112` als Rückfallebene,
  Freigabe mit `M999`. Halt-Meldungen (`!!`, `kill()`) beenden den Job.
- **G-Code:** Marlin `M3 I`/`M4 I` (Inline-Modus: Leistung nur bei G1), S bis 255; Smoothieware ohne Einschaltbefehl,
  S als Anteil 0…1 mit bis zu vier Nachkommastellen.

### 6.4 Netzwerk
Anschluss „Netzwerk“ mit Adresse `Host[:Port]` (Standard 23): rohe TCP-Verbindung wie bei FluidNC, grblHAL und ESP3D,
für alle Steuerungsarten. Da Netzwerksteuerungen beim Verbinden nicht neu starten, wird nur 0,3 s gewartet.

### 6.5 Drehachse
Y im Dokument ist die abgewickelte Oberfläche. Ausgabe: `y' = Anker + (y − Anker) · k` mit
`k = Weg je Umdrehung / (π · Durchmesser)` (Rolle: Rollendurchmesser, Futter: Werkstückdurchmesser), bei A-Achse als
`A`-Wörter mit dem Anker bei A = 0. Gilt für Job, Rahmen und Verfahren in Y.

## 7. Sicherheitskonzept

| Risiko | Maßnahme |
|---|---|
| Laser brennt bei Leerfahrten | Laser-Modus `$32` wird beim Verbinden gelesen; bei `$32=0` Warnung und Startsperre mit Angebot `$32=1` |
| Laser bleibt nach Abbruch an | Stopp = Feed-Hold + Soft-Reset; ebenso bei Verbindungsfehler und beim Beenden des Programms |
| Befehle während des Jobs | Konsole, Jog, Referenzfahrt sind gesperrt; nur Pause/Stopp |
| Job außerhalb der Fläche | Prüfung inkl. Overscan, Bestätigung nötig |
| Rahmenfahren mit Leistung | Standard 0 %, maximal 5 %, Anfahrt immer im Eilgang |
| Bedienfehler | Einmaliger Sicherheitshinweis (Klasse 4, Schutzbrille, Aufsicht, Belüftung) vor dem ersten Job |
| Beenden während Job | Schließen wird verhindert, Hinweis erscheint |

## 8. Qualitätssicherung

- **Automatische Tests (162):** Geometrie, Pfade, Projektdatei, Dithering, Bildabtastung (auch gedreht), Schraffur mit
  Löchern und Winkeln, Rasterläufe, Reihenfolge/innen-vor-außen, Zeitschätzung, G-Code (absolut/relativ, exakte
  Rückkehr), Bereichsprüfung, GRBL-Antworten, Streaming über den Simulator inkl. Pufferüberlauf-Erkennung und Not-Stopp,
  SVG/DXF/PDF/HPGL/Bild/Schrift-Import, DXF-Export, Rückgängig, Boolesche Operationen/Versatz/Laschen, Knoten,
  Objektfang, Materialtest und -bibliothek, Z je Durchgang, Nachzeichnen, Platzhalter, Text auf Pfad, Drehachse,
  G-Code-Dialekte und Streaming für Marlin/Smoothieware (Simulator), Netzwerkverbindung, CSV-Datenlisten,
  LightBurn-Import (Ebenen, Formen, Gruppen, gemeinsame Punktlisten, Werkzeugebenen), Leistung je Objekt, EPS (mit
  Ghostscript, sonst übersprungen), Homographie und Entzerrung des Kamerabildes, Scan-Versatz (Interpolation, Verschiebung je Fahrtrichtung), Strichcodes (Umrisse, Prüfziffern, Zeichensatzwahl, QR-Code wird mit einem unabhängigen Decoder (`rqrr`, nur Test) wieder gelesen, auch invertiert), Übersetzung (Vorlagen, verschachtelte
  Meldungen, Vollständigkeit der englischen Tabelle). Zusätzlich
  prüft ein abschaltbarer Test echte LightBurn-Projekte aus einem Ordner (`VL_LBRN_DIR`).
- **End-to-End:** Ein Job mit Schnitt, Kreuzschraffur und Graustufenbild sowie die Rahmenfahrt werden zeilenweise durch
  den Simulator gestreamt – jede Zeile muss mit `ok` bestätigt werden.
- **Oberfläche:** manuell bzw. per automatisierten Klicks geprüft (Zeichnen, Drehen, Text, Import, Vorschau, Job im
  Simulator starten und stoppen).
- **Statische Prüfung:** `cargo clippy --workspace --all-targets` ohne Warnungen, `cargo fmt`.
- **Hardware-Test (durch den Auftraggeber):** zuerst Rahmen ohne Leistung, dann ein kleiner Testjob mit 5–10 % Leistung.

Befehle:
```
cargo test --workspace
cargo clippy --workspace --all-targets
cargo build --release
```

## 9. Abhängigkeiten und Lizenzen

Alle Abhängigkeiten sind freizügig lizenziert (MIT, Apache-2.0, BSD, Zlib, ISC, Unicode); Pakete mit GPL/LGPL-Option
werden unter ihrer MIT/Apache-Alternative genutzt. `serialport` (und `option-ext` über `dirs`) stehen unter MPL-2.0
(Änderungen an deren Dateien wären offenzulegen, die eigene Software nicht). `mozjpeg` (JPEG-Dekoder der Kamera) steht
unter der IJG-Lizenz: In der Dokumentation einer Weitergabe ist zu vermerken „This software is based in part on the work
of the Independent JPEG Group“. Die eingebauten Ersatzschriften von egui stehen unter OFL-1.1 bzw. der Ubuntu Font
Licence (Weitergabe mit Software erlaubt). Damit bleiben Veröffentlichung als Open Source und Verkauf möglich.

**Weitergabe:** `tools\package.ps1` erzeugt das Setup (Inno Setup 6, Benutzerinstallation ohne Adminrechte, gleicher
Ordner und gleiche Dateitypen wie `install.ps1`, Deinstallation über *Apps*, Assistent deutsch/englisch) und ein portables
ZIP. Beide enthalten `THIRD-PARTY-LICENSES.html` mit den Lizenztexten aller mitgelieferten Bibliotheken (cargo-about;
`tools\about.toml` listet die zulässigen Lizenzen – eine neue, nicht aufgeführte Lizenz bricht die Paketierung ab).
Der Info-Dialog verlinkt die Datei. Inno Setup darf auch für kommerzielle Software verwendet werden. Code-Signatur ⏳
(ohne Zertifikat warnt SmartScreen).

| Paket | Zweck | Lizenz |
|---|---|---|
| eframe/egui 0.36 | Oberfläche | MIT/Apache-2.0 |
| egui-phosphor 0.14 | Icons | MIT/Apache-2.0 |
| usvg 0.48 | SVG | Apache-2.0/MIT |
| dxf 0.6 | DXF | MIT |
| lopdf 0.46 | PDF/AI-Import | MIT |
| i_overlay 9.1 | Boolesche Operationen, Versatz | MIT/Apache-2.0 |
| winresource 0.1 | Programmsymbol (Build) | MIT |
| image 0.25 | Bilder | MIT/Apache-2.0 |
| serialport 4.10 | USB/seriell | MPL-2.0 |
| rfd 0.17 | Dateidialoge | MIT |
| fontdb 0.24, ttf-parser 0.25 | Schriften | MIT, MIT/Apache-2.0 |
| qrcode 0.14, datamatrix 0.3, barcoders 2 | QR-, Data-Matrix- und Strichcodes | MIT/Apache-2.0 |
| serde, serde_json, base64, dirs, log | Infrastruktur | MIT/Apache-2.0 |
| roxmltree | XML lesen (LightBurn-Projekte) | MIT/Apache-2.0 |
| sys-locale 0.3 | Systemsprache beim ersten Start | MIT/Apache-2.0 |
| nokhwa 0.10 | Kamera (Media Foundation unter Windows) | Apache-2.0 |
| mozjpeg 0.10 (über nokhwa) | JPEG-Bilder der Kamera | IJG, Zlib, BSD-3-Clause |

## 10. Weiteres Vorgehen

| Phase | Inhalt |
|---|---|
| 1 ✅ | Nutzbare Version für GRBL/SF-A9 (dieses Dokument) |
| 2 ✅ | Knotenbearbeitung, Boolesche Operationen, Offset/Offset-Füllen, Arrays, Laschen, Materialbibliothek, Testmuster-Generator, Autosicherung, Objektfang, PDF-Import |
| 3 🟡 | Bild nachzeichnen ✅, Text auf Pfad/variabler Text ✅, Rotary ✅, Kamera ✅, Marlin/Smoothieware ✅ (experimentell), danach Ruida und Galvo (Hardware-Tester nötig) |
| 4 🟡 | Eigene Verbesserungen (u. a. Leistung je Objekt, LightBurn-Import, Scan-Versatz, Strichcodes ✅), Installer (Benutzerinstallation mit Verknüpfungen und Dateitypen `.vlume`/„Öffnen mit“ für `.lbrn2` ✅, Setup-Paket ✅, Code-Signatur ⏳), mehrsprachige Oberfläche (Deutsch/Englisch ✅), Veröffentlichung |
