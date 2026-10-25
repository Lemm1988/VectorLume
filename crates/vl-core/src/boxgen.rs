//! Box mit Fingerzinken: sechs Platten (ohne Deckel fünf), deren Kanten ineinandergreifen.
//!
//! Aufteilung: Vorder- und Rückwand haben an allen Kanten Zinken nach außen und behalten die Ecken, die Seiten an
//! den senkrechten Kanten Aussparungen und oben/unten Zinken, Boden und Deckel ringsum Aussparungen. Damit gehört
//! jedes Stück der Wand genau einer Platte; gemeinsame Kanten werden gleich geteilt (ungerade Anzahl gleich langer
//! Abschnitte), sodass sich Zinken und Aussparungen genau ergänzen.

use crate::geom::Point;
use crate::path::Polyline;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BoxSpec {
    /// Breite (X), Tiefe (Y) und Höhe (Z) in mm.
    pub width: f64,
    pub depth: f64,
    pub height: f64,
    /// Die Maße gelten innen (sonst außen).
    pub inner: bool,
    /// Materialstärke in mm.
    pub thickness: f64,
    /// Gewünschte Zinkenbreite in mm; je Kante wird auf eine ungerade Anzahl gleich breiter Abschnitte gerundet.
    pub finger: f64,
    /// Ohne Deckel: Wände oben gerade.
    pub open_top: bool,
}

