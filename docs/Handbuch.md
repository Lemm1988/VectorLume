# VectorLume – Handbuch

VectorLume ist ein Programm zum Gestalten und Lasern für Diodenlaser mit GRBL-Steuerung, z. B. den Sculpfun SF-A9. Du zeichnest oder importierst Formen, legst für jede Farbebene fest, wie gelasert wird, prüfst das Ergebnis in der Vorschau und schickst den Job direkt an den Laser.

> Sicherheit zuerst: Laserschutzbrille tragen, nie unbeaufsichtigt lasern, für Absaugung und Brandschutz sorgen und neue Einstellungen zuerst mit wenig Leistung an einem Reststück testen.

## Installation und erster Start

1. `VectorLume-Setup-<Version>.exe` ausführen. Adminrechte sind nicht nötig; entfernen lässt sich das Programm über *Einstellungen → Apps*.
2. VectorLume über die Verknüpfung auf dem Desktop oder im Startmenü starten.
3. *Hilfe → Beispielprojekt laden* zeigt alle Bearbeitungsarten an einem Beispiel.

Die Sprache (Deutsch/Englisch), das Design (dunkel/hell) und die Einheit (mm/Zoll) stellst du unter *Ansicht* ein. `.vlume`-Projekte öffnen sich per Doppelklick; LightBurn-Projekte (`.lbrn2`) öffnest du über *Datei → Öffnen*.

Ohne angeschlossenen Laser kannst du alles mit dem Anschluss **Simulator** ausprobieren.

## Den Laser verbinden

1. Den Laser per USB anschließen und einschalten. Windows meldet ihn als COM-Anschluss. Erscheint keiner, fehlt meist der USB-Treiber des Laserherstellers.
2. Rechts im Reiter **Laser** den Anschluss wählen und **Verbinden** klicken. In der **Konsole** siehst du, was die Steuerung meldet.
3. *Laser → Maschineneinstellungen…*: Mit **Werte vom Gerät** übernimmst du Arbeitsfläche und S-Max (`$30`) aus der Steuerung. Prüfe auch, an welcher Ecke der Nullpunkt liegt (beim SF-A9 vorne links).
4. VectorLume prüft den Laser-Modus. Steht er aus (`$32=0`), bleibt der Laser bei Leerfahrten an. Mit einem Klick auf **$32=1 senden** schaltest du ihn dauerhaft ein.

Mehrere Laser verwaltest du in den Maschineneinstellungen als eigene Geräte. Netzwerk-Steuerungen (z. B. FluidNC) verbindest du über den Anschluss *Netzwerk*.

## Die Oberfläche

- **Menüleiste** oben: *Datei*, *Bearbeiten*, *Anordnen*, *Ansicht*, *Laser*, *Hilfe*.
- **Werkzeugleiste** links: Auswählen, Knoten bearbeiten, Trimmen, Laschen setzen, Rechteck, Ellipse, Linie, Vieleck, Stern, Text, QR-/Barcode und Messen.
- **Arbeitsfläche** in der Mitte: Mausrad zoomt; mit gedrückter mittlerer Maustaste oder Leertaste + Ziehen verschiebst du die Ansicht. Lineale und Raster zeigen Maschinenkoordinaten.
- **Rechte Seite**: oben die Reiter *Laser*, *Bewegen*, *Konsole* und *Bibliothek*, darunter die **Ebenen** mit ihren Einstellungen.
- **Statusleiste** unten: Mausposition, Objektanzahl, Zoom und Maschinenzustand; darunter die Farbleiste der Ebenen.

## Gestalten

Wähle ein Werkzeug und ziehe auf der Arbeitsfläche. Mit dem Auswahlwerkzeug (V) verschiebst, skalierst (Griffe an Ecken und Kanten) und drehst (Drehgriff, Umschalt = 15°-Schritte) du Formen. Ziehst du einen Rahmen von links nach rechts, werden vollständig umschlossene Formen gewählt, von rechts nach links alle berührten.

- **Objektfang** (*Ansicht → Objektfang*) rastet an Knoten, Mittelpunkten und Kanten ein; Alt gedrückt halten schaltet ihn kurz aus.
- **Maße eingeben**: Position und Größe der Auswahl stehen oben in der Werkzeugleiste und lassen sich direkt ändern.
- **Knoten bearbeiten** (N): Knoten und Kurvengriffe ziehen, Doppelklick fügt einen Knoten ein, Entf löscht ihn.
- **Trimmen**: Ein Klick entfernt ein Linienstück bis zu den nächsten Schnittpunkten.
- **Messen** (M): Abstand und Winkel zwischen zwei Punkten; die Linie dient auch als Achse für *An Linie spiegeln*.

