//! Maßeinheit der Oberfläche: intern rechnet alles in Millimetern, angezeigt und eingegeben wird in mm oder Zoll.
//! Eingabefelder bekommen die Einheit über [`UnitField::mm`] bzw. [`UnitField::mm_per_min`]; dort lassen sich
//! Werte auch ausdrücklich mit Einheit eingeben („10 mm“, „0.5 in“, „1/4"“).

use egui::{DragValue, Slider};
use serde::{Deserialize, Serialize};
use std::cell::Cell;

pub const MM_PER_INCH: f64 = 25.4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Units {
    #[default]
    Mm,
    Inch,
}

impl Units {
    pub const ALL: [Units; 2] = [Units::Mm, Units::Inch];

    /// Übersetzter Name.
    pub fn label(self) -> &'static str {
        match self {
            Units::Mm => crate::i18n::tr("Millimeter"),
            Units::Inch => crate::i18n::tr("Zoll"),
        }
    }

    /// Zoll für die USA (wie dort üblich), sonst Millimeter.
    pub fn from_system() -> Self {
        match sys_locale::get_locale() {
            Some(l) if l.to_ascii_uppercase().ends_with("-US") => Units::Inch,
            _ => Units::Mm,
        }
    }
}

thread_local! {
    // Je Thread: die Oberfläche läuft in einem, und parallele Tests stören sich so nicht.
    static UNITS: Cell<Units> = const { Cell::new(Units::Mm) };
}

pub fn set_units(u: Units) {
    UNITS.with(|c| c.set(u));
}

pub fn units() -> Units {
    UNITS.with(|c| c.get())
}

/// Millimeter je Anzeigeeinheit.
pub fn per_unit() -> f64 {
    match units() {
        Units::Mm => 1.0,
        Units::Inch => MM_PER_INCH,
    }
}

/// Kurzzeichen der Längeneinheit.
pub fn unit() -> &'static str {
    match units() {
        Units::Mm => "mm",
        Units::Inch => "in",
    }
}

/// Zahl in der Anzeigeeinheit ohne Einheit; `decimals` gilt für mm, in Zoll eine Stelle mehr.
pub fn num(mm: f64, decimals: usize) -> String {
    match units() {
        Units::Mm => format!("{mm:.decimals$}"),
        Units::Inch => format!("{:.*}", decimals + 1, mm / MM_PER_INCH),
    }
}

/// Länge mit Einheit, z. B. „12.50 mm“ bzw. „0.492 in“.
pub fn len(mm: f64, decimals: usize) -> String {
    format!("{} {}", num(mm, decimals), unit())
}

/// Geschwindigkeit mit Einheit (mm/min bzw. in/min).
pub fn speed(mm_per_min: f64) -> String {
    match units() {
        Units::Mm => format!("{mm_per_min:.0} mm/min"),
        Units::Inch => format!("{:.1} in/min", mm_per_min / MM_PER_INCH),
    }
}

/// Liest eine Länge; ohne Einheit gilt die eingestellte. Erlaubt Komma, „mm“, „cm“, „in“, `"` und Brüche („1/4“).
pub fn parse_len(s: &str) -> Option<f64> {
    let t = s.trim().to_ascii_lowercase().replace(',', ".");
    let (value, factor) = if let Some(v) = t.strip_suffix("mm") {
        (v, 1.0)
    } else if let Some(v) = t.strip_suffix("cm") {
        (v, 10.0)
    } else if let Some(v) = t.strip_suffix("in").or_else(|| t.strip_suffix('"')).or_else(|| t.strip_suffix('″')) {
        (v, MM_PER_INCH)
    } else {
        (t.as_str(), per_unit())
    };
    parse_number(value.trim()).map(|v| v * factor)
}

/// Zahl, auch als Bruch „3/8“ oder gemischt „1 1/2“.
fn parse_number(s: &str) -> Option<f64> {
    let frac = |f: &str| {
        let (a, b) = f.split_once('/')?;
        let (a, b) = (a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?);
        (b != 0.0).then(|| a / b)
    };
    if let Some((whole, f)) = s.split_once(' ')
        && f.contains('/')
    {
        let w: f64 = whole.trim().parse().ok()?;
        let f = frac(f)?;
        return Some(if w < 0.0 { w - f } else { w + f });
    }
    if s.contains('/') {
        return frac(s);
    }
    s.parse().ok()
}

/// Wie [`parse_len`], „aus“ bzw. „off“ ergibt 0.
pub fn parse_len_or_off(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("aus") || t.eq_ignore_ascii_case("off") {
        return Some(0.0);
    }
    parse_len(t)
}

/// Beschriftung des Fangrasters.
pub fn snap_grid_label(mm_setting: f64) -> String {
    match units() {
        Units::Mm => format!("{} mm", trim(mm_setting, 2)),
        Units::Inch => "1/16 in".into(),
    }
}

