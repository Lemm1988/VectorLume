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