## Text, QR- und Barcodes

Das Textwerkzeug (T) nutzt alle installierten Schriften sowie eingebaute Einlinienschriften, die sich besonders schnell gravieren lassen. Texte bleiben bearbeitbar. Einstellbar sind unter anderem Höhe, Zeichenabstand, Breite in Prozent und Großbuchstaben. Mit *Anordnen → Text auf Pfad…* läuft ein Text entlang einer Form.

**Variabler Text** setzt beim Lasern Werte ein: `{n}` für eine Seriennummer, `{datum}` und `{zeit}`, dazu Spalten aus einer CSV-Datenliste.

Das Werkzeug QR-/Barcode (Q) erzeugt QR-Codes, Data Matrix, Code 128/39, EAN-13/-8 und UPC-A, auf Wunsch invertiert für eloxiertes Aluminium.

## Ebenen und Bearbeitungsarten

Jede Form gehört zu einer der 30 Farbebenen. Klicke unten in der Farbleiste auf eine Farbe, um die Auswahl dorthin zu legen. Für jede Ebene stellst du ein:

- **Modus**: *Linie* (Kontur schneiden oder ritzen), *Füllen* (Fläche zeilenweise gravieren), *Versatz-Füllen* (Fläche in Bahnen entlang der Kontur) oder *Bild*.
- **Geschwindigkeit**, **Leistung max./min.** und **Durchgänge**.
- Bei *Linie*: Schnittbreite (Kerf-Ausgleich), Laschen, Überschnitt, Einlauf und Perforieren.
- Bei *Füllen*: Zeilenabstand, Winkel, Kreuzschraffur, Overscan und Winkelschritt je Durchgang.
- **Unterebenen**: dieselben Formen mehrfach bearbeiten, z. B. erst füllen, dann die Kontur schneiden.

Ebenen lassen sich ausblenden, sperren und in ihrer Reihenfolge verschieben. Einzelnen Objekten kannst du eine eigene Leistung in Prozent der Ebenenleistung geben.

## Bilder gravieren

Importierte Fotos liegen auf einer Bild-Ebene. Wähle das Verfahren:

- *Schwellwert*, *Floyd-Steinberg*, *Jarvis-Judice-Ninke*, *Stucki*, *Atkinson*, *Burkes*, *Sierra* und *Geordnet*: Punkte an oder aus. Bei Holz und Leder sind Jarvis oder Stucki ein guter Anfang.
- *Graustufen*: Die Leistung folgt der Helligkeit.
- *Halbton* und *Linienraster*: grafische Raster mit einstellbarer Weite und Winkel.
- *Skizze*: betont Kanten wie eine Bleistiftzeichnung.
- *3D-Schichten*: Tiefe über mehrere Durchgänge.
- *Durchreichen*: das Bild unverändert lasern, wenn es schon fertig schwarz-weiß ist.

Helligkeit, Kontrast, Gamma, Entrauschen und Schärfen findest du in den Bildeinstellungen. *Anordnen → Bild mit Form zuschneiden* begrenzt ein Bild auf eine beliebige Form. *Bild nachzeichnen…* macht aus einem Bild Vektorformen, auch als Mittellinie.

**3D-Modelle** (STL, OBJ) importierst du wie Bilder. Wähle im Dialog die Ansicht, die Breite und die Auflösung. VectorLume legt daraus ein Tiefenbild an: Hohe Stellen werden hell, tiefe dunkel. Mit dem Bildmodus *Graustufen* oder *3D-Schichten* wird daraus ein Relief. *Invertieren* kehrt das um, z. B. für Stempel und Gussformen.

## Anordnen und Bearbeiten

Im Menü *Anordnen* findest du:

