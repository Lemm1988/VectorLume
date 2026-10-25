//! Puzzle: Schnittlinien für ein Puzzle aus Zeilen × Spalten mit Nasen (z. B. über ein graviertes Foto).
//!
//! Jede innere Rasterlinie wird ein durchgehender Linienzug; auf jeder Teilkante sitzt eine Nase aus sechs
//! kubischen Bézierkurven (schmaler Hals, breiter Kopf), zufällig nach links oder rechts gerichtet und leicht
//! verschoben bzw. in der Höhe verändert. Der Zufall hängt nur vom Startwert ab – gleiche Werte, gleiches Puzzle.

use crate::geom::Point;
use crate::path::{PathBuilder, VectorPath};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PuzzleSpec {
    /// Größe in mm.
    pub width: f64,
    pub height: f64,
    pub cols: u32,
    pub rows: u32,
    /// Höhe der Nasen relativ zur kleineren Teileseite.
    pub tab: f64,
    /// Zufällige Abweichung von Lage und Größe der Nasen (0 … 1).
    pub jitter: f64,
    pub seed: u64,
    /// Außenrahmen mitschneiden.
    pub border: bool,
    /// Eckenradius des Rahmens in mm.
    pub corner_radius: f64,
}

impl Default for PuzzleSpec {
    fn default() -> Self {
        Self {
            width: 200.0,
            height: 150.0,
            cols: 6,
            rows: 4,
            tab: 0.22,
            jitter: 0.5,
            seed: 1,
            border: true,
            corner_radius: 0.0,
        }
    }
}

/// Linke Hälfte der Nase: (Kontrollpunkt 1, Kontrollpunkt 2, Endpunkt) je Kurve, u entlang der Kante (0 … 1),
/// v quer dazu in Nasenhöhen. Die rechte Hälfte ist gespiegelt.
const HALF: [[(f64, f64); 3]; 3] = [
    [(0.18, 0.0), (0.42, -0.04), (0.40, 0.04)],
    [(0.45, 0.22), (0.27, 0.36), (0.33, 0.60)],
    [(0.37, 0.86), (0.42, 1.0), (0.50, 1.0)],
];

/// Kleiner, reproduzierbarer Zufallsgenerator (SplitMix64).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Gleichverteilt in [-1, 1).
    fn signed(&mut self) -> f64 {
        self.next() * 2.0 - 1.0
    }
}

impl PuzzleSpec {
    fn cell(&self) -> (f64, f64) {
        (self.width / self.cols.max(1) as f64, self.height / self.rows.max(1) as f64)
    }

