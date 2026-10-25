//! Zahnräder: Stirnräder mit Evolventenverzahnung nach Modul, Zähnezahl und Eingriffswinkel.
//!
//! Kopfhöhe 1·m, Fußhöhe 1,25·m. Die Flanken sind Evolventen des Grundkreises; liegt der Fußkreis innerhalb des
//! Grundkreises, laufen sie darunter radial weiter (übliche Vereinfachung ohne Fußrundung). Das Flankenspiel wird zu
//! gleichen Teilen von beiden Rädern eines Paars abgenommen.

use crate::geom::Point;
use crate::path::{PathBuilder, VectorPath};
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GearSpec {
    /// Modul in mm (Teilkreisdurchmesser / Zähnezahl).
    pub module: f64,
    pub teeth: u32,
    /// Eingriffswinkel in Grad (Norm: 20°).
    pub pressure_angle: f64,
    /// Flankenspiel des Radpaars am Teilkreis in mm; jedes Rad wird um die Hälfte dünner.
    pub backlash: f64,
    /// Bohrung in mm (0 = keine).
    pub bore: f64,
}

impl Default for GearSpec {
    fn default() -> Self {
        Self { module: 2.0, teeth: 20, pressure_angle: 20.0, backlash: 0.15, bore: 5.0 }
    }
}

fn inv(a: f64) -> f64 {
    a.tan() - a
}

/// Punkte je Flanke.
const FLANK_STEPS: usize = 14;
/// Größter Winkelschritt auf Kopf- und Fußkreis.
const ARC_STEP: f64 = PI / 90.0;

impl GearSpec {
    fn alpha(&self) -> f64 {
        self.pressure_angle.to_radians()
    }

    pub fn pitch_radius(&self) -> f64 {
        self.module * self.teeth as f64 / 2.0
    }

    pub fn base_radius(&self) -> f64 {
        self.pitch_radius() * self.alpha().cos()
    }

    pub fn tip_radius(&self) -> f64 {
        self.pitch_radius() + self.module
    }

    pub fn root_radius(&self) -> f64 {
        self.pitch_radius() - 1.25 * self.module
    }

    /// Achsabstand zu einem Gegenrad mit gleichem Modul.
    pub fn center_distance(&self, other: &GearSpec) -> f64 {
        self.pitch_radius() + other.pitch_radius()
    }

    /// Halber Zahnwinkel bei Radius `r` (ab dem Grundkreis).
    fn half_angle(&self, r: f64) -> f64 {
        let rb = self.base_radius();
        let ar = (rb / r.max(rb)).acos();
        PI / (2.0 * self.teeth as f64) - self.backlash / (4.0 * self.pitch_radius()) + inv(self.alpha()) - inv(ar)
    }

