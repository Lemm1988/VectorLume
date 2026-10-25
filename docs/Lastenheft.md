# Lastenheft – VectorLume

| | |
|---|---|
| Projekt | VectorLume – Software zum Gestalten und Steuern von Laser-Gravier- und Schneidmaschinen |
| Auftraggeber | Projektinhaber (GitHub: Lemm1988) |
| Stand | 24.10.2026, Version 1.0 |
| Status | privat, Veröffentlichung als Open Source zu einem späteren Zeitpunkt, Verkauf nicht ausgeschlossen |

## 1. Ausgangssituation

Der Auftraggeber betreibt einen Diodenlaser **Sculpfun SF-A9 (40 W, GRBL-Steuerung, ca. 400 × 400 mm Arbeitsfläche)**.
Auf dem Markt etablierte Programme (z. B. LightBurn) sind kostenpflichtig und geschlossen. Es soll eine eigene,
moderne Software entstehen, die den Funktionsumfang solcher Programme abdeckt, anschließend verbessert wird
und später auch von anderen Laser-Besitzern genutzt werden kann.

## 2. Zielsetzung

1. Eigenständige Desktop-Software in **Rust** mit moderner, gut aussehender Oberfläche.
2. Funktionsumfang vergleichbar mit marktüblicher Laser-Software (Referenz: LightBurn 2.x), umgesetzt als
   **eigene Implementierung**.
3. Zuerst voll nutzbar mit dem SF-A9 (GRBL), danach Unterstützung weiterer Steuerungen.
4. Später: eigene Verbesserungen, die über die Referenz hinausgehen.

## 3. Rechtliche Rahmenbedingungen

- Es werden **nur Funktionen nachgebaut**, keine fremden Inhalte übernommen: kein Code, keine Icons, Grafiken,
  Oberflächentexte, Handbuchtexte oder Screenshots aus LightBurn oder anderen geschlossenen Programmen.
- **Kein Dekompilieren/Reverse Engineering** geschlossener Programme (Lizenzbedingungen, §69e UrhG).
- Fremde Dateiformate dürfen zur Kompatibilität gelesen werden (z. B. `.lbrn2`, das XML ist), anhand eigener Dateien
  bzw. öffentlicher Dokumentation.
- Name und Erscheinungsbild dürfen nicht mit LightBurn verwechselbar sein.
- Abhängigkeiten nur mit freizügigen Lizenzen (MIT, Apache-2.0, BSD, Zlib, MPL-2.0), damit Veröffentlichung
  und Verkauf möglich bleiben.

## 4. Produkteinsatz

- **Zielgruppe:** Hobby- und Kleingewerbe-Anwender von Dioden-, CO2- und Faserlasern.
- **Betriebssysteme:** Windows 10/11 (zuerst), später Linux und macOS.
- **Sicherheit:** Laser der Klasse 4 – die Software darf keinen Zustand herbeiführen, in dem der Laser unbeabsichtigt feuert.

## 5. Funktionale Anforderungen

Die Anforderungen sind nach Bereichen gegliedert. Priorität: **M** = Muss (erste nutzbare Version),
**S** = Soll (Funktionsgleichheit mit der Referenz), **K** = Kann (Verbesserungen, später).

### 5.1 Projekt und Dateien
| Nr. | Anforderung | Prio |
|---|---|---|
| LF-01 | Projekt neu anlegen, öffnen, speichern, „speichern unter“ (eigenes Projektformat) | M |
| LF-02 | Import von SVG, DXF, PNG, JPG, BMP, GIF, WEBP, TIFF | M |
| LF-03 | Import von PDF, AI, EPS, HPGL | S |
| LF-04 | Import von LightBurn-Projekten (`.lbrn`/`.lbrn2`) zur Kompatibilität | S |
| LF-05 | Export als G-Code, SVG und DXF | M (G-Code) / S |
| LF-06 | Rückgängig/Wiederholen, Zwischenablage, Duplizieren | M |
| LF-07 | Zuletzt geöffnete Dateien, automatische Sicherung | S |

### 5.2 Gestalten und Bearbeiten
| Nr. | Anforderung | Prio |
|---|---|---|
| LF-10 | Arbeitsfläche mit Raster, Linealen in Maschinenkoordinaten, Zoom und Verschieben | M |
| LF-11 | Auswählen (Klick, Mehrfachauswahl, Rahmenauswahl), Verschieben, Skalieren, Drehen | M |
| LF-12 | Zahleneingabe für Position, Größe, Drehung | M |
| LF-13 | Grundformen: Rechteck (mit Eckenradius), Ellipse/Kreis, Linie, Polygon, Stern | M / S |
| LF-14 | Text mit installierten Schriften, Text auf Pfad, variabler Text (Seriennummern, CSV) | M (Text) / S |
| LF-15 | Spiegeln, 90°-Drehung, Ausrichten und Verteilen | M |
| LF-16 | Gruppieren/Gruppe auflösen, Ebenen-Sperre, Ausblenden | S |
| LF-17 | Knotenbearbeitung (Punkte verschieben, einfügen, löschen, Kurven glätten) | S |
| LF-18 | Boolesche Operationen (Vereinigen, Differenz, Schnitt), Verschweißen | S |
| LF-19 | Versatz (Offset) von Pfaden, Pfade schließen/verbinden, Optimieren | S |
| LF-20 | Reihen-Kopien (Raster, kreisförmig), Kopien entlang eines Pfades | S |
| LF-21 | Bild nachzeichnen (Vektorisierung) | S |
| LF-22 | Fangen/Einrasten am Raster und an Objekten, Hilfslinien | S |
| LF-23 | Laschen/Stege (Tabs) für Schneidteile | S |
| LF-25 | Bereinigen importierter Zeichnungen: offene Pfade verbinden/schließen, doppelte Linien löschen | S |
| LF-26 | Vektoren, Texte und Bilder in ein Bild umwandeln (zum gemeinsamen Rastern) | K |
| LF-27 | Bibliothek für wiederkehrende Motive | K |
| LF-28 | Teile platzsparend auf Material verschachteln (Nesting), auch auf Reststücken | K |
| LF-24 | QR-Codes und Strichcodes (EAN, Code 128 u. a.), auch mit fortlaufender Nummer | S |