impl Default for BoxSpec {
    fn default() -> Self {
        Self { width: 100.0, depth: 80.0, height: 50.0, inner: false, thickness: 3.0, finger: 10.0, open_top: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Front,
    Back,
    Left,
    Right,
    Bottom,
    Top,
}

impl Panel {
    pub fn label(self) -> &'static str {
        match self {
            Panel::Front => "Vorderwand",
            Panel::Back => "Rückwand",
            Panel::Left => "Seite links",
            Panel::Right => "Seite rechts",
            Panel::Bottom => "Boden",
            Panel::Top => "Deckel",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    /// Zinken nach außen, Ecken bleiben.
    Out,
    /// Aussparungen, Ecken fehlen.
    In,
    /// Gerade Kante in voller Größe.
    Flat,
}

impl BoxSpec {
    /// Außenmaße (X, Y, Z).
    pub fn outer(&self) -> (f64, f64, f64) {
        let t = self.thickness;
        if self.inner {
            let top = if self.open_top { t } else { 2.0 * t };
            (self.width + 2.0 * t, self.depth + 2.0 * t, self.height + top)
        } else {
            (self.width, self.depth, self.height)
        }
    }

    /// Anzahl gleich langer Abschnitte (ungerade, mindestens 3) entlang einer Kante der Länge `len`.
    pub fn sections(&self, len: f64) -> usize {
        let n = ((len / self.finger.max(0.1) - 1.0) / 2.0).round().max(1.0) as usize * 2 + 1;
        n.max(3)
    }

    /// Prüft die Maße; der Text erklärt das Problem.
    pub fn check(&self) -> Result<(), &'static str> {
        let t = self.thickness;
        let (x, y, z) = self.outer();
        if t.is_nan() || t <= 0.0 || !x.is_finite() || !y.is_finite() || !z.is_finite() {
            return Err("Materialstärke und Maße müssen größer als 0 sein.");
        }
        if x.min(y).min(z) < 4.0 * t {
            return Err("Die Box ist für diese Materialstärke zu klein (jede Seite mindestens 4 × Materialstärke).");
        }
        if [x, y, z].iter().any(|&len| len / self.sections(len) as f64 <= 1.2 * t) {
            return Err("Die Zinken sind zu schmal – Zinkenbreite mindestens etwa 1,5 × Materialstärke.");
        }
        Ok(())
    }

    /// Platten mit ihren Umrissen (geschlossen, Plattenkoordinaten: Ursprung oben links, y nach unten). Vorder-,
    /// Rückwand und Seiten stehen aufrecht (unten = Boden).
    pub fn panels(&self) -> Vec<(Panel, Polyline)> {
        use Edge::*;
        let (x, y, z) = self.outer();
        let top = if self.open_top { Flat } else { Out };
        // Kanten in der Reihenfolge unten, rechts, oben, links
        let mut out = vec![
            (Panel::Front, self.outline(x, z, [Out, Out, top, Out])),
            (Panel::Back, self.outline(x, z, [Out, Out, top, Out])),
            (Panel::Left, self.outline(y, z, [Out, In, top, In])),
            (Panel::Right, self.outline(y, z, [Out, In, top, In])),
            (Panel::Bottom, self.outline(x, y, [In, In, In, In])),
        ];
        if !self.open_top {
            out.push((Panel::Top, self.outline(x, y, [In, In, In, In])));
        }
        out
    }

    fn depth(&self, e: Edge, k: usize) -> f64 {
        match (e, k.is_multiple_of(2)) {
            (Edge::Flat, _) | (Edge::Out, true) | (Edge::In, false) => 0.0,
            _ => self.thickness,
        }
    }

    /// Umriss einer w × h großen Platte. Jede Kante verläuft in ihren Abschnitten bündig oder um die Materialstärke
    /// eingerückt; an den Ecken beginnt sie dort, wo die Nachbarkante eingerückt ist.
    fn outline(&self, w: f64, h: f64, edges: [Edge; 4]) -> Polyline {
        // (Anfang, Richtung, nach innen, Länge) – zunächst mit y nach oben
        let frames = [
            (Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(0.0, 1.0), w),
            (Point::new(w, 0.0), Point::new(0.0, 1.0), Point::new(-1.0, 0.0), h),
            (Point::new(w, h), Point::new(-1.0, 0.0), Point::new(0.0, -1.0), w),
            (Point::new(0.0, h), Point::new(0.0, -1.0), Point::new(1.0, 0.0), h),
        ];
        let mut pts: Vec<Point> = Vec::new();
        for (i, &(origin, dir, inward, len)) in frames.iter().enumerate() {
            let e = edges[i];
            let n = if e == Edge::Flat { 1 } else { self.sections(len) };
            let start = self.depth(edges[(i + 3) % 4], 0);
            let end = len - self.depth(edges[(i + 1) % 4], 0);
            for k in 0..n {
                let u0 = (k as f64 * len / n as f64).max(start);
                let u1 = ((k + 1) as f64 * len / n as f64).min(end);
                let v = self.depth(e, k);
                for u in [u0, u1] {
                    let p = origin + dir * u + inward * v;
                    if pts.last().is_none_or(|q| q.distance(p) > 1e-9) {
                        pts.push(p);
                    }
                }
            }
        }
        if pts.len() > 1 && pts[0].distance(pts[pts.len() - 1]) < 1e-9 {
            pts.pop();
        }
        // y nach unten, „unten“ der Platte unten im Dokument
        Polyline::new(pts.into_iter().map(|p| Point::new(p.x, h - p.y)).collect(), true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(spec: &BoxSpec) -> f64 {
        spec.panels().iter().map(|(_, p)| p.signed_area().abs()).sum::<f64>() * spec.thickness
    }

    #[test]
    fn panels_fill_the_walls_exactly() {
        // Jede Stelle der Wand gehört genau einer Platte: Plattenvolumen = Außen- minus Innenvolumen.
        for spec in [
            BoxSpec::default(),
            BoxSpec { width: 123.0, depth: 47.5, height: 61.0, thickness: 4.0, finger: 9.0, ..Default::default() },
            BoxSpec { open_top: true, ..Default::default() },
            BoxSpec { inner: true, open_top: true, thickness: 6.0, finger: 15.0, ..Default::default() },
        ] {
            spec.check().unwrap();
            let t = spec.thickness;
            let (x, y, z) = spec.outer();
            let inner_z = if spec.open_top { z - t } else { z - 2.0 * t };
            let expected = x * y * z - (x - 2.0 * t) * (y - 2.0 * t) * inner_z;
            assert!((volume(&spec) - expected).abs() < 1e-6, "{spec:?}: {} statt {expected}", volume(&spec));
            assert_eq!(spec.panels().len(), if spec.open_top { 5 } else { 6 });
        }
    }

    #[test]
    fn panel_sizes_and_fingers() {
        let spec =
            BoxSpec { width: 100.0, depth: 80.0, height: 50.0, thickness: 3.0, finger: 10.0, ..Default::default() };
        let size = |p: Panel| {
            let b = spec.panels().into_iter().find(|(q, _)| *q == p).unwrap().1.bounds();
            (b.width(), b.height())
        };
        assert_eq!(size(Panel::Front), (100.0, 50.0));
        // auch Kanten mit Aussparungen reichen zwischen ihnen bis ans Außenmaß
        assert_eq!(size(Panel::Left), (80.0, 50.0));
        assert_eq!(size(Panel::Bottom), (100.0, 80.0));
        // nächstgelegene ungerade Anzahl, bei Gleichstand die schmaleren Zinken
        assert_eq!((spec.sections(100.0), spec.sections(80.0), spec.sections(50.0)), (11, 9, 5));
        // Vorderwand: 11 + 5 + 11 + 5 Abschnitte, an jedem Wechsel zwei Ecken
        let front = spec.panels().remove(0).1;
        assert_eq!(front.points.len(), 2 * (10 + 4 + 10 + 4) + 4);
        // Innenmaße
        let inner = BoxSpec { inner: true, ..spec };
        assert_eq!(inner.outer(), (106.0, 86.0, 56.0));
        assert_eq!(BoxSpec { open_top: true, ..inner }.outer(), (106.0, 86.0, 53.0));
    }

    #[test]
    fn rejects_impossible_boxes() {
        assert!(BoxSpec { thickness: 0.0, ..Default::default() }.check().is_err());
        assert!(BoxSpec { width: 10.0, ..Default::default() }.check().is_err());
        assert!(BoxSpec { finger: 3.0, ..Default::default() }.check().is_err());
    }
}