    pub fn check(&self) -> Result<(), &'static str> {
        if !(self.module.is_finite() && self.module >= 0.3 && self.module <= 50.0) {
            return Err("Modul: 0,3 bis 50 mm.");
        }
        if !(6..=300).contains(&self.teeth) {
            return Err("Zähne: 6 bis 300.");
        }
        if !(10.0..=35.0).contains(&self.pressure_angle) {
            return Err("Eingriffswinkel: 10° bis 35°.");
        }
        if !(0.0..=self.module).contains(&self.backlash) {
            return Err("Flankenspiel: 0 bis 1 Modul.");
        }
        if self.half_angle(self.tip_radius()) <= 0.02 / self.teeth as f64 {
            return Err("Die Zähne werden spitz – mehr Zähne oder kleineren Eingriffswinkel wählen.");
        }
        if self.bore < 0.0 || self.bore / 2.0 > self.root_radius() - self.module {
            return Err("Die Bohrung ist zu groß für das Zahnrad.");
        }
        Ok(())
    }

    /// Umriss um 0/0 (ein Zahn zeigt in Richtung +x) und die Bohrung.
    pub fn outline(&self) -> VectorPath {
        let (rb, ra, rf) = (self.base_radius(), self.tip_radius(), self.root_radius());
        let z = self.teeth as usize;
        let polar = |r: f64, a: f64| Point::new(r * a.cos(), r * a.sin());
        // Flanke vom Fuß bis zum Kopf als (Radius, halber Zahnwinkel)
        let mut flank: Vec<(f64, f64)> = Vec::new();
        if rf < rb {
            flank.push((rf, self.half_angle(rb)));
        }
        let start = rf.max(rb);
        let (t0, t1) = (((start / rb).powi(2) - 1.0).max(0.0).sqrt(), ((ra / rb).powi(2) - 1.0).sqrt());
        for i in 0..=FLANK_STEPS {
            let t = t0 + (t1 - t0) * i as f64 / FLANK_STEPS as f64;
            let r = rb * (1.0 + t * t).sqrt();
            flank.push((r, self.half_angle(r)));
        }
        let arc = |pts: &mut Vec<Point>, r: f64, from: f64, to: f64| {
            let n = ((to - from) / ARC_STEP).ceil().max(1.0) as usize;
            for i in 1..n {
                pts.push(polar(r, from + (to - from) * i as f64 / n as f64));
            }
        };
        let mut pts = Vec::with_capacity(z * (2 * flank.len() + 20));
        for k in 0..z {
            let c = TAU * k as f64 / z as f64;
            pts.extend(flank.iter().map(|&(r, h)| polar(r, c - h)));
            let tip = flank.last().unwrap().1;
            arc(&mut pts, ra, c - tip, c + tip);
            pts.extend(flank.iter().rev().map(|&(r, h)| polar(r, c + h)));
            let root = flank[0].1;
            arc(&mut pts, rf, c + root, c + TAU / z as f64 - root);
        }
        let mut path = VectorPath::polygon(&pts, true);
        if self.bore > 0.0 {
            let mut pb = PathBuilder::new();
            pb.ellipse(Point::ZERO, self.bore / 2.0, self.bore / 2.0);
            path.subpaths.extend(pb.finish().subpaths);
        }
        path
    }

    /// Drehung des Gegenrads (um seine Mitte rechts von diesem Rad), damit eine Lücke einem Zahn gegenübersteht.
    pub fn mesh_rotation(&self, other: &GearSpec) -> f64 {
        PI + PI / other.teeth as f64
    }

    /// Ob die Zahnköpfe eines der beiden Räder unterhalb des Grundkreises in das andere eingreifen würden
    /// (Eingriffsstörung, typisch bei wenigen Zähnen).
    pub fn interferes(&self, other: &GearSpec) -> bool {
        let line = self.center_distance(other) * self.alpha().sin();
        let limit = |g: &GearSpec| (g.base_radius().powi(2) + line.powi(2)).sqrt();
        self.tip_radius() > limit(self) + 1e-9 || other.tip_radius() > limit(other) + 1e-9
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Affine;
    use crate::path::{Polyline, point_in_polylines};

    fn outline(g: &GearSpec, t: &Affine) -> Polyline {
        g.outline().flatten(t, 0.01).remove(0)
    }

    #[test]
    fn teeth_count_radii_and_thickness() {
        let g = GearSpec { backlash: 0.2, bore: 0.0, ..Default::default() };
        g.check().unwrap();
        let p = outline(&g, &Affine::IDENTITY);
        assert!(p.closed);
        let r = |q: &Point| q.distance(Point::ZERO);
        assert!(p.points.iter().all(|q| r(q) > g.root_radius() - 1e-9 && r(q) < g.tip_radius() + 1e-9));
        // Zähne zählen: zusammenhängende Läufe auf dem Kopfkreis
        let on_tip: Vec<bool> = p.points.iter().map(|q| r(q) > g.tip_radius() - 1e-6).collect();
        let runs = (0..on_tip.len()).filter(|&i| on_tip[i] && !on_tip[(i + on_tip.len() - 1) % on_tip.len()]).count();
        assert_eq!(runs, 20);
        // Zahndicke am Teilkreis: π·m/2 − Spiel/2 je Zahn
        let n = 20_000;
        let inside = (0..n)
            .filter(|i| {
                let a = TAU * *i as f64 / n as f64;
                point_in_polylines(
                    Point::new(g.pitch_radius() * a.cos(), g.pitch_radius() * a.sin()),
                    std::slice::from_ref(&p),
                )
            })
            .count() as f64
            / n as f64;
        let expected = (PI * g.module / 2.0 - g.backlash / 2.0) / (PI * g.module);
        assert!((inside - expected).abs() < 0.002, "{inside} vs {expected}");
    }

    /// Das Paar dreht sich ineinander, ohne dass sich die Umrisse überschneiden, und greift tatsächlich ein.
    #[test]
    fn pair_meshes_without_collision() {
        let g1 = GearSpec { teeth: 15, bore: 0.0, ..Default::default() };
        let g2 = GearSpec { teeth: 31, bore: 0.0, ..Default::default() };
        assert!(!g1.interferes(&g2));
        let a = g1.center_distance(&g2);
        let c2 = Point::new(a, 0.0);
        let steps = 24;
        for s in 0..steps {
            let th1 = TAU / g1.teeth as f64 * s as f64 / steps as f64;
            let th2 = g1.mesh_rotation(&g2) - th1 * g1.teeth as f64 / g2.teeth as f64;
            let p1 = outline(&g1, &Affine::rotate(th1));
            let p2 = outline(&g2, &Affine::rotate(th2).then(&Affine::translate(c2.x, c2.y)));
            let hits = p2.points.iter().filter(|q| point_in_polylines(**q, std::slice::from_ref(&p1))).count()
                + p1.points.iter().filter(|q| point_in_polylines(**q, std::slice::from_ref(&p2))).count();
            assert_eq!(hits, 0, "Überschneidung bei Schritt {s}");
            let near = p1
                .points
                .iter()
                .filter(|q| q.x > a / 3.0)
                .flat_map(|q| p2.points.iter().map(move |w| q.distance(*w)))
                .fold(f64::MAX, f64::min);
            assert!(near < 0.4, "kein Eingriff bei Schritt {s}: {near}");
        }
    }

    #[test]
    fn checks_and_interference() {
        assert!(GearSpec { teeth: 5, ..Default::default() }.check().is_err());
        assert!(GearSpec { bore: 40.0, ..Default::default() }.check().is_err());
        let small = GearSpec { teeth: 8, ..Default::default() };
        small.check().unwrap();
        assert!(small.interferes(&GearSpec { teeth: 60, ..Default::default() }));
        assert_eq!(GearSpec::default().outline().subpaths.len(), 2);
    }
}
