//! Objektfang: Knoten, Mittelpunkte und Ausrichtungslinien anderer Objekte beim Zeichnen, Verschieben und
//! Bearbeiten von Knoten.

use vl_core::nodes;
use vl_core::{Document, ObjectId, Point, Rect as DRect, Shape};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapKind {
    Node,
    Center,
    Corner,
}

impl SnapKind {
    pub fn label(self) -> &'static str {
        match self {
            SnapKind::Node => "Knoten",
            SnapKind::Center => "Mitte",
            SnapKind::Corner => "Ecke",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Guide {
    /// senkrechte Linie bei x
    Vertical(f64),
    /// waagrechte Linie bei y
    Horizontal(f64),
}

/// Was gerade gefangen wurde (für die Anzeige auf der Arbeitsfläche).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnapHint {
    pub point: Option<(Point, SnapKind)>,
    pub guides: Vec<Guide>,
}

/// Obergrenze, damit sehr große Dokumente (z. B. nachgezeichnete Fotos) flüssig bleiben.
const MAX_POINTS: usize = 50_000;

/// Fangziele, einmal zu Beginn eines Ziehvorgangs gesammelt.
#[derive(Default)]
pub struct SnapTargets {
    points: Vec<(Point, SnapKind)>,
    xs: Vec<f64>,
    ys: Vec<f64>,
}

fn nearest_value(values: &[f64], v: f64, tol: f64) -> Option<f64> {
    values.iter().copied().filter(|t| (t - v).abs() <= tol).min_by(|a, b| (a - v).abs().total_cmp(&(b - v).abs()))
}

impl SnapTargets {
    /// Alle sichtbaren Objekte außer denen, für die `skip` zutrifft.
    pub fn collect(doc: &Document, skip: impl Fn(ObjectId) -> bool) -> Self {
        let mut t = SnapTargets::default();
        for obj in &doc.objects {
            if !doc.layers[obj.layer].visible || skip(obj.id) {
                continue;
            }
            let b = obj.bounds();
            if b.is_empty() {
                continue;
            }
            let c = b.center();
            t.points.push((c, SnapKind::Center));
            t.xs.extend([b.min.x, c.x, b.max.x]);
            t.ys.extend([b.min.y, c.y, b.max.y]);
            match &obj.shape {
                Shape::Path(p) => {
                    for (_, n) in nodes::nodes(p) {
                        if t.points.len() >= MAX_POINTS {
                            break;
                        }
                        t.points.push((obj.transform.apply(n), SnapKind::Node));
                    }
                }
                Shape::Image(_) => t.points.extend(b.corners().map(|c| (c, SnapKind::Corner))),
            }
        }
        t
    }

    pub fn add_point(&mut self, p: Point, kind: SnapKind) {
        self.points.push((p, kind));
    }

    fn nearest_point(&self, p: Point, tol: f64) -> Option<(Point, SnapKind, f64)> {
        let tol_sq = tol * tol;
        let mut best: Option<(Point, SnapKind, f64)> = None;
        for &(q, kind) in &self.points {
            let d = q.distance_sq(p);
            // Knoten vor Mittelpunkten bevorzugen, wenn beide gleich nah sind
            if d <= tol_sq && best.is_none_or(|b| d < b.2) {
                best = Some((q, kind, d));
            }
        }
        best.map(|(q, k, d)| (q, k, d.sqrt()))
    }

    /// Fängt einen einzelnen Punkt (Mauszeiger, gezogener Knoten). `tol` in mm.
    pub fn snap_point(&self, p: Point, tol: f64) -> Option<(Point, SnapHint)> {
        if let Some((q, kind, _)) = self.nearest_point(p, tol) {
            return Some((q, SnapHint { point: Some((q, kind)), guides: Vec::new() }));
        }
        let x = nearest_value(&self.xs, p.x, tol);
        let y = nearest_value(&self.ys, p.y, tol);
        if x.is_none() && y.is_none() {
            return None;
        }
        let mut hint = SnapHint::default();
        let mut q = p;
        if let Some(x) = x {
            q.x = x;
            hint.guides.push(Guide::Vertical(x));
        }
        if let Some(y) = y {
            q.y = y;
            hint.guides.push(Guide::Horizontal(y));
        }
        Some((q, hint))
    }

