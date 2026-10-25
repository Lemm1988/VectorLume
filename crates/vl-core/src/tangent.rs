//! Tangentialkreise (Problem des Apollonius): alle Kreise, die drei gegebene Kreise je in einem Punkt berühren –
//! von außen oder von innen, bis zu acht Lösungen.

use crate::geom::Point;

/// Ein Kreis als (Mittelpunkt, Radius).
pub type Circle = (Point, f64);

/// Alle Kreise mit Radius > 0, die `c[0]`, `c[1]` und `c[2]` berühren, nach Radius sortiert.
pub fn tangent_circles(c: [Circle; 3]) -> Vec<Circle> {
    // Zwei Kreise mit verschiedenen Mittelpunkten legen das Koordinatensystem fest
    let scale = c.iter().map(|(p, r)| p.x.abs().max(p.y.abs()).max(*r)).fold(1.0, f64::max);
    let eps = 1e-9 * scale;
    let Some(order) = [[0, 1, 2], [1, 2, 0], [0, 2, 1]].into_iter().find(|o| c[o[0]].0.distance(c[o[1]].0) > eps)
    else {
        return Vec::new();
    };
    let [c1, c2, c3] = order.map(|i| c[i]);
    // Rahmen: c1 im Ursprung, c2 auf der positiven x-Achse
    let dir = (c2.0 - c1.0) * (1.0 / c1.0.distance(c2.0));
    let to_local = |p: Point| {
        let d = p - c1.0;
        Point::new(d.x * dir.x + d.y * dir.y, -d.x * dir.y + d.y * dir.x)
    };
    let to_world = |p: Point| c1.0 + Point::new(p.x * dir.x - p.y * dir.y, p.x * dir.y + p.y * dir.x);
    let (p2, p3) = (to_local(c2.0), to_local(c3.0));
    let (r1, r2, r3) = (c1.1, c2.1, c3.1);

    let mut out: Vec<Circle> = Vec::new();
    for signs in 0..8 {
        let s = |bit: u32| if signs & (1 << bit) != 0 { -1.0 } else { 1.0 };
        let (s1, s2, s3) = (s(0), s(1), s(2));
        // |P - Ci|² = (r + si·ri)²; Differenz zu Kreis 1 ist linear in x, y, r:  a·x + b·y + k·r = d
        let lin = |p: Point, ri: f64, si: f64| {
            (2.0 * p.x, 2.0 * p.y, 2.0 * (si * ri - s1 * r1), p.x * p.x + p.y * p.y - ri * ri + r1 * r1)
        };
        let (a2, _, k2, d2) = lin(p2, r2, s2);
        let (a3, b3, k3, d3) = lin(p3, r3, s3);
        let mut found: Vec<(Point, f64)> = Vec::new();
        if b3.abs() > eps {
            // x = (d2 - k2·r)/a2, y aus Gleichung 3; eingesetzt in Kreis 1 ergibt sich eine quadratische Gleichung
            let (x0, x1) = (d2 / a2, -k2 / a2);
            let (y0, y1) = ((d3 - a3 * x0) / b3, (-k3 - a3 * x1) / b3);
            let qa = x1 * x1 + y1 * y1 - 1.0;
            let qb = 2.0 * (x0 * x1 + y0 * y1 - s1 * r1);
            let qc = x0 * x0 + y0 * y0 - r1 * r1;
            for r in roots(qa, qb, qc) {
                found.push((Point::new(x0 + x1 * r, y0 + y1 * r), r));
            }
        } else {
            // Mittelpunkte auf einer Geraden: x und r aus den beiden linearen Gleichungen, y aus Kreis 1
            let det = a2 * k3 - a3 * k2;
            if det.abs() > eps * eps {
                let x = (d2 * k3 - d3 * k2) / det;
                let r = (a2 * d3 - a3 * d2) / det;
                let y2 = (r + s1 * r1).powi(2) - x * x;
                if y2 >= -eps {
                    let y = y2.max(0.0).sqrt();
                    found.push((Point::new(x, y), r));
                    found.push((Point::new(x, -y), r));
                }
            }
        }
        for (p, r) in found {
            if r <= eps || !r.is_finite() {
                continue;
            }
            // Probe an allen drei Kreisen (sonst landen Scheinlösungen aus Rundungsfehlern im Ergebnis)
            let ok = [(Point::ZERO, r1, s1), (p2, r2, s2), (p3, r3, s3)]
                .iter()
                .all(|(q, ri, si)| (p.distance(*q) - (r + si * ri).abs()).abs() <= 1e-6 * scale);
            let world = (to_world(p), r);
            if ok && !out.iter().any(|(q, rr)| q.distance(world.0) <= 1e-6 * scale && (rr - r).abs() <= 1e-6 * scale) {
                out.push(world);
            }
        }
    }
    out.sort_by(|a, b| a.1.total_cmp(&b.1));
    out
}