- **Gruppieren**, **Zerlegen** und **Automatisch gruppieren**. Letzteres fasst Teile mit ihren Löchern zusammen, z. B. nach einem DXF-Import.
- **Ausrichten**, **Verteilen**, **Gleiche Größe**, **Reihenfolge**, **Andocken**, **Spiegeln** und **Drehen**.
- **Auf Arbeitsfläche zentrieren** und **An Arbeitsfläche schieben** (Ecken, Kantenmitten, Mitte).
- **Boolesche Operationen**, **Verschweißen** und **Versatz**.
- **Bereinigen**: offene Pfade verbinden und doppelte Linien löschen.
- **Formen vereinfachen**, **Ecken abrunden**, **Schlitze anpassen**, **Verzerren** und **Schablonenstege**.
- **Verschachteln**: Teile platzsparend auf die Arbeitsfläche oder ein Reststück legen.
- **Reihen-Kopien**: als Raster, im Kreis oder entlang eines Pfades.

*Bearbeiten → Bearbeitungsverlauf…* listet die letzten 200 Schritte mit ihrer Änderung (z. B. „Verschoben: Kreis“ oder „Ebene gewechselt: 3 Objekte“). Ein Klick auf einen Eintrag stellt diesen Stand wieder her. Spätere Schritte bleiben in der Liste, bis du etwas Neues änderst.

## Generatoren

Unter *Anordnen → Generatoren*:

- **Box-Generator**: Kisten mit Fingerzinken aus Maßen, Materialstärke und Zinkenbreite.
- **Biegescharnier**: versetzte Schnitte, die Holz biegsam machen. Zuerst die Fläche als Form auswählen.
- **Puzzle**: Puzzleschnitte, z. B. über einem gravierten Foto. Ist beim Öffnen etwas ausgewählt, übernimmt das Puzzle dessen Lage und Größe.
- **Zahnräder**: Evolventen-Zahnräder nach Modul und Zähnezahl, auf Wunsch mit passendem Gegenrad. Der Achsabstand wird angezeigt.
- **Tangentialkreise**: Wähle drei Kreise aus. VectorLume zeigt alle Kreise, die alle drei berühren, als helle Vorschläge (bis zu acht). Ein Klick übernimmt einen, mit Umschalt mehrere, Esc beendet.
- **Schnittbreite ermitteln**: Testkamm schneiden, messen, Ergebnis auf die Ebene übernehmen (siehe unten).

## Material einstellen

**Materialtest** (*Laser → Materialtest…*): ein beschriftetes Raster aus Feldern mit gestaffelten Werten, z. B. Leistung × Geschwindigkeit. Das beste Feld speicherst du direkt in der **Materialbibliothek** und wendest es später auf jede Ebene an.

**Schnittbreite ermitteln** (*Anordnen → Generatoren*): Für passgenaue Teile, etwa Boxen mit Zinken, braucht die Schneid-Ebene die Schnittbreite des Lasers.

1. Testkamm erzeugen und mit den späteren Schneidwerten schneiden.
2. Streifen herausnehmen, dicht zusammenschieben und quer messen (oder den Spalt im Rahmen messen).
3. Messwert eingeben und **Auf Ebene … übernehmen** klicken.

## Lasern

1. **Vorschau** (Alt+P): zeigt den fertigen Job mit Laufzeit, Leerfahrten und Abspielregler.
2. **Rahmen fahren**: Der Laser fährt das umgebende Rechteck ab (Laser aus), damit du die Lage auf dem Material prüfen kannst. *Umriss-Rahmen* fährt die Hülle der Formen ab.
3. **Job starten**. Pause und Stopp sind jederzeit möglich; Stopp schaltet den Laser sofort aus.

**Start von** legt fest, wohin der Job kommt:

- *Absolute Koordinaten*: wie auf der Arbeitsfläche gezeichnet.
- *Benutzer-Ursprung*: relativ zu einem gespeicherten Punkt.
- *Aktuelle Position*: relativ zur Position des Laserkopfs.

Mit dem **Ursprung** (9 Ankerpunkte) bestimmst du, welche Ecke des Jobs auf diesem Punkt liegt. Im Reiter **Bewegen** fährst du den Kopf von Hand. **Laser zur Auswahl fahren** und **Mittelpunkt finden** helfen beim Ausrichten auf vorhandenen Werkstücken.

Unter den Job-Einstellungen findest du außerdem: Schnittreihenfolge optimieren, innen vor außen schneiden, an Ecken beginnen und überlappende Linien entfernen (gemeinsame Kanten nur einmal schneiden).

Fertige G-Code-Dateien anderer Programme sendest du mit *Laser → G-Code-Datei senden…*; eigene Jobs speicherst du mit *Datei → G-Code speichern…*.

## Kamera, Drucken & Schneiden, Drehachse

