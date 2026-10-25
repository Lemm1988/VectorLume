//! Biegescharnier („Living Hinge“): versetzte Schnittreihen, die eine Fläche aus Sperrholz o. Ä. biegsam machen.
//!
//! Parallele Linien im Abstand `spacing`; jede Linie besteht aus Schnitten der Länge `cut`, getrennt durch Stege der
//! Länge `bridge`. Benachbarte Linien sind um eine halbe Periode versetzt. Das Muster wird auf das Innere der
//! Bereichsform beschnitten (Gerade-Ungerade-Regel, Löcher bleiben frei).

use crate::geom::{Affine, Point};
use crate::path::Polyline;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HingeSpec {
    /// Länge eines Schnitts in mm.
    pub cut: f64,
    /// Steg zwischen zwei Schnitten derselben Linie in mm.
    pub bridge: f64,
    /// Abstand der Linien in mm.
    pub spacing: f64,
    /// Richtung der Linien in Grad (0 = senkrecht, Biegung um eine senkrechte Achse).
    pub angle: f64,
}

impl Default for HingeSpec {
    fn default() -> Self {
        Self { cut: 15.0, bridge: 3.0, spacing: 1.5, angle: 0.0 }
    }
}

/// Kürzere Reststücke am Rand entfallen (mm).
const MIN_PIECE: f64 = 0.5;

impl HingeSpec {
    pub fn check(&self) -> Result<(), &'static str> {
        if !(self.cut.is_finite() && self.bridge.is_finite() && self.spacing.is_finite()) {
            return Err("Ungültige Werte.");
        }
        if self.cut <= 0.0 || self.bridge <= 0.0 || self.spacing <= 0.0 {
            return Err("Schnittlänge, Steg und Linienabstand müssen größer als 0 sein.");
        }
        if self.spacing < 0.3 {
            return Err("Der Linienabstand ist zu klein (mindestens 0,3 mm).");
        }
        Ok(())
    }

    /// Schnittlinien innerhalb des Bereichs (geschlossene Polylinien; offene werden ignoriert).
    pub fn lines(&self, region: &[Polyline]) -> Vec<Polyline> {
        if self.check().is_err() {
            return Vec::new();
        }
        // In ein Koordinatensystem drehen, in dem die Linien senkrecht verlaufen
        let rot = Affine::rotate(-self.angle.to_radians());
        let back = Affine::rotate(self.angle.to_radians());
        let closed: Vec<Polyline> = region
            .iter()
            .filter(|p| p.closed && p.points.len() > 2)
            .map(|p| Polyline::new(p.points.iter().map(|q| rot.apply(*q)).collect(), true))
            .collect();
        let Some(bounds) = closed.iter().map(|p| p.bounds()).reduce(|a, b| a.union(&b)) else { return Vec::new() };
        let period = self.cut + self.bridge;
        let mut out = Vec::new();
        let mut k = 0usize;
        loop {
            let x = bounds.min.x + self.spacing / 2.0 + k as f64 * self.spacing;
            if x >= bounds.max.x {
                break;
            }
            let phase = if k.is_multiple_of(2) { 0.0 } else { period / 2.0 };
            for (y0, y1) in inside(&closed, x) {
                // Schnitte im festen Raster ab der Oberkante des Bereichs, damit Nachbarbereiche zusammenpassen
                let first = ((y0 - bounds.min.y - phase) / period).floor() as i64;
                let mut j = first;
                loop {
                    let a = bounds.min.y + phase + j as f64 * period;
                    if a >= y1 {
                        break;
                    }
                    let (s, e) = (a.max(y0), (a + self.cut).min(y1));
                    if e - s >= MIN_PIECE {
                        out.push(Polyline::new(
                            vec![back.apply(Point::new(x, s)), back.apply(Point::new(x, e))],
                            false,
                        ));
                    }
                    j += 1;
                }
            }
            k += 1;
        }
        out
    }
}

/// Abschnitte der senkrechten Geraden bei `x`, die im Bereich liegen (Gerade-Ungerade-Regel).
fn inside(polys: &[Polyline], x: f64) -> Vec<(f64, f64)> {
    let mut ys: Vec<f64> = Vec::new();
    for p in polys {
        for (a, b) in p.edges() {
            if (a.x <= x) != (b.x <= x) {
                ys.push(a.y + (x - a.x) * (b.y - a.y) / (b.x - a.x));
            }
        }
    }
    ys.sort_by(f64::total_cmp);
    ys.as_chunks::<2>().0.iter().map(|&[a, b]| (a, b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::VectorPath;

    fn region(path: VectorPath) -> Vec<Polyline> {
        path.flatten(&Affine::IDENTITY, 0.01)
    }

    #[test]
    fn rectangle_gets_staggered_cuts() {
        let spec = HingeSpec { cut: 10.0, bridge: 3.0, spacing: 2.0, angle: 0.0 };
        let rect = region(VectorPath::rect(0.0, 0.0, 50.0, 30.0, 0.0));
        let lines = spec.lines(&rect);
        let xs: std::collections::BTreeSet<i64> =
            lines.iter().map(|l| (l.points[0].x * 1000.0).round() as i64).collect();
        assert_eq!(xs.len(), 25, "eine Linie je 2 mm");
        for l in &lines {
            assert_eq!(l.points.len(), 2);
            assert!(l.points.iter().all(|p| p.x > 0.0 && p.x < 50.0 && p.y >= -1e-9 && p.y <= 30.0 + 1e-9));
            assert!(l.length() <= 10.0 + 1e-9 && l.length() >= MIN_PIECE);
        }
        // Stege: Lücken zwischen den Schnitten einer Linie genau 3 mm, Nachbarlinie um eine halbe Periode versetzt
        let col = |x: f64| {
            let mut v: Vec<(f64, f64)> = lines
                .iter()
                .filter(|l| (l.points[0].x - x).abs() < 1e-6)
                .map(|l| (l.points[0].y.min(l.points[1].y), l.points[0].y.max(l.points[1].y)))
                .collect();
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
            v
        };
        let (c0, c1) = (col(1.0), col(3.0));
        assert!(c0.windows(2).all(|w| (w[1].0 - w[0].1 - 3.0).abs() < 1e-9), "{c0:?}");
        assert_eq!(c0[0], (0.0, 10.0));
        assert!((c1[0].1 - 3.5).abs() < 1e-9, "versetzt: {c1:?}");
    }

    #[test]
    fn follows_round_regions_holes_and_angle() {
        let spec = HingeSpec::default();
        let mut ring = region(VectorPath::ellipse(Point::new(0.0, 0.0), 30.0, 30.0));
        ring.extend(region(VectorPath::ellipse(Point::new(0.0, 0.0), 10.0, 10.0)));
        let lines = spec.lines(&ring);
        assert!(!lines.is_empty());
        for l in &lines {
            for p in &l.points {
                let r = p.distance(Point::new(0.0, 0.0));
                // Bézier-Näherung des Kreises weicht um wenige µm ab
                assert!((10.0 - 0.02..=30.0 + 0.02).contains(&r), "{p:?}");
            }
        }
        // waagrecht: alle Linien waagrecht
        let flat = HingeSpec { angle: 90.0, ..spec }.lines(&region(VectorPath::rect(0.0, 0.0, 40.0, 20.0, 0.0)));
        assert!(!flat.is_empty() && flat.iter().all(|l| (l.points[0].y - l.points[1].y).abs() < 1e-9));
        assert!(HingeSpec { spacing: 0.0, ..spec }.check().is_err());
        assert!(spec.lines(&[]).is_empty());
    }
}