fn roots(a: f64, b: f64, c: f64) -> Vec<f64> {
    if a.abs() < 1e-12 {
        return if b.abs() < 1e-12 { Vec::new() } else { vec![-c / b] };
    }
    let disc = b * b - 4.0 * a * c;
    if disc < -1e-12 * b * b.max(1.0) {
        return Vec::new();
    }
    let s = disc.max(0.0).sqrt();
    vec![(-b + s) / (2.0 * a), (-b - s) / (2.0 * a)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touches(sol: Circle, c: Circle) -> bool {
        let d = sol.0.distance(c.0);
        (d - (sol.1 + c.1)).abs() < 1e-7 || (d - (sol.1 - c.1).abs()).abs() < 1e-7
    }

    #[test]
    fn three_separate_circles_have_eight_solutions() {
        let h = 4.0 * 3f64.sqrt() / 2.0;
        let c = [(Point::new(0.0, 0.0), 1.0), (Point::new(4.0, 0.0), 1.0), (Point::new(2.0, h), 1.0)];
        let sols = tangent_circles(c);
        assert_eq!(sols.len(), 8, "{sols:?}");
        for s in &sols {
            assert!(c.iter().all(|ci| touches(*s, *ci)), "{s:?}");
        }
        // kleinster: im Schwerpunkt, außen berührend; größter: umschließt alle drei
        let centroid = Point::new(2.0, h / 3.0);
        let rc = centroid.distance(c[0].0);
        assert!(sols[0].0.distance(centroid) < 1e-9 && (sols[0].1 - (rc - 1.0)).abs() < 1e-9);
        assert!(sols[7].0.distance(centroid) < 1e-9 && (sols[7].1 - (rc + 1.0)).abs() < 1e-9);
    }

    #[test]
    fn unequal_and_rotated_circles() {
        let c = [(Point::new(10.0, 5.0), 2.0), (Point::new(17.0, 9.0), 1.5), (Point::new(12.0, 16.0), 3.0)];
        let sols = tangent_circles(c);
        assert_eq!(sols.len(), 8);
        assert!(sols.iter().all(|s| c.iter().all(|ci| touches(*s, *ci))));
        // Reihenfolge der Eingabe spielt keine Rolle
        let again = tangent_circles([c[2], c[0], c[1]]);
        assert_eq!(again.len(), 8);
        for (a, b) in sols.iter().zip(&again) {
            assert!(a.0.distance(b.0) < 1e-7 && (a.1 - b.1).abs() < 1e-7);
        }
    }

    #[test]
    fn collinear_centers_and_degenerate_cases() {
        let c = [(Point::new(0.0, 0.0), 1.0), (Point::new(3.0, 0.0), 1.0), (Point::new(6.0, 0.0), 1.0)];
        let sols = tangent_circles(c);
        assert!(sols.len() >= 2, "{sols:?}");
        assert!(sols.iter().all(|s| c.iter().all(|ci| touches(*s, *ci))));
        // spiegelbildlich zur Achse der Mittelpunkte
        assert!(sols.iter().all(|s| sols.iter().any(|t| t.0.distance(Point::new(s.0.x, -s.0.y)) < 1e-9)));
        // drei Kreise um denselben Mittelpunkt: keine eindeutige Lösung
        let p = Point::new(1.0, 1.0);
        assert!(tangent_circles([(p, 1.0), (p, 2.0), (p, 3.0)]).is_empty());
        // ein Kreis liegt im anderen: es gibt trotzdem Lösungen, alle berühren
        let c = [(Point::new(0.0, 0.0), 5.0), (Point::new(1.0, 0.0), 1.0), (Point::new(-2.0, 1.0), 1.0)];
        let sols = tangent_circles(c);
        assert!(!sols.is_empty() && sols.iter().all(|s| c.iter().all(|ci| touches(*s, *ci))));
    }
}