/// Geschwindigkeit lesen (mm/min bzw. in/min, Einheit wahlweise angegeben).
pub fn parse_speed(s: &str) -> Option<f64> {
    let t = s.trim().to_ascii_lowercase().replace(',', ".");
    let (value, factor) = if let Some(v) = t.strip_suffix("mm/min") {
        (v, 1.0)
    } else if let Some(v) = t.strip_suffix("in/min") {
        (v, MM_PER_INCH)
    } else {
        (t.as_str(), per_unit())
    };
    value.trim().parse::<f64>().ok().map(|v| v * factor)
}

/// Runder Schritt in der Anzeigeeinheit (für Lineale und Raster), in mm, mindestens `min_mm`.
pub fn nice_step(min_mm: f64) -> f64 {
    let steps: &[f64] = match units() {
        Units::Mm => &[0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0],
        Units::Inch => &[0.01, 0.02, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0],
    };
    let u = per_unit();
    steps.iter().map(|s| s * u).find(|&s| s >= min_mm).unwrap_or(steps[steps.len() - 1] * u)
}

/// Unterteilung eines Linealschritts: mm in Fünfteln, Zoll in Vierteln.
pub fn subdivisions() -> i64 {
    match units() {
        Units::Mm => 5,
        Units::Inch => 4,
    }
}

/// Fangraster in mm: 1 mm bzw. 1/16 Zoll.
pub fn snap_grid(mm_setting: f64) -> f64 {
    match units() {
        Units::Mm => mm_setting.max(0.01),
        Units::Inch => MM_PER_INCH / 16.0,
    }
}

/// Schrittweiten zum Verfahren in mm.
pub fn jog_steps() -> Vec<f64> {
    match units() {
        Units::Mm => vec![0.1, 1.0, 5.0, 10.0, 50.0],
        Units::Inch => [0.01, 0.1, 0.25, 0.5, 2.0].iter().map(|v| v * MM_PER_INCH).collect(),
    }
}

/// Zahl ohne überflüssige Nullen (Lineale, Schrittweiten).
pub fn trim(v: f64, max_decimals: usize) -> String {
    let s = format!("{v:.max_decimals$}");
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// Eingabefelder mit Einheit: zeigen und lesen in mm oder Zoll, speichern immer mm.
pub trait UnitField: Sized {
    fn formatter(self, f: impl Fn(f64) -> String + 'static) -> Self;
    fn parser(self, p: impl Fn(&str) -> Option<f64> + 'static) -> Self;
    fn unit_suffix(self, s: &str) -> Self;

    /// Länge; in mm wie bisher, in Zoll mit drei Nachkommastellen.
    fn mm(self) -> Self {
        match units() {
            Units::Mm => self.unit_suffix(" mm").parser(parse_len),
            Units::Inch => self.formatter(|v| format!("{:.3}", v / MM_PER_INCH)).parser(parse_len).unit_suffix(" in"),
        }
    }

    /// Geschwindigkeit in mm/min bzw. in/min.
    fn mm_per_min(self) -> Self {
        match units() {
            Units::Mm => self.unit_suffix(" mm/min").parser(parse_speed),
            Units::Inch => {
                self.formatter(|v| format!("{:.1}", v / MM_PER_INCH)).parser(parse_speed).unit_suffix(" in/min")
            }
        }
    }
}

impl UnitField for DragValue<'_> {
    fn formatter(self, f: impl Fn(f64) -> String + 'static) -> Self {
        self.custom_formatter(move |v, _| f(v))
    }
    fn parser(self, p: impl Fn(&str) -> Option<f64> + 'static) -> Self {
        self.custom_parser(p)
    }
    fn unit_suffix(self, s: &str) -> Self {
        self.suffix(s)
    }
}

impl UnitField for Slider<'_> {
    fn formatter(self, f: impl Fn(f64) -> String + 'static) -> Self {
        self.custom_formatter(move |v, _| f(v))
    }
    fn parser(self, p: impl Fn(&str) -> Option<f64> + 'static) -> Self {
        self.custom_parser(p)
    }
    fn unit_suffix(self, s: &str) -> Self {
        self.suffix(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lengths_with_and_without_units() {
        set_units(Units::Mm);
        assert_eq!(parse_len("12,5"), Some(12.5));
        assert_eq!(parse_len("1 in"), Some(25.4));
        assert_eq!(parse_len("2cm"), Some(20.0));
        assert_eq!(len(12.5, 2), "12.50 mm");
        set_units(Units::Inch);
        assert_eq!(parse_len("1/4"), Some(6.35));
        assert!((parse_len("1 1/2\"").unwrap() - 38.1).abs() < 1e-9);
        assert_eq!(parse_len("10 mm"), Some(10.0));
        assert_eq!(parse_speed("100"), Some(2540.0));
        assert_eq!(len(25.4, 2), "1.000 in");
        assert_eq!(speed(2540.0), "100.0 in/min");
        assert!((nice_step(20.0) - 25.4).abs() < 1e-9, "1 Zoll");
        assert_eq!(parse_len("abc"), None);
        set_units(Units::Mm);
        assert_eq!(trim(2.50, 2), "2.5");
        assert_eq!(trim(10.0, 1), "10");
    }
}
