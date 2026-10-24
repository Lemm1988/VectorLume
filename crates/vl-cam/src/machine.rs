//! Maschinenprofil und Job-Einstellungen sowie die Umrechnung Dokument ↔ Maschine.

use serde::{Deserialize, Serialize};
use vl_core::{Point, Rect};

/// Lage des Maschinen-Nullpunkts, von oben auf die Maschine gesehen (hinten = oben).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OriginCorner {
    #[default]
    FrontLeft,
    RearLeft,
    FrontRight,
    RearRight,
}

impl OriginCorner {
    pub const ALL: [OriginCorner; 4] =
        [OriginCorner::FrontLeft, OriginCorner::RearLeft, OriginCorner::FrontRight, OriginCorner::RearRight];

    pub fn label(self) -> &'static str {
        match self {
            OriginCorner::FrontLeft => "vorne links",
            OriginCorner::RearLeft => "hinten links",
            OriginCorner::FrontRight => "vorne rechts",
            OriginCorner::RearRight => "hinten rechts",
        }
    }

    fn mirror_x(self) -> bool {
        matches!(self, OriginCorner::FrontRight | OriginCorner::RearRight)
    }

    fn mirror_y(self) -> bool {
        matches!(self, OriginCorner::FrontLeft | OriginCorner::FrontRight)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MachineProfile {
    pub name: String,
    pub bed_width: f64,
    pub bed_height: f64,
    pub origin: OriginCorner,
    /// Wert von `$30` (Maximalwert für S).
    pub s_max: f64,
    /// M4 (Leistung folgt der Geschwindigkeit) statt M3.
    pub dynamic_power: bool,
    /// Eilgang in mm/min (`$110`/`$111`), nur für die Zeitschätzung.
    pub rapid_rate: f64,
    /// Beschleunigung in mm/s² (`$120`/`$121`), nur für die Zeitschätzung.
    pub acceleration: f64,
    pub air_assist_supported: bool,
    /// Steuerbare Z-Achse (Fokus bzw. Tischhöhe per G-Code).
    pub z_axis: bool,
    pub air_on: String,
    pub air_off: String,
    pub baud: u32,
    /// Größe des GRBL-Empfangspuffers in Byte.
    pub rx_buffer: usize,
    pub return_to_origin: bool,
    /// Leistung beim Rahmenfahren in Prozent (0 = Laser aus).
    pub frame_power: f64,
    pub frame_speed: f64,
    pub jog_speed: f64,
}

impl Default for MachineProfile {
    fn default() -> Self {
        Self::sculpfun_sf_a9()
    }
}

impl MachineProfile {
    pub fn sculpfun_sf_a9() -> Self {
        Self {
            name: "Sculpfun SF-A9 40W".into(),
            bed_width: 400.0,
            bed_height: 400.0,
            origin: OriginCorner::FrontLeft,
            s_max: 1000.0,
            dynamic_power: true,
            rapid_rate: 6000.0,
            acceleration: 1000.0,
            air_assist_supported: true,
            z_axis: false,
            air_on: "M8".into(),
            air_off: "M9".into(),
            baud: 115_200,
            rx_buffer: 127,
            return_to_origin: true,
            frame_power: 0.0,
            frame_speed: 3000.0,
            jog_speed: 3000.0,
        }
    }

    pub fn bed_rect(&self) -> Rect {
        Rect::from_xywh(0.0, 0.0, self.bed_width, self.bed_height)
    }

    /// Dokument → Maschine. Die Abbildung ist ihre eigene Umkehrung.
    pub fn doc_to_machine(&self, p: Point) -> Point {
        Point::new(
            if self.origin.mirror_x() { self.bed_width - p.x } else { p.x },
            if self.origin.mirror_y() { self.bed_height - p.y } else { p.y },
        )
    }

    pub fn machine_to_doc(&self, p: Point) -> Point {
        self.doc_to_machine(p)
    }

    /// Richtung auf dem Bildschirm (rechts/unten positiv) → Achsrichtung der Maschine.
    pub fn screen_to_machine_delta(&self, d: Point) -> Point {
        Point::new(if self.origin.mirror_x() { -d.x } else { d.x }, if self.origin.mirror_y() { -d.y } else { d.y })
    }

    pub fn doc_rect_to_machine(&self, r: &Rect) -> Rect {
        Rect::from_points(self.doc_to_machine(r.min), self.doc_to_machine(r.max))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StartFrom {
    /// Maschinenkoordinaten wie in der Arbeitsfläche angezeigt (Referenzfahrt nötig).
    #[default]
    Absolute,
    /// Ankerpunkt des Jobs liegt auf dem gespeicherten Benutzer-Ursprung.
    UserOrigin,
    /// Ankerpunkt des Jobs liegt dort, wo der Laserkopf gerade steht.
    CurrentPosition,
}

impl StartFrom {
    pub const ALL: [StartFrom; 3] = [StartFrom::Absolute, StartFrom::UserOrigin, StartFrom::CurrentPosition];

    pub fn label(self) -> &'static str {
        match self {
            StartFrom::Absolute => "Absolute Koordinaten",
            StartFrom::UserOrigin => "Benutzer-Ursprung",
            StartFrom::CurrentPosition => "Aktuelle Position",
        }
    }
}

/// Ankerpunkt des Jobs (bezogen auf die Bildschirmansicht).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    #[default]
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    pub const GRID: [[Anchor; 3]; 3] = [
        [Anchor::TopLeft, Anchor::Top, Anchor::TopRight],
        [Anchor::Left, Anchor::Center, Anchor::Right],
        [Anchor::BottomLeft, Anchor::Bottom, Anchor::BottomRight],
    ];

    pub fn label(self) -> &'static str {
        match self {
            Anchor::TopLeft => "Oben links",
            Anchor::Top => "Oben Mitte",
            Anchor::TopRight => "Oben rechts",
            Anchor::Left => "Links Mitte",
            Anchor::Center => "Mitte",
            Anchor::Right => "Rechts Mitte",
            Anchor::BottomLeft => "Unten links",
            Anchor::Bottom => "Unten Mitte",
            Anchor::BottomRight => "Unten rechts",
        }
    }

    pub fn point_in(self, r: &Rect) -> Point {
        let (fx, fy) = match self {
            Anchor::TopLeft => (0.0, 0.0),
            Anchor::Top => (0.5, 0.0),
            Anchor::TopRight => (1.0, 0.0),
            Anchor::Left => (0.0, 0.5),
            Anchor::Center => (0.5, 0.5),
            Anchor::Right => (1.0, 0.5),
            Anchor::BottomLeft => (0.0, 1.0),
            Anchor::Bottom => (0.5, 1.0),
            Anchor::BottomRight => (1.0, 1.0),
        };
        Point::new(r.min.x + fx * r.width(), r.min.y + fy * r.height())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JobSettings {
    pub start_from: StartFrom,
    pub anchor: Anchor,
    /// Benutzer-Ursprung in Maschinenkoordinaten.
    pub user_origin: Point,
    pub selected_only: bool,
    pub optimize_order: bool,
    pub inner_first: bool,
    /// Bild- und Füllebenen vor Linienebenen ausgeben, damit ausgeschnittene Teile nicht vorher herausfallen.
    pub engrave_first: bool,
}

impl Default for JobSettings {
    fn default() -> Self {
        Self {
            start_from: StartFrom::Absolute,
            anchor: Anchor::BottomLeft,
            user_origin: Point::ZERO,
            selected_only: false,
            optimize_order: true,
            inner_first: true,
            engrave_first: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping_is_involutive_for_all_corners() {
        let mut p = MachineProfile { bed_width: 300.0, bed_height: 200.0, ..Default::default() };
        for c in OriginCorner::ALL {
            p.origin = c;
            let q = Point::new(12.0, 34.0);
            assert_eq!(p.machine_to_doc(p.doc_to_machine(q)), q);
        }
        p.origin = OriginCorner::FrontLeft;
        // Hintere linke Ecke im Dokument ist bei Nullpunkt vorne links Y = Bettiefe.
        assert_eq!(p.doc_to_machine(Point::ZERO), Point::new(0.0, 200.0));
        assert_eq!(p.screen_to_machine_delta(Point::new(0.0, -1.0)), Point::new(0.0, 1.0));
    }
}
