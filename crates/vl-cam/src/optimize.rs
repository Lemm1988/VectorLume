//! Reihenfolge-Optimierung für Konturen: kürzeste Leerfahrten und „innen vor außen“.

use vl_core::{Point, Polyline, Rect};

/// Ab dieser Anzahl wird auf die (quadratische) Verschachtelungsprüfung verzichtet.
const NESTING_LIMIT: usize = 4000;

/// Sortiert Polylinien nach dem Prinzip „nächster Nachbar“. Mit `inner_first` wird eine geschlossene
/// Kontur erst geschnitten, wenn alle in ihr liegenden Konturen fertig sind (Teile fallen nicht vorher heraus).
pub fn order_polylines(polys: Vec<Polyline>, start: Point, inner_first: bool) -> Vec<Polyline> {
    let n = polys.len();
    if n == 0 {
        return polys;
    }
    let bounds: Vec<Rect> = polys.iter().map(|p| p.bounds()).collect();
    // blockers[i] = Anzahl noch offener Konturen innerhalb von i
    let mut blockers = vec![0usize; n];
    let mut parents: Vec<Vec<usize>> = vec![Vec::new(); n];
    if inner_first && n <= NESTING_LIMIT {
        for outer in 0..n {
            if !polys[outer].closed {
                continue;
            }
            for inner in 0..n {
                if inner == outer || !bounds[outer].contains_rect(&bounds[inner]) {
                    continue;
                }
                // Gleich große Umrandungen nur in einer Richtung zählen, sonst blockieren sie sich gegenseitig.
                if bounds[inner] == bounds[outer] && inner > outer {
                    continue;
                }
                let probe = polys[inner].points[0];
                if vl_core::path::point_in_polylines(probe, std::slice::from_ref(&polys[outer])) {
                    blockers[outer] += 1;
                    parents[inner].push(outer);
                }
            }
        }
    }

    let mut done = vec![false; n];
    let mut slots: Vec<Option<Polyline>> = polys.into_iter().map(Some).collect();
    let mut out = Vec::with_capacity(n);
    let mut cur = start;
    for _ in 0..n {
        let mut best: Option<(usize, f64, usize, bool)> = None;
        for i in 0..n {
            if done[i] || blockers[i] > 0 {
                continue;
            }
            let p = slots[i].as_ref().unwrap();
            // Grobe Vorauswahl über die Umrandung spart bei vielen Konturen viel Zeit.
            if let Some((_, bd, ..)) = best {
                if rect_distance_sq(&bounds[i], cur) > bd {
                    continue;
                }
            }
            let (d, idx, rev) = best_entry(p, cur);
            if best.is_none_or(|(_, bd, ..)| d < bd) {
                best = Some((i, d, idx, rev));
            }
        }
        let (i, _, idx, rev) = best.expect("Verschachtelung ohne Zyklen hat immer eine freie Kontur");
        done[i] = true;
        for &parent in &parents[i] {
            blockers[parent] -= 1;
        }
        let mut p = slots[i].take().unwrap();
        if p.closed {
            p.points.rotate_left(idx);
        } else if rev {
            p.points.reverse();
        }
        cur = if p.closed { p.points[0] } else { *p.points.last().unwrap() };
        out.push(p);
    }
    out
}

/// Günstigster Einstieg: bei geschlossenen Konturen jeder Punkt, bei offenen Anfang oder Ende.
fn best_entry(p: &Polyline, cur: Point) -> (f64, usize, bool) {
    if p.closed {
        let (idx, d) = p
            .points
            .iter()
            .enumerate()
            .map(|(i, q)| (i, q.distance_sq(cur)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        (d, idx, false)
    } else {
        let ds = p.points[0].distance_sq(cur);
        let de = p.points.last().unwrap().distance_sq(cur);
        if de < ds { (de, 0, true) } else { (ds, 0, false) }
    }
}

fn rect_distance_sq(r: &Rect, p: Point) -> f64 {
    let dx = (r.min.x - p.x).max(0.0).max(p.x - r.max.x);
    let dy = (r.min.y - p.y).max(0.0).max(p.y - r.max.y);
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::{Affine, VectorPath};

    fn poly(path: VectorPath) -> Polyline {
        path.flatten(&Affine::IDENTITY, 0.01).remove(0)
    }

    #[test]
    fn inner_shapes_come_first() {
        let outer = poly(VectorPath::rect(0.0, 0.0, 100.0, 100.0, 0.0));
        let hole = poly(VectorPath::ellipse(Point::new(50.0, 50.0), 10.0, 10.0));
        let ordered = order_polylines(vec![outer.clone(), hole.clone()], Point::ZERO, true);
        assert_eq!(ordered[0].points.len(), hole.points.len());
        let unordered = order_polylines(vec![outer, hole.clone()], Point::ZERO, false);
        assert_ne!(unordered[0].points.len(), hole.points.len());
    }

    #[test]
    fn nearest_neighbour_and_reversal() {
        let a = Polyline::new(vec![Point::new(10.0, 0.0), Point::new(20.0, 0.0)], false);
        let b = Polyline::new(vec![Point::new(50.0, 0.0), Point::new(21.0, 0.0)], false);
        let ordered = order_polylines(vec![b, a], Point::ZERO, true);
        assert_eq!(ordered[0].points[0], Point::new(10.0, 0.0));
        // b wird umgedreht, damit es am nahen Ende beginnt.
        assert_eq!(ordered[1].points[0], Point::new(21.0, 0.0));
    }

    #[test]
    fn closed_contour_starts_at_nearest_vertex() {
        let sq = poly(VectorPath::rect(10.0, 10.0, 10.0, 10.0, 0.0));
        let ordered = order_polylines(vec![sq], Point::new(25.0, 25.0), true);
        assert_eq!(ordered[0].points[0], Point::new(20.0, 20.0));
    }
}
