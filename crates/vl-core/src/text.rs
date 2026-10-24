//! Textdaten an Objekten: Schriftangaben, Platzhalter für variablen Text (Zähler, Datum, Uhrzeit,
//! Spalten einer Datenliste) und Text entlang eines Pfades. Die Umwandlung in Umrisse übernimmt `vl-io`,
//! weil dafür Schriften nötig sind.

use crate::geom::Point;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextSpec {
    pub text: String,
    pub family: String,
    pub bold: bool,
    pub italic: bool,
    /// Höhe der Großbuchstaben in mm.
    pub height_mm: f64,
    /// Zusätzlicher Zeichenabstand in mm.
    pub letter_spacing: f64,
    /// Zeilenabstand als Faktor der Schrifthöhe.
    pub line_spacing: f64,
    pub align: TextAlign,
}

impl Default for TextSpec {
    fn default() -> Self {
        Self {
            text: "VectorLume".into(),
            family: "Arial".into(),
            bold: false,
            italic: false,
            height_mm: 10.0,
            letter_spacing: 0.0,
            line_spacing: 1.0,
            align: TextAlign::Left,
        }
    }
}

/// Zähler für den Platzhalter `{n}` und Datenliste für Serienbriefe.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextVars {
    pub counter: i64,
    pub step: i64,
    /// Mindestanzahl Stellen (mit führenden Nullen).
    pub digits: u32,
    /// Nach jedem erfolgreich beendeten Job um `step` erhöhen bzw. zur nächsten Datenzeile gehen.
    pub advance_after_job: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<DataList>,
}

impl Default for TextVars {
    fn default() -> Self {
        Self { counter: 1, step: 1, digits: 3, advance_after_job: true, data: None }
    }
}

/// Datenliste (z. B. aus einer CSV-Datei). Jede Spalte ist ein Platzhalter `{Spaltenname}`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DataList {
    /// Dateiname, nur zur Anzeige.
    pub source: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// Aktuelle Zeile (0-basiert).
    pub row: usize,
}

impl DataList {
    pub fn placeholder(header: &str) -> String {
        format!("{{{header}}}")
    }

    /// Wert der Spalte in der aktuellen Zeile.
    pub fn value(&self, column: usize) -> &str {
        let row = self.row.min(self.rows.len().saturating_sub(1));
        self.rows.get(row).and_then(|r| r.get(column)).map(String::as_str).unwrap_or("")
    }

    pub fn is_last_row(&self) -> bool {
        self.row + 1 >= self.rows.len()
    }
}

/// Text entlang eines Pfades (Polylinie in Dokumentkoordinaten).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextOnPath {
    pub points: Vec<Point>,
    pub closed: bool,
    /// Verschiebung entlang des Pfades in mm.
    pub offset: f64,
    /// Abstand der Grundlinie vom Pfad in mm (positiv = links der Laufrichtung, bei Laufrichtung nach rechts oben).
    pub distance: f64,
    /// Laufrichtung umkehren (Text auf der anderen Seite, z. B. unten in einem Kreis).
    pub reverse: bool,
}

impl Default for TextOnPath {
    fn default() -> Self {
        Self { points: Vec::new(), closed: false, offset: 0.0, distance: 0.0, reverse: false }
    }
}

/// Strichcode-Arten: 2D-Codes aus quadratischen Modulen, 1D-Codes aus senkrechten Strichen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BarcodeKind {
    #[default]
    Qr,
    DataMatrix,
    Code128,
    Code39,
    Ean13,
    Ean8,
    UpcA,
}