### 5.3 Ebenen und Laser-Einstellungen
| Nr. | Anforderung | Prio |
|---|---|---|
| LF-30 | Farbebenen (Palette mit mind. 30 Farben), Objekte per Klick einer Ebene zuweisen | M |
| LF-31 | Modi je Ebene: Linie, Füllen, Bild; später Offset-Füllen; mehrere Bearbeitungen je Ebene (Unterebenen) | M / S |
| LF-32 | Je Ebene: Geschwindigkeit, Leistung min/max, Durchgänge, Ausgabe an/aus, Luftunterstützung | M |
| LF-33 | Füllen: Linienabstand, Winkel, bidirektional, Kreuzschraffur, Overscan | M |
| LF-34 | Bild: Graustufen und Dithering (Schwellwert, Floyd-Steinberg, Jarvis, Stucki, Atkinson, Burkes, Sierra, geordnet, Halbton), Negativ | M |
| LF-35 | Bildanpassung: Helligkeit, Kontrast, Gamma; Bild auf eine Form zuschneiden | M / S |
| LF-36 | Reihenfolge der Ebenen festlegen | M |
| LF-37 | Materialbibliothek (Einstellungen speichern/laden) | S |
| LF-38 | Z-Achse/Fokus je Durchgang, Schnittfugen-Ausgleich (Kerf) | S |
| LF-39 | Testmuster-Generator für Material-Tests (Leistung × Geschwindigkeit) | S |

### 5.4 Maschinensteuerung
| Nr. | Anforderung | Prio |
|---|---|---|
| LF-40 | Verbindung über USB/seriell zu GRBL-Steuerungen, Port-Auswahl, Baudrate | M |
| LF-41 | Statusanzeige (Zustand, Position), Konsole mit Befehlseingabe | M |
| LF-42 | Referenzfahrt, Entsperren, Verfahren (Jog) in Schritten | M |
| LF-43 | Rahmen fahren (Umriss des Jobs abfahren) | M |
| LF-44 | Job starten, pausieren, fortsetzen, stoppen; Fortschrittsanzeige | M |
| LF-45 | Job-Ursprung: absolut, Benutzer-Ursprung, aktuelle Position; Ankerpunkt (9 Punkte) | M |
| LF-46 | Maschinenprofil (Arbeitsfläche, Nullpunkt-Ecke, S-Max, Geschwindigkeiten), Werte vom Gerät lesen | M |
| LF-47 | Simulator ohne angeschlossenen Laser | M |
| LF-48 | Weitere Steuerungen: Marlin, Smoothieware, Ruida, Trocen, TopWisdom, Galvo (EZCAD-Karten) | S |
| LF-49 | Netzwerkverbindung (WLAN/Ethernet) für Steuerungen, die es unterstützen | S |
| LF-50 | Drehachse (Rotary), Kamera-Ausrichtung, Drucken & Schneiden | S |

### 5.5 Vorschau und Optimierung
| Nr. | Anforderung | Prio |
|---|---|---|
| LF-60 | Vorschau des Jobs mit Leerfahrten und geschätzter Laufzeit | M |
| LF-61 | Schnittreihenfolge optimieren (kürzeste Wege, innen vor außen) | M |
| LF-62 | Prüfung, ob der Job in die Arbeitsfläche passt | M |
| LF-63 | Abspielbare Vorschau (Fortschritt schrittweise ansehen) | S |

## 6. Nichtfunktionale Anforderungen

| Nr. | Anforderung |
|---|---|
| NF-01 | Moderne, aufgeräumte Oberfläche mit dunklem und hellem Design, deutsche Sprache (später weitere) |
| NF-02 | Flüssige Bedienung bei typischen Projekten (bis ca. 10 000 Pfade bzw. 20 Megapixel Bild) |
| NF-03 | Laser darf nie unbeabsichtigt feuern: Stopp und Verbindungsabbruch schalten ihn ab, Leerfahrten ohne Leistung |
| NF-04 | Prüfung des GRBL-Laser-Modus (`$32=1`) und Warnung bei Abweichung |
| NF-05 | Modularer Aufbau, damit weitere Steuerungen und Dateiformate ergänzt werden können |
| NF-06 | Automatisierte Tests für Geometrie, Werkzeugwege, G-Code und Streaming |
| NF-07 | Quellcode und Dokumentation versioniert in einem privaten GitHub-Repository |

## 7. Lieferumfang und Abnahme

- Lauffähige Windows-Anwendung `vectorlume.exe`, Quellcode, Lastenheft, Pflichtenheft, README.
- **Abnahme der ersten Version:** Alle Muss-Anforderungen erfüllt, alle automatisierten Tests grün,
  ein Testjob (Linie, Füllung, Bild) läuft im Simulator vollständig durch und erzeugt gültigen G-Code.
- Der Test an der echten Maschine erfolgt durch den Auftraggeber, zuerst mit geringer Leistung und Schutzbrille.
