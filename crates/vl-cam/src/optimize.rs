//! Reihenfolge-Optimierung für Konturen: kürzeste Leerfahrten und „innen vor außen“.

use std::collections::HashMap;
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
            if let Some((_, bd, ..)) = best
                && rect_distance_sq(&bounds[i], cur) > bd
            {
                continue;
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

/// Entfernt Linienstücke, die ein bereits vorhandenes Stück noch einmal abfahren würde (Abstand höchstens `tol`),
/// z. B. die gemeinsame Kante zweier aneinanderliegender Teile oder doppelt übereinanderliegende Formen. Das erste
/// Vorkommen bleibt; Konturen ohne Überlappung bleiben unverändert, andere werden an den Lücken in offene Stücke
/// geteilt.
pub fn remove_overlaps(polys: Vec<Polyline>, tol: f64) -> Vec<Polyline> {
    let tol = tol.max(1e-6);
    let mut index = SegmentGrid::new((tol * 4.0).max(1.0));
    let mut out = Vec::with_capacity(polys.len());
    for poly in polys {
        let edges: Vec<(Point, Point)> = poly.edges().collect();
        let mut changed = false;
        let mut pieces: Vec<Vec<(f64, f64)>> = Vec::with_capacity(edges.len());
        for &(a, b) in &edges {
            let mut free = vec![(0.0, 1.0)];
            let len = a.distance(b);
            if len > 1e-9 {
                let mut hit = false;
                for (c, d) in index.near(a, b) {
                    if let Some((u0, u1)) = covered(a, b, c, d, tol) {
                        subtract(&mut free, u0, u1);
                        hit = true;
                        if free.is_empty() {
                            break;
                        }
                    }
                }
                if hit {
                    // Reste unterhalb der Toleranz lohnen nicht
                    free.retain(|(t0, t1)| (t1 - t0) * len > tol);
                    changed |= free != [(0.0, 1.0)];
                }
                for &(t0, t1) in &free {
                    index.insert(a.lerp(b, t0), a.lerp(b, t1));
                }
            }
            pieces.push(free);
        }
        if !changed {
            out.push(poly);
            continue;
        }
        let mut runs: Vec<Vec<Point>> = Vec::new();
        let mut cur: Vec<Point> = Vec::new();
        for (&(a, b), free) in edges.iter().zip(&pieces) {
            for &(t0, t1) in free {
                let (p0, p1) = (a.lerp(b, t0), a.lerp(b, t1));
                if cur.last().is_some_and(|l| l.distance(p0) < 1e-9) {
                    cur.push(p1);
                } else {
                    if cur.len() >= 2 {
                        runs.push(std::mem::take(&mut cur));
                    }
                    cur = vec![p0, p1];
                }
            }
        }
        if cur.len() >= 2 {
            runs.push(cur);
        }
        // Bei geschlossenen Konturen schließt das letzte Stück am Startpunkt an das erste an.
        if poly.closed && runs.len() >= 2 && runs[0][0].distance(*runs[runs.len() - 1].last().unwrap()) < 1e-9 {
            let first = runs.remove(0);
            runs.last_mut().unwrap().extend_from_slice(&first[1..]);
        }
        out.extend(runs.into_iter().map(|mut pts| {
            pts.dedup_by(|q, p| q.distance(*p) < 1e-9);
            Polyline::new(pts, false)
        }));
    }
    out
}

/// Abschnitt [u0, u1] der Strecke a–b, den die Strecke c–d abdeckt: c und d liegen höchstens `tol` neben der
/// Geraden durch a und b, dann ist jeder Punkt von a–b im Schattenbereich von c–d höchstens `tol` von c–d entfernt.
fn covered(a: Point, b: Point, c: Point, d: Point, tol: f64) -> Option<(f64, f64)> {
    let ab = b - a;
    let len = ab.dot(ab).sqrt();
    let dir = ab * (1.0 / len);
    let off = |p: Point| (p - a).cross(dir).abs();
    if off(c) > tol || off(d) > tol {
        return None;
    }
    let t = |p: Point| (p - a).dot(dir) / len;
    let (u0, u1) = (t(c).min(t(d)).max(0.0), t(c).max(t(d)).min(1.0));
    (u1 > u0).then_some((u0, u1))
}

/// Zieht [u0, u1] von den freien Abschnitten ab.
fn subtract(free: &mut Vec<(f64, f64)>, u0: f64, u1: f64) {
    let mut out = Vec::with_capacity(free.len() + 1);
    for &(t0, t1) in free.iter() {
        if u1 <= t0 || u0 >= t1 {
            out.push((t0, t1));
            continue;
        }
        if u0 > t0 {
            out.push((t0, u0));
        }
        if u1 < t1 {
            out.push((u1, t1));
        }
    }
    *free = out;
}

/// Gitter über die bisher behaltenen Strecken für die Suche nach Nachbarn.
struct SegmentGrid {
    cell: f64,
    cells: HashMap<(i64, i64), Vec<usize>>,
    segments: Vec<(Point, Point)>,
    /// Zuletzt geliefert bei Abfrage Nummer … (gegen doppelte Treffer)
    seen: Vec<usize>,
    query: usize,
}

impl SegmentGrid {
    fn new(cell: f64) -> Self {
        Self { cell, cells: HashMap::new(), segments: Vec::new(), seen: Vec::new(), query: 0 }
    }

    /// Zellen der Punkte im Abstand einer halben Zelle entlang der Strecke, je mit `radius` Nachbarzellen.
    fn cells_along(&self, a: Point, b: Point, radius: i64) -> Vec<(i64, i64)> {
        let steps = (a.distance(b) / (self.cell / 2.0)).ceil() as usize;
        let mut out: Vec<(i64, i64)> = Vec::new();
        for k in 0..=steps {
            let p = a.lerp(b, if steps == 0 { 0.0 } else { k as f64 / steps as f64 });
            let (cx, cy) = ((p.x / self.cell).floor() as i64, (p.y / self.cell).floor() as i64);
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    out.push((cx + dx, cy + dy));
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    fn insert(&mut self, a: Point, b: Point) {
        let i = self.segments.len();
        self.segments.push((a, b));
        self.seen.push(usize::MAX);
        for c in self.cells_along(a, b, 1) {
            self.cells.entry(c).or_default().push(i);
        }
    }

    /// Strecken, die bis auf eine Zelle an a–b heranreichen.
    fn near(&mut self, a: Point, b: Point) -> Vec<(Point, Point)> {
        self.query += 1;
        let mut out = Vec::new();
        for c in self.cells_along(a, b, 2) {
            for &i in self.cells.get(&c).map(Vec::as_slice).unwrap_or(&[]) {
                if self.seen[i] != self.query {
                    self.seen[i] = self.query;
                    out.push(self.segments[i]);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::{Affine, VectorPath};

    #[test]
    fn overlapping_lines_are_cut_once() {
        let total = |v: &[Polyline]| v.iter().map(|p| p.length()).sum::<f64>();
        let a = poly(VectorPath::rect(0.0, 0.0, 10.0, 10.0, 0.0));
        let b = poly(VectorPath::rect(10.0, 0.0, 10.0, 10.0, 0.0));
        // gemeinsame Kante nur einmal; das erste Teil bleibt geschlossen
        let out = remove_overlaps(vec![a.clone(), b.clone()], 0.05);
        assert!((total(&out) - 70.0).abs() < 1e-9, "{}", total(&out));
        assert!(out[0].closed && out[0] == a);
        assert_eq!(out.len(), 2, "Rest des zweiten Teils als ein offenes Stück");
        assert!(!out[1].closed);
        // doppelte Form fällt ganz weg, auch knapp daneben
        let near = poly(VectorPath::rect(0.03, 0.03, 10.0, 10.0, 0.0));
        let out = remove_overlaps(vec![a.clone(), near.clone()], 0.05);
        assert_eq!(out, vec![a.clone()]);
        // außerhalb der Toleranz bleibt sie
        assert_eq!(remove_overlaps(vec![a.clone(), near.clone()], 0.01).len(), 2);
        // teilweise Überdeckung: nur der überstehende Teil bleibt
        let line = Polyline::new(vec![Point::new(5.0, 0.0), Point::new(20.0, 0.0)], false);
        let out = remove_overlaps(vec![a.clone(), line], 0.05);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].points, vec![Point::new(10.0, 0.0), Point::new(20.0, 0.0)]);
        // Kreise: deckungsgleiche Kopie verschwindet
        let c = poly(VectorPath::ellipse(Point::new(50.0, 50.0), 10.0, 10.0));
        assert_eq!(remove_overlaps(vec![c.clone(), c.clone()], 0.05).len(), 1);
        // ohne Überlappung unverändert
        let far = poly(VectorPath::rect(30.0, 0.0, 10.0, 10.0, 0.0));
        assert_eq!(remove_overlaps(vec![a.clone(), far.clone()], 0.05), vec![a, far]);
    }

    #[test]
    fn grid_of_squares_keeps_each_edge_once() {
        let n = 30;
        let polys: Vec<Polyline> = (0..n * n)
            .map(|i| poly(VectorPath::rect((i % n) as f64 * 5.0, (i / n) as f64 * 5.0, 5.0, 5.0, 0.0)))
            .collect();
        let t = std::time::Instant::now();
        let out = remove_overlaps(polys, 0.05);
        let total: f64 = out.iter().map(|p| p.length()).sum();
        assert!((total - 2.0 * (n * (n + 1)) as f64 * 5.0).abs() < 1e-6, "{total}");
        assert!(t.elapsed().as_secs_f64() < 5.0, "{:?}", t.elapsed());
    }

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