impl BarcodeKind {
    pub const ALL: [BarcodeKind; 7] = [
        BarcodeKind::Qr,
        BarcodeKind::DataMatrix,
        BarcodeKind::Code128,
        BarcodeKind::Code39,
        BarcodeKind::Ean13,
        BarcodeKind::Ean8,
        BarcodeKind::UpcA,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BarcodeKind::Qr => "QR-Code",
            BarcodeKind::DataMatrix => "Data Matrix",
            BarcodeKind::Code128 => "Code 128",
            BarcodeKind::Code39 => "Code 39",
            BarcodeKind::Ean13 => "EAN-13",
            BarcodeKind::Ean8 => "EAN-8",
            BarcodeKind::UpcA => "UPC-A",
        }
    }

    /// Erlaubter Inhalt (für die Hilfe im Dialog).
    pub fn hint(self) -> &'static str {
        match self {
            BarcodeKind::Qr => "Beliebiger Text, Internetadresse oder WLAN-Zugang",
            BarcodeKind::DataMatrix => "Beliebiger Text, sehr kompakt (z. B. für Teilekennzeichnung)",
            BarcodeKind::Code128 => "Buchstaben, Ziffern und Satzzeichen (ASCII)",
            BarcodeKind::Code39 => "Großbuchstaben, Ziffern und - . $ / + % Leerzeichen",
            BarcodeKind::Ean13 => "12 Ziffern (Prüfziffer wird ergänzt) oder 13 Ziffern",
            BarcodeKind::Ean8 => "7 Ziffern (Prüfziffer wird ergänzt) oder 8 Ziffern",
            BarcodeKind::UpcA => "11 Ziffern (Prüfziffer wird ergänzt) oder 12 Ziffern",
        }
    }

    pub fn is_2d(self) -> bool {
        matches!(self, BarcodeKind::Qr | BarcodeKind::DataMatrix)
    }

    /// Vorgeschriebene Ruhezone in Modulen (freier Rand, beim Invertieren mitgraviert).
    pub fn quiet_zone(self) -> u32 {
        match self {
            BarcodeKind::Qr => 4,
            BarcodeKind::DataMatrix => 1,
            _ => 10,
        }
    }
}

/// Fehlerkorrektur eines QR-Codes: Anteil der Fläche, der beschädigt sein darf.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum QrLevel {
    L,
    #[default]
    M,
    Q,
    H,
}

impl QrLevel {
    pub const ALL: [QrLevel; 4] = [QrLevel::L, QrLevel::M, QrLevel::Q, QrLevel::H];

    pub fn label(self) -> &'static str {
        match self {
            QrLevel::L => "L (7 %)",
            QrLevel::M => "M (15 %)",
            QrLevel::Q => "Q (25 %)",
            QrLevel::H => "H (30 %)",
        }
    }
}

/// Einstellungen eines Strichcodes. Der Inhalt steht in `TextData::spec.text` (mit Platzhaltern),
/// die Schrift dort wird für den Klartext unter 1D-Codes verwendet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BarcodeSpec {
    pub kind: BarcodeKind,
    /// Breite eines Moduls (QR-Punkt bzw. schmalster Strich) in mm.
    pub module_mm: f64,
    /// Strichhöhe bei 1D-Codes in mm.
    pub height_mm: f64,
    pub level: QrLevel,
    /// Klartext unter 1D-Codes.
    pub show_text: bool,
    /// Helle statt dunkle Module gravieren (für Material, das beim Lasern hell wird, z. B. eloxiertes Aluminium).
    pub invert: bool,
    /// Gravierte Flächen um diesen Betrag schmaler machen, weil der Laserpunkt über die Kontur hinaus brennt (mm).
    pub shrink_mm: f64,
}

impl Default for BarcodeSpec {
    fn default() -> Self {
        Self {
            kind: BarcodeKind::Qr,
            module_mm: 0.5,
            height_mm: 15.0,
            level: QrLevel::M,
            show_text: true,
            invert: false,
            shrink_mm: 0.0,
        }
    }
}

/// Alles, was nötig ist, um einen Text erneut zu erzeugen (Bearbeiten, variabler Text).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextData {
    /// `spec.text` ist die Vorlage, ggf. mit Platzhaltern.
    pub spec: TextSpec,
    pub vars: TextVars,
    pub on_path: Option<TextOnPath>,
    /// Zuletzt ausgegebener Text (nach Ersetzen der Platzhalter).
    pub rendered: String,
    /// Strichcode statt Schrift: der Text ist dann der Inhalt des Codes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub barcode: Option<BarcodeSpec>,
}

/// Datum und Uhrzeit für die Platzhalter (vom Aufrufer, damit der Kern keine Zeitzonen kennen muss).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Now {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

/// Unterstützte Platzhalter (für Hilfe und Schaltflächen).
pub const PLACEHOLDERS: [(&str, &str); 4] =
    [("{n}", "fortlaufende Nummer"), ("{datum}", "Datum TT.MM.JJJJ"), ("{zeit}", "Uhrzeit HH:MM"), ("{jahr}", "Jahr")];

