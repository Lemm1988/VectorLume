//! Schnittbreiten-Test: ein Kamm aus gleich breiten Streifen in einem Rahmen. Nach dem Schneiden werden die Streifen
//! zusammengeschoben und gemessen; aus dem Messwert ergibt sich die Schnittbreite (Kerf) des Lasers.
//!
//! Jeder Schnitt nimmt die Schnittbreite k weg. Bei n Streifen der Breite w entstehen n + 1 Schnitte quer zum Kamm:
//! Das Loch im Rahmen wird w·n + k breit, jeder Streifen w − k. Schiebt man alle Streifen an eine Seite, bleibt ein
//! Spalt von (n + 1)·k; misst man die zusammengeschobenen Streifen, fehlen n·k gegenüber n·w. Der Fehler des Messgeräts
//! verteilt sich dabei auf alle Schnitte – deutlich genauer als ein einzelnes Teil nachzumessen.

use crate::geom::Point;
use crate::path::{PathBuilder, VectorPath};
use serde::{Deserialize, Serialize};

/// So weit laufen die Trennschnitte über den Rahmen hinaus (mm), damit die Streifen sicher frei sind – auch wenn der
/// Rahmen durch eine schon eingestellte Schnittbreite etwas größer geschnitten wird.
pub const OVERRUN: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KerfTest {
    pub strips: u32,
    /// Breite eines Streifens in mm.
    pub strip_width: f64,
    /// Länge der Streifen (Höhe des Kamms) in mm.
    pub height: f64,
}

impl Default for KerfTest {
    fn default() -> Self {
        Self { strips: 10, strip_width: 6.0, height: 20.0 }
    }
}

/// Was nach dem Schneiden gemessen wurde.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Measured {
    /// Breite aller Streifen, dicht zusammengeschoben.
    Strips,
    /// Spalt im Rahmen, wenn alle Streifen an einer Seite liegen.
    Gap,
}

impl KerfTest {
    pub fn check(&self) -> Result<(), &'static str> {
        if !(2..=50).contains(&self.strips) {
            return Err("Streifen: 2 bis 50.");
        }
        if !(self.strip_width.is_finite() && self.strip_width >= 2.0 && self.height.is_finite() && self.height >= 5.0) {
            return Err("Streifen mindestens 2 mm breit und 5 mm lang.");
        }
        Ok(())
    }

    /// Breite des Kamms ohne Schnittbreite.
    pub fn width(&self) -> f64 {
        self.strips as f64 * self.strip_width
    }

    /// Rahmen (geschlossen) und Trennschnitte, links oben bei 0/0. Jeder Trennschnitt besteht aus zwei Hälften, die in
    /// der Mitte des Kamms beginnen: So liegen sie innerhalb des Rahmens, und „innen vor außen“ schneidet sie zuerst,
    /// solange der Kamm noch fest im Material sitzt.
    pub fn build(&self) -> VectorPath {
        let mut path = VectorPath::rect(0.0, 0.0, self.width(), self.height, 0.0);
        let mut pb = PathBuilder::new();
        let mid = self.height / 2.0;
        for i in 1..self.strips {
            let x = i as f64 * self.strip_width;
            for end in [-OVERRUN, self.height + OVERRUN] {
                pb.move_to(Point::new(x, mid));
                pb.line_to(Point::new(x, end));
            }
        }
        path.subpaths.extend(pb.finish().subpaths);
        path
    }

    /// Schnittbreite aus dem Messwert in mm. `cut_kerf` ist die Schnittbreite, die beim Schneiden des Tests schon auf
    /// der Ebene eingestellt war: Sie versetzt den Rahmen nach außen und macht die beiden äußeren Streifen um je die
    /// Hälfte breiter (auf den Spalt wirkt sie sich nicht aus).
    pub fn kerf(&self, measured: Measured, value: f64, cut_kerf: f64) -> Result<f64, &'static str> {
        let n = self.strips as f64;
        let k = match measured {
            Measured::Strips => (self.width() + cut_kerf - value) / n,
            Measured::Gap => value / (n + 1.0),
        };
        if !k.is_finite() || value <= 0.0 {
            return Err("Bitte den Messwert eingeben.");
        }
        if !(0.0..=2.0).contains(&k) {
            return Err("Der Messwert ergibt keine plausible Schnittbreite (0 bis 2 mm) – bitte nachmessen.");
        }
        Ok(k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Affine;
    use crate::path::point_in_polylines;

    #[test]
    fn comb_has_frame_and_dividers_starting_inside() {
        let t = KerfTest::default();
        t.check().unwrap();
        let polys = t.build().flatten(&Affine::IDENTITY, 0.01);
        assert_eq!(polys.len(), 1 + 2 * 9);
        let frame = &polys[0];
        assert!(frame.closed);
        assert_eq!(frame.bounds().width(), 60.0);
        for p in &polys[1..] {
            assert!(!p.closed);
            assert!(point_in_polylines(p.points[0], std::slice::from_ref(frame)));
            let end = p.points.last().unwrap();
            assert!(end.y < 0.0 || end.y > t.height, "Schnitt läuft über den Rahmen hinaus");
        }
    }

    #[test]
    fn kerf_from_both_measurements() {
        let t = KerfTest::default();
        let k = 0.15;
        // Streifen: 10 · (6 − 0,15) = 58,5; Spalt: 11 · 0,15 = 1,65
        assert!((t.kerf(Measured::Strips, 58.5, 0.0).unwrap() - k).abs() < 1e-9);
        assert!((t.kerf(Measured::Gap, 1.65, 0.0).unwrap() - k).abs() < 1e-9);
        // mit 0,1 mm schon eingestellter Schnittbreite werden die äußeren Streifen je 0,05 mm breiter
        assert!((t.kerf(Measured::Strips, 58.6, 0.1).unwrap() - k).abs() < 1e-9);
        assert!((t.kerf(Measured::Gap, 1.65, 0.1).unwrap() - k).abs() < 1e-9);
        assert!(t.kerf(Measured::Strips, 61.0, 0.0).is_err());
        assert!(t.kerf(Measured::Gap, 0.0, 0.0).is_err());
        assert!(KerfTest { strips: 1, ..t }.check().is_err());
    }
}
