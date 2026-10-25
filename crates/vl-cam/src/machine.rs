//! Maschinenprofil und Job-Einstellungen sowie die Umrechnung Dokument ↔ Maschine.

use serde::{Deserialize, Serialize};
use std::f64::consts::PI;
use vl_core::{Firmware, Point, Rect};

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

/// Gemessener Versatz beim Gravieren mit einer Geschwindigkeit: Abstand der Kanten von hin- und zurückgefahrenen
/// Zeilen (Laser und Mechanik reagieren verzögert, die Zeilen landen in Fahrtrichtung zu weit).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScanOffset {
    /// mm/min
    pub speed: f64,
    /// mm
    pub offset: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MachineProfile {
    pub name: String,
    pub firmware: Firmware,
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
    pub rotary: Rotary,
    /// Scan-Versatz je Geschwindigkeit (leer = aus).
    pub scan_offsets: Vec<ScanOffset>,
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
            firmware: Firmware::Grbl,
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
            rotary: Rotary::default(),
            scan_offsets: Vec::new(),
        }
    }

    /// Verschiebung jeder Füll- bzw. Bildzeile gegen ihre Fahrtrichtung (halber gemessener Versatz). Zwischen den
    /// Messwerten wird linear interpoliert; außerhalb wächst der Versatz proportional zur Geschwindigkeit (feste
    /// Verzögerung).
    pub fn scan_shift(&self, speed: f64) -> f64 {
        let mut table: Vec<ScanOffset> = self.scan_offsets.iter().copied().filter(|s| s.speed > 0.0).collect();
        if table.is_empty() || speed <= 0.0 {
            return 0.0;
        }
        table.sort_by(|a, b| a.speed.total_cmp(&b.speed));
        let (first, last) = (table[0], table[table.len() - 1]);
        let offset = if speed <= first.speed {
            first.offset * speed / first.speed
        } else if speed >= last.speed {
            last.offset * speed / last.speed
        } else {
            let i = table.iter().position(|s| s.speed >= speed).unwrap_or(table.len() - 1);
            let (a, b) = (table[i - 1], table[i]);
            a.offset + (b.offset - a.offset) * (speed - a.speed) / (b.speed - a.speed)
        };
        offset / 2.0
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

/// Bauart der Drehachse.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RotaryKind {
    /// Werkstück liegt auf angetriebenen Rollen.
    #[default]
    Roller,
    /// Werkstück ist im Futter eingespannt.
    Chuck,
}

impl RotaryKind {
    pub const ALL: [RotaryKind; 2] = [RotaryKind::Roller, RotaryKind::Chuck];

    pub fn label(self) -> &'static str {
        match self {
            RotaryKind::Roller => "Rollen",
            RotaryKind::Chuck => "Futter",
        }
    }
}

/// Anschluss der Drehachse an der Steuerung.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RotaryAxis {
    /// Statt des Y-Motors angeschlossen (übliche Lösung bei GRBL-Diodenlasern).
    #[default]
    Y,
    /// Eigene A-Achse (z. B. FluidNC/grblHAL mit vier Achsen).
    A,
}

impl RotaryAxis {
    pub const ALL: [RotaryAxis; 2] = [RotaryAxis::Y, RotaryAxis::A];

    pub fn label(self) -> &'static str {
        match self {
            RotaryAxis::Y => "Y-Achse (statt Y-Motor)",
            RotaryAxis::A => "A-Achse",
        }
    }

    pub fn letter(self) -> char {
        match self {
            RotaryAxis::Y => 'Y',
            RotaryAxis::A => 'A',
        }
    }
}

/// Drehachse: Y im Dokument entspricht der abgewickelten Oberfläche des Werkstücks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rotary {
    pub enabled: bool,
    pub kind: RotaryKind,
    pub axis: RotaryAxis,
    /// Achsweg, den die Steuerung für eine volle Umdrehung der Rolle bzw. des Futters fährt
    /// (mm bzw. Einheiten der A-Achse, z. B. 360 bei Grad).
    pub travel_per_rotation: f64,
    /// Durchmesser der angetriebenen Rolle (nur Rollen).
    pub roller_diameter: f64,
    /// Durchmesser des Werkstücks.
    pub object_diameter: f64,
    /// Drehrichtung umkehren.
    pub reverse: bool,
}

impl Default for Rotary {
    fn default() -> Self {
        Self {
            enabled: false,
            kind: RotaryKind::Roller,
            axis: RotaryAxis::Y,
            travel_per_rotation: 100.0,
            roller_diameter: 20.0,
            object_diameter: 70.0,
            reverse: false,
        }
    }
}

impl Rotary {
    /// Umfang des Werkstücks in mm.
    pub fn circumference(&self) -> f64 {
        PI * self.object_diameter
    }

    /// Achsweg je mm Oberfläche (mit Vorzeichen). Auf Rollen bewegt sich die Oberfläche des Werkstücks
    /// so weit wie die der Rolle – unabhängig vom Werkstückdurchmesser.
    pub fn scale(&self) -> f64 {
        let d = match self.kind {
            RotaryKind::Roller => self.roller_diameter,
            RotaryKind::Chuck => self.object_diameter,
        };
        let s = self.travel_per_rotation / (PI * d.max(0.1));
        if self.reverse { -s } else { s }
    }

    /// Faktor, falls die Drehachse aktiv ist.
    pub fn active_scale(&self) -> Option<f64> {
        self.enabled.then(|| self.scale())
    }

    /// Ausgabe über eine eigene A-Achse.
    pub fn uses_a_axis(&self) -> bool {
        self.enabled && self.axis == RotaryAxis::A
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
    /// Jede Gruppe mit allen ihren Ebenen fertigstellen, bevor die nächste beginnt (z. B. mehrere Untersetzer:
    /// gravieren und ausschneiden, dann der nächste). Ungruppierte Objekte bilden zusammen eine Einheit.
    pub order_by_group: bool,
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
            order_by_group: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_shift_interpolates() {
        let mut p = MachineProfile::default();
        assert_eq!(p.scan_shift(6000.0), 0.0);
        p.scan_offsets = vec![ScanOffset { speed: 6000.0, offset: 0.2 }, ScanOffset { speed: 2000.0, offset: 0.1 }];
        let close = |a: f64, b: f64| (a - b).abs() < 1e-12;
        assert!(close(p.scan_shift(2000.0), 0.05));
        assert!(close(p.scan_shift(4000.0), 0.075));
        // außerhalb proportional zur Geschwindigkeit
        assert!(close(p.scan_shift(1000.0), 0.025));
        assert!(close(p.scan_shift(12000.0), 0.2));
    }

    #[test]
    fn rotary_scale() {
        let mut r = Rotary { enabled: true, travel_per_rotation: 40.0, roller_diameter: 20.0, ..Rotary::default() };
        // Rolle: unabhängig vom Werkstück; 20 mm Rolle → 62,83 mm Oberfläche je 40 mm Achsweg
        assert!((r.scale() - 40.0 / (PI * 20.0)).abs() < 1e-12);
        r.object_diameter = 200.0;
        assert!((r.scale() - 40.0 / (PI * 20.0)).abs() < 1e-12);
        r.kind = RotaryKind::Chuck;
        r.reverse = true;
        assert!((r.scale() + 40.0 / (PI * 200.0)).abs() < 1e-12);
        assert!(!r.uses_a_axis());
        r.enabled = false;
        assert_eq!(r.active_scale(), None);
    }

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