    pub fn check(&self) -> Result<(), &'static str> {
        if !(self.width.is_finite() && self.height.is_finite()) || self.width <= 0.0 || self.height <= 0.0 {
            return Err("Breite und Höhe müssen größer als 0 sein.");
        }
        if self.cols == 0 || self.rows == 0 || self.cols > 100 || self.rows > 100 {
            return Err("Spalten und Zeilen: 1 bis 100.");
        }
        let (cw, ch) = self.cell();
        if cw.min(ch) < 5.0 {
            return Err("Die Teile wären kleiner als 5 mm.");
        }
        Ok(())
    }

    /// Schnittlinien (ein offener Teilpfad je innerer Rasterlinie) und der Rahmen, links oben bei 0/0.
    pub fn build(&self) -> (VectorPath, Option<VectorPath>) {
        let (cw, ch) = self.cell();
        let m = cw.min(ch);
        let tab = self.tab.clamp(0.05, 0.35) * m;
        let jitter = self.jitter.clamp(0.0, 1.0);
        let mut rng = Rng(self.seed);
        let mut pb = PathBuilder::new();
        let edge = |pb: &mut PathBuilder, a: Point, b: Point, rng: &mut Rng| {
            let len = a.distance(b);
            let dir = (b - a) * (1.0 / len);
            let normal = Point::new(-dir.y, dir.x);
            let flip = if rng.next() < 0.5 { -1.0 } else { 1.0 };
            let shift = rng.signed() * 0.06 * jitter;
            let height = tab * (1.0 + rng.signed() * 0.12 * jitter) * flip;
            // Nasenbreite an der kleineren Teileseite ausrichten, damit lange Kanten keine breiten Nasen bekommen
            let squeeze = m / len;
            let map = |(u, v): (f64, f64)| {
                let u = if u <= 0.0 || u >= 1.0 { u } else { 0.5 + (u - 0.5) * squeeze + shift * squeeze };
                a + dir * (u * len) + normal * (v * height)
            };
            let right: Vec<[(f64, f64); 3]> = HALF
                .iter()
                .rev()
                .zip(HALF.iter().rev().skip(1).map(|c| c[2]).chain([(0.0, 0.0)]))
                .map(|(c, end)| {
                    let m = |(u, v): (f64, f64)| (1.0 - u, v);
                    [m(c[1]), m(c[0]), m(end)]
                })
                .collect();
            for c in HALF.iter().chain(right.iter()) {
                pb.cubic_to(map(c[0]), map(c[1]), map(c[2]));
            }
        };
        for i in 1..self.cols {
            let x = i as f64 * cw;
            pb.move_to(Point::new(x, 0.0));
            for j in 0..self.rows {
                edge(&mut pb, Point::new(x, j as f64 * ch), Point::new(x, (j + 1) as f64 * ch), &mut rng);
            }
        }
        for j in 1..self.rows {
            let y = j as f64 * ch;
            pb.move_to(Point::new(0.0, y));
            for i in 0..self.cols {
                edge(&mut pb, Point::new(i as f64 * cw, y), Point::new((i + 1) as f64 * cw, y), &mut rng);
            }
        }
        let border = self.border.then(|| {
            let r = self.corner_radius.clamp(0.0, m / 2.0);
            VectorPath::rect(0.0, 0.0, self.width, self.height, r)
        });
        (pb.finish(), border)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Affine;

    #[test]
    fn grid_lines_with_tabs_stay_inside() {
        let spec = PuzzleSpec::default();
        spec.check().unwrap();
        let (cuts, border) = spec.build();
        assert_eq!(cuts.subpaths.len(), 5 + 3);
        assert!(border.is_some());
        let polys = cuts.flatten(&Affine::IDENTITY, 0.01);
        for p in &polys {
            assert!(!p.closed);
            for q in &p.points {
                assert!(q.x >= -1e-9 && q.x <= 200.0 + 1e-9 && q.y >= -1e-9 && q.y <= 150.0 + 1e-9, "{q:?}");
            }
        }
        // senkrechte Linien von oben nach unten, waagrechte von links nach rechts
        let first = &polys[0];
        assert!(first.points[0].distance(Point::new(200.0 / 6.0, 0.0)) < 1e-9);
        assert!(first.points.last().unwrap().distance(Point::new(200.0 / 6.0, 150.0)) < 1e-9);
        // Nasen: die Linie weicht deutlich von der Geraden ab, aber höchstens um die Nasenhöhe (+ Zufall)
        let dev = first.points.iter().map(|q| (q.x - 200.0 / 6.0).abs()).fold(0.0, f64::max);
        let m = (200.0f64 / 6.0).min(150.0 / 4.0);
        assert!(dev > 0.15 * m && dev < 0.22 * m * 1.07 + 0.1 * m, "{dev}");
    }

    #[test]
    fn same_seed_same_puzzle() {
        let a = PuzzleSpec::default().build().0;
        assert_eq!(a, PuzzleSpec::default().build().0);
        assert_ne!(a, PuzzleSpec { seed: 2, ..Default::default() }.build().0);
        let single = PuzzleSpec { cols: 1, rows: 1, ..Default::default() }.build().0;
        assert!(single.subpaths.is_empty());
        assert!(PuzzleSpec { cols: 100, width: 100.0, ..Default::default() }.check().is_err());
    }
}
