//! Andocken: wie weit sich Formen in eine Richtung schieben lassen, bis sie andere Formen berühren.

use crate::geom::Point;
use crate::path::Polyline;

/// Fächer quer zur Schubrichtung, damit nicht jede Ecke gegen jede Kante geprüft wird.
const BUCKETS: usize = 128;

/// Weg (≥ 0) entlang der Einheitsrichtung `dir`, den `moving` zurücklegen kann, bis es eine Kante von
/// `obstacles` berührt (genau an den Konturen, nicht an den Umrandungen). `None`, wenn nichts im Weg liegt.
pub fn contact_distance(moving: &[Polyline], obstacles: &[Polyline], dir: Point) -> Option<f64> {
    let perp = Point::new(-dir.y, dir.x);
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in moving.iter().flat_map(|l| &l.points) {
        let q = p.dot(perp);
        lo = lo.min(q);
        hi = hi.max(q);
    }
    if lo > hi {
        return None;
    }
    let eps = 1e-9 * (1.0 + lo.abs().max(hi.abs()));
    let width = (hi - lo).max(1e-9);
    let bucket = |q: f64| (((q - lo) / width * BUCKETS as f64).floor().max(0.0) as usize).min(BUCKETS - 1);
    // Kanten nach ihrem Querbereich einsortieren; Kanten außerhalb des Streifens können nie getroffen werden
    let sort = |lines: &[Polyline]| {
        let mut b: Vec<Vec<(Point, Point)>> = vec![Vec::new(); BUCKETS];
        for (a, e) in lines.iter().flat_map(|l| l.edges()) {
            let (qa, qe) = (a.dot(perp), e.dot(perp));
            let (q0, q1) = (qa.min(qe) - eps, qa.max(qe) + eps);
            if q1 < lo || q0 > hi {
                continue;
            }
            for slot in &mut b[bucket(q0)..=bucket(q1)] {
                slot.push((a, e));
            }
        }
        b
    };
    let (obstacle_edges, moving_edges) = (sort(obstacles), sort(moving));

    let mut best = f64::INFINITY;
    // Ecken der bewegten Formen laufen vorwärts gegen Hinderniskanten …
    for p in moving.iter().flat_map(|l| &l.points) {
        for &(a, b) in &obstacle_edges[bucket(p.dot(perp))] {
            if let Some(t) = ray_hit(*p, dir, a, b, eps) {
                best = best.min(t);
            }
        }
    }
    // … und Ecken der Hindernisse rückwärts gegen die bewegten Kanten
    let back = Point::new(-dir.x, -dir.y);
    for p in obstacles.iter().flat_map(|l| &l.points) {
        let q = p.dot(perp);
        if q < lo - eps || q > hi + eps {
            continue;
        }
        for &(a, b) in &moving_edges[bucket(q)] {
            if let Some(t) = ray_hit(*p, back, a, b, eps) {
                best = best.min(t);
            }
        }
    }
    best.is_finite().then_some(best.max(0.0))
}

/// Strahl `p + t·d` (t ≥ 0) gegen die Strecke `a`–`b`; parallele Kanten zählen nicht (ihre Enden treffen
/// die Nachbarkanten).
fn ray_hit(p: Point, d: Point, a: Point, b: Point, eps: f64) -> Option<f64> {
    let e = b - a;
    let denom = d.cross(e);
    if denom.abs() < 1e-12 * (1.0 + e.length()) {
        return None;
    }
    let ap = a - p;
    let t = ap.cross(e) / denom;
    let s = ap.cross(d) / denom;
    let tol = eps / e.length().max(1e-12);
    (t >= -eps && (-tol..=1.0 + tol).contains(&s)).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Polyline {
        Polyline::new(
            vec![Point::new(x, y), Point::new(x + w, y), Point::new(x + w, y + h), Point::new(x, y + h)],
            true,
        )
    }

    #[test]
    fn docks_against_shapes() {
        let left = Point::new(-1.0, 0.0);
        // Rechteck rechts eines anderen, gleiche Höhe → genau der Zwischenraum
        let d = contact_distance(&[rect(30.0, 0.0, 10.0, 10.0)], &[rect(0.0, 0.0, 10.0, 10.0)], left);
        assert!((d.unwrap() - 20.0).abs() < 1e-9);
        // Teilweise überlappend in der Höhe
        let d = contact_distance(&[rect(30.0, 8.0, 10.0, 10.0)], &[rect(0.0, 0.0, 10.0, 10.0)], left);
        assert!((d.unwrap() - 20.0).abs() < 1e-9);
        // Nicht im Streifen → nichts im Weg
        assert!(contact_distance(&[rect(30.0, 20.0, 10.0, 10.0)], &[rect(0.0, 0.0, 10.0, 10.0)], left).is_none());
        // Spitze eines Dreiecks gegen eine schräge Kante: an der Kontur, nicht an der Umrandung
        let tri = Polyline::new(vec![Point::new(0.0, 0.0), Point::new(10.0, 5.0), Point::new(0.0, 10.0)], true);
        let d = contact_distance(&[rect(20.0, 1.0, 5.0, 1.0)], std::slice::from_ref(&tri), left).unwrap();
        // Rechteck-Ecke (20, 2) trifft die Kante (0,0)–(10,5) bei x = 4 → 16 mm (Umrandung wäre 10 mm)
        assert!((d - 16.0).abs() < 1e-9, "{d}");
        // Die Dreiecksspitze (10, 5) trifft die linke Rechteckkante
        let d = contact_distance(&[rect(20.0, 4.0, 5.0, 2.0)], &[tri], left).unwrap();
        assert!((d - 10.0).abs() < 1e-9, "{d}");
        // Innen in einem Rahmen: bis zur Innenwand
        let d = contact_distance(&[rect(40.0, 40.0, 10.0, 10.0)], &[rect(5.0, 5.0, 90.0, 90.0)], left).unwrap();
        assert!((d - 35.0).abs() < 1e-9);
    }
}