/// Zeilennummer der Datenliste (1-basiert).
pub const ROW_PLACEHOLDER: &str = "{zeile}";

impl TextData {
    pub fn is_variable(&self) -> bool {
        PLACEHOLDERS.iter().any(|(p, _)| self.spec.text.contains(p)) || self.uses_data()
    }

    pub fn uses_counter(&self) -> bool {
        self.spec.text.contains("{n}")
    }

    /// Enthält Platzhalter der Datenliste.
    pub fn uses_data(&self) -> bool {
        self.vars.data.as_ref().is_some_and(|d| {
            self.spec.text.contains(ROW_PLACEHOLDER)
                || d.headers.iter().any(|h| self.spec.text.contains(&DataList::placeholder(h)))
        })
    }

    /// Vorlage mit ersetzten Platzhaltern.
    pub fn expand(&self, now: Now) -> String {
        let v = &self.vars;
        let n = if v.counter < 0 {
            format!("-{:0w$}", v.counter.unsigned_abs(), w = v.digits as usize)
        } else {
            format!("{:0w$}", v.counter, w = v.digits as usize)
        };
        let mut text = self
            .spec
            .text
            .replace("{n}", &n)
            .replace("{datum}", &format!("{:02}.{:02}.{:04}", now.day, now.month, now.year))
            .replace("{zeit}", &format!("{:02}:{:02}", now.hour, now.minute))
            .replace("{jahr}", &format!("{:04}", now.year));
        if let Some(d) = &v.data {
            text = text.replace(ROW_PLACEHOLDER, &(d.row + 1).to_string());
            for (i, h) in d.headers.iter().enumerate() {
                text = text.replace(&DataList::placeholder(h), d.value(i));
            }
        }
        text
    }

    /// Kurzer Objektname aus der ersten Zeile.
    pub fn display_name(&self) -> String {
        let first = self.spec.text.lines().next().unwrap_or("Text").chars().take(24).collect::<String>();
        if let Some(b) = &self.barcode {
            return format!("{} „{first}“", b.kind.label());
        }
        if self.on_path.is_some() { format!("Text auf Pfad „{first}“") } else { format!("Text „{first}“") }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_replaced() {
        let mut d = TextData::default();
        d.spec.text = "Nr. {n} vom {datum} {zeit} ({jahr}) {x}".into();
        d.vars.counter = 7;
        let now = Now { year: 2026, month: 3, day: 9, hour: 8, minute: 5 };
        assert!(d.is_variable() && d.uses_counter());
        assert_eq!(d.expand(now), "Nr. 007 vom 09.03.2026 08:05 (2026) {x}");
        d.vars.counter = -4;
        d.vars.digits = 2;
        assert!(d.expand(now).starts_with("Nr. -04"));
        d.spec.text = "fest".into();
        assert!(!d.is_variable());
    }

    #[test]
    fn barcode_data_roundtrip() {
        let d = TextData {
            barcode: Some(BarcodeSpec { kind: BarcodeKind::Ean13, invert: true, ..Default::default() }),
            ..Default::default()
        };
        let back: TextData = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(back, d);
        // ältere Projekte ohne Code bleiben unverändert
        assert!(!serde_json::to_string(&TextData::default()).unwrap().contains("barcode"));
        assert_eq!(d.display_name(), "EAN-13 „VectorLume“");
    }

    #[test]
    fn data_list_columns() {
        let mut d = TextData::default();
        d.vars.data = Some(DataList {
            source: "gaeste.csv".into(),
            headers: vec!["Name".into(), "Tisch".into()],
            rows: vec![vec!["Anna".into(), "3".into()], vec!["Ben".into()]],
            row: 0,
        });
        d.spec.text = "{Name} – Tisch {Tisch} ({zeile})".into();
        assert!(d.is_variable() && d.uses_data() && !d.uses_counter());
        assert_eq!(d.expand(Now::default()), "Anna – Tisch 3 (1)");
        d.vars.data.as_mut().unwrap().row = 1;
        // fehlende Zelle bleibt leer
        assert_eq!(d.expand(Now::default()), "Ben – Tisch  (2)");
        assert!(d.vars.data.as_ref().unwrap().is_last_row());
        d.spec.text = "{Ort}".into();
        assert!(!d.uses_data());
    }
}