    /// Fängt eine verschobene Auswahl mit Umriss `b`. Liefert die Korrektur, die zur Verschiebung addiert wird.
    pub fn snap_rect(&self, b: &DRect, tol: f64) -> Option<(Point, SnapHint)> {
        let c = b.center();
        let candidates = [b.min, b.max, Point::new(b.max.x, b.min.y), Point::new(b.min.x, b.max.y), c];
        let mut best: Option<(Point, Point, SnapKind, f64)> = None;
        for cand in candidates {
            if let Some((q, kind, d)) = self.nearest_point(cand, tol)
                && best.is_none_or(|b| d < b.3)
            {
                best = Some((cand, q, kind, d));
            }
        }
        if let Some((cand, q, kind, _)) = best {
            return Some((q - cand, SnapHint { point: Some((q, kind)), guides: Vec::new() }));
        }
        // Ausrichtung je Achse an Kanten und Mitten anderer Objekte
        let axis = |values: &[f64], own: [f64; 3]| {
            own.iter()
                .filter_map(|&v| nearest_value(values, v, tol).map(|t| (t, t - v)))
                .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        };
        let x = axis(&self.xs, [b.min.x, c.x, b.max.x]);
        let y = axis(&self.ys, [b.min.y, c.y, b.max.y]);
        if x.is_none() && y.is_none() {
            return None;
        }
        let mut hint = SnapHint::default();
        let mut corr = Point::ZERO;
        if let Some((t, d)) = x {
            corr.x = d;
            hint.guides.push(Guide::Vertical(t));
        }
        if let Some((t, d)) = y {
            corr.y = d;
            hint.guides.push(Guide::Horizontal(t));
        }
        Some((corr, hint))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::VectorPath;

    fn doc() -> (Document, ObjectId, ObjectId) {
        let mut d = Document::default();
        let a = d.add_path("A", 0, VectorPath::rect(0.0, 0.0, 10.0, 10.0, 0.0));
        let b = d.add_path("B", 0, VectorPath::rect(50.0, 0.0, 10.0, 20.0, 0.0));
        (d, a, b)
    }

    #[test]
    fn point_snaps_to_nodes_then_axes() {
        let (d, _, _) = doc();
        let t = SnapTargets::collect(&d, |_| false);
        let (q, hint) = t.snap_point(Point::new(10.3, 9.8), 0.5).unwrap();
        assert_eq!(q, Point::new(10.0, 10.0));
        assert_eq!(hint.point.unwrap().1, SnapKind::Node);
        // weit weg von Punkten, aber auf Höhe der Oberkante von B
        let (q, hint) = t.snap_point(Point::new(30.0, 19.8), 0.5).unwrap();
        assert_eq!(q, Point::new(30.0, 20.0));
        assert_eq!(hint.guides, vec![Guide::Horizontal(20.0)]);
        assert!(t.snap_point(Point::new(30.0, 30.0), 0.5).is_none());
    }

    #[test]
    fn moved_rect_snaps_to_other_object() {
        let (d, a, _) = doc();
        let t = SnapTargets::collect(&d, |id| id == a);
        // A nach rechts geschoben, linke obere Ecke knapp neben der rechten oberen Ecke von B
        let moved = DRect::from_xywh(60.4, 0.3, 10.0, 10.0);
        let (corr, hint) = t.snap_rect(&moved, 1.0).unwrap();
        assert!((corr.x + 0.4).abs() < 1e-9 && (corr.y + 0.3).abs() < 1e-9, "{corr:?}");
        assert!(hint.point.is_some());
    }
}