- **Kamera** (*Laser → Kamera…*): Eine Webcam oder ein Foto der Arbeitsfläche wird über vier Passpunkte entzerrt und unter den Entwurf gelegt.
- **Drucken & Schneiden**: Den Entwurf über zwei mitgedruckte Passermarken genau auf bedrucktes Material ausrichten.
- **Drehachse**: Rollen oder Futter für Gläser und Becher, angeschlossen statt des Y-Motors oder als A-Achse.

## GRBL-Parameter

*Laser → GRBL-Parameter…* zeigt alle Einstellungen der Steuerung (`$$`) mit Beschreibung, zum Beispiel Verfahrweg (`$130`, `$131`), Höchstgeschwindigkeit und Beschleunigung.

1. Vor jeder Änderung mit **Sichern…** eine Sicherung als Textdatei anlegen.
2. Geänderte Werte sind blau, ungültige rot umrandet.
3. **Schreiben** sendet nur die geänderten Werte und liest danach neu ein.

**Laden…** übernimmt eine Sicherung, die du anschließend schreibst.

> Falsche Werte, z. B. bei den Schritten je mm, lassen die Maschine falsch oder gegen den Anschlag fahren. Im Zweifel nichts ändern.

## Dateien

- **Öffnen und Importieren**: VectorLume-Projekte (`.vlume`), LightBurn-Projekte (`.lbrn2`), SVG, DXF, PDF/AI, EPS (mit installiertem Ghostscript), HPGL/PLT sowie Bilder (PNG, JPG, BMP, GIF, WEBP, TIFF). Dateien lassen sich auch einfach auf das Fenster ziehen.
- **Leiterplatten**: Gerber-Dateien (Kupfer, Lötstopp, Bestückungsdruck) werden zu gefüllten Flächen, der Platinenumriss zur Schneidlinie. Excellon-Bohrdateien liefern die Bohrungen als Kreise. Importierst du mehrere Dateien desselben Boards nacheinander, liegen sie deckungsgleich übereinander. Die Unterseite musst du zum Lasern selbst spiegeln.
- **Exportieren**: G-Code, SVG, DXF und PDF (1:1 zum Drucken).
- **Autosicherung**: Ungespeicherte Arbeit wird regelmäßig gesichert und nach einem Absturz beim nächsten Start angeboten.

## Tastenkürzel

- Strg+N neu, Strg+O öffnen, Strg+S speichern, Strg+Umschalt+S speichern unter
- Strg+I importieren, Strg+E G-Code speichern
- Strg+Z rückgängig, Strg+Y wiederholen
- Strg+C, Strg+X, Strg+V kopieren, ausschneiden, einfügen; Strg+D duplizieren
- Strg+A alles auswählen, Entf löschen
- Strg+G gruppieren, Strg+Umschalt+G Gruppierung lösen
- Bild↑/Bild↓ Reihenfolge ändern (mit Strg ganz nach vorne/hinten)
- Pfeiltasten verschieben um 1 mm, mit Umschalt um 10 mm
- F oder Strg+0 Arbeitsfläche einpassen; Alt+P Vorschau; F1 dieses Handbuch
- Werkzeuge: V Auswählen, R Rechteck, E Ellipse, L Linie, T Text, Q QR-/Barcode, N Knoten, M Messen

## Probleme lösen

- **Kein Anschluss in der Liste**: USB-Kabel und Treiber prüfen; manche Kabel laden nur und übertragen keine Daten.
- **Der Laser brennt Linien zwischen den Formen**: Der Laser-Modus ist aus. `$32=1` senden (VectorLume bietet das beim Verbinden an).
- **Alarm nach dem Einschalten**: Referenzfahrt ausführen oder die Sperre in der Konsole mit `$X` lösen.
- **Der Job liegt an der falschen Stelle**: Nullpunkt-Ecke in den Maschineneinstellungen und *Start von* prüfen; vor dem Start den Rahmen fahren.
- **Gravur zu dunkel oder zu hell**: Materialtest machen; bei Bildern Zeilenabstand (dpi) und Bildeinstellungen anpassen.
- **Teile fallen nicht heraus**: Durchgänge oder Leistung erhöhen, Fokus prüfen, Laschen entfernen.
- **Programmfehler**: VectorLume schreibt einen Fehlerbericht (`crash.log` im Einstellungsordner) und zeigt ihn beim nächsten Start an. Bitte bei Fehlermeldungen mitschicken.
