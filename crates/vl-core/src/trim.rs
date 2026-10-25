//! Trimmen: das Linienstück unter dem Mauszeiger bis zu den nächsten Schnittpunkten entfernen.

use crate::geom::{Affine, Point, Rect};
use crate::path::{Polyline, SubPath, VectorPath};

/// Ergebnis eines Trimmschnitts: das entfernte Stück (für die Vorschau) und die verbleibenden, offenen Teile.
#[derive(Clone, Debug, PartialEq)]
pub struct Trim {
    pub removed: Vec<Point>,
    pub kept: Vec<Polyline>,
}

/// Strecken eines Linienzugs, bei geschlossenen inklusive der Rückkehr zum Start.
fn segments(p: &Polyline) -> Vec<(Point, Point)> {
    let n = p.points.len();
    let m = if p.closed { n } else { n.saturating_sub(1) };
    (0..m).map(|i| (p.points[i], p.points[(i + 1) % n])).collect()
}

/// Parameter `t` auf `a`–`b`, an dem `c`–`d` sie schneidet (parallele Strecken zählen nicht).
fn crossing(a: Point, b: Point, c: Point, d: Point) -> Option<f64> {
    let (r, s) = (b - a, d - c);
    let den = r.cross(s);
    if den.abs() < 1e-12 {
        return None;
    }
    let q = c - a;
    let (t, u) = (q.cross(s) / den, q.cross(r) / den);
    let range = -1e-9..=1.0 + 1e-9;
    (range.contains(&t) && range.contains(&u)).then_some(t.clamp(0.0, 1.0))
}

/// Gleichmäßiges Raster über die Strecken des Ziels, damit nicht jede Strecke mit jeder geprüft wird.
struct Grid {
    origin: Point,
    cell: f64,
    size: usize,
    cells: Vec<Vec<usize>>,
}

impl Grid {
    const SIZE: usize = 64;

    fn new(bounds: Rect, segs: &[(Point, Point)]) -> Self {
        let cell = (bounds.width().max(bounds.height()) / Self::SIZE as f64).max(1e-6);
        let mut grid =
            Grid { origin: bounds.min, cell, size: Self::SIZE, cells: vec![Vec::new(); Self::SIZE * Self::SIZE] };
        for (i, (a, b)) in segs.iter().enumerate() {
            grid.visit(Rect::from_points(*a, *b), |c| c.push(i));
        }
        grid
    }

    fn index(&self, v: f64, o: f64) -> usize {
        (((v - o) / self.cell).floor().max(0.0) as usize).min(self.size - 1)
    }

    fn visit(&mut self, r: Rect, mut f: impl FnMut(&mut Vec<usize>)) {
        let (x0, x1) = (self.index(r.min.x, self.origin.x), self.index(r.max.x, self.origin.x));
        let (y0, y1) = (self.index(r.min.y, self.origin.y), self.index(r.max.y, self.origin.y));
        for y in y0..=y1 {
            for x in x0..=x1 {
                f(&mut self.cells[y * self.size + x]);
            }
        }
    }

    /// Strecken des Ziels, deren Zellen das Rechteck berührt (ohne Doppelte).
    fn candidates(&mut self, r: Rect, stamp: &mut [usize], round: usize, out: &mut Vec<usize>) {
        out.clear();
        self.visit(r, |c| {
            for &i in c.iter() {
                if stamp[i] != round {
                    stamp[i] = round;
                    out.push(i);
                }
            }
        });
    }
}

/// Entfernt aus `target` das Stück um `at` zwischen den nächsten Schnittpunkten mit `cutters` oder sich selbst.
/// Ohne Schnittpunkte verschwindet der ganze Linienzug.
pub fn trim_at(target: &Polyline, cutters: &[&Polyline], at: Point) -> Trim {
    let segs = segments(target);
    let mut ring = target.points.clone();
    if target.closed && !ring.is_empty() {
        ring.push(ring[0]);
    }
    let whole = || Trim { removed: ring.clone(), kept: Vec::new() };
    if segs.is_empty() {
        return whole();
    }
    let mut cum = vec![0.0];
    for (a, b) in &segs {
        cum.push(cum[cum.len() - 1] + a.distance(*b));
    }
    let total = cum[cum.len() - 1];
    if total <= 1e-9 {
        return whole();
    }

    let bounds = target.bounds().expanded(1e-6);
    let mut grid = Grid::new(bounds, &segs);
    let mut stamp = vec![usize::MAX; segs.len()];
    let (mut round, mut found) = (0, Vec::new());
    let mut cuts = Vec::new();
    let mut probe = |c: Point, d: Point, skip: &dyn Fn(usize) -> bool, cuts: &mut Vec<f64>| {
        let r = Rect::from_points(c, d).expanded(1e-9);
        if !r.intersects(&bounds) {
            return;
        }
        round += 1;
        grid.candidates(r, &mut stamp, round, &mut found);
        for &i in &found {
            let (a, b) = segs[i];
            if !skip(i)
                && let Some(t) = crossing(a, b, c, d)
            {
                cuts.push(cum[i] + t * (cum[i + 1] - cum[i]));
            }
        }
    };
    for cutter in cutters {
        if !cutter.bounds().intersects(&bounds) {
            continue;
        }
        for (c, d) in segments(cutter) {
            probe(c, d, &|_| false, &mut cuts);
        }
    }
    // Selbstüberschneidungen, ohne die Nachbarstrecken (die teilen nur ihren Endpunkt)
    let m = segs.len();
    for (j, &(c, d)) in segs.iter().enumerate() {
        let adjacent =
            |i: usize| i == j || i + 1 == j || j + 1 == i || (target.closed && ((i + 1) % m == j || (j + 1) % m == i));
        probe(c, d, &adjacent, &mut cuts);
    }
    if !target.closed {
        cuts.retain(|&s| s > 1e-6 && s < total - 1e-6);
    }
    cuts.sort_by(f64::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    if target.closed && cuts.len() >= 2 && cuts[0] + total - cuts[cuts.len() - 1] < 1e-6 {
        cuts.pop();
    }

    // Bogenlänge des Punkts auf dem Linienzug, der `at` am nächsten liegt
    let mut best = (f64::INFINITY, 0.0);
    for (i, (a, b)) in segs.iter().enumerate() {
        let ab = *b - *a;
        let len2 = ab.dot(ab);
        let t = if len2 > 0.0 { ((at - *a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
        let d = at.distance(*a + ab * t);
        if d < best.0 {
            best = (d, cum[i] + t * (cum[i + 1] - cum[i]));
        }
    }
    let s0 = best.1;

    let point_at = |s: f64| {
        let i = cum.partition_point(|&c| c <= s).clamp(1, cum.len() - 1) - 1;
        let len = cum[i + 1] - cum[i];
        let t = if len > 0.0 { ((s - cum[i]) / len).clamp(0.0, 1.0) } else { 0.0 };
        ring[i].lerp(ring[i + 1], t)
    };
    let sub = |a: f64, b: f64| {
        let mut pts = vec![point_at(a)];
        pts.extend((0..ring.len()).filter(|&k| cum[k] > a + 1e-9 && cum[k] < b - 1e-9).map(|k| ring[k]));
        pts.push(point_at(b));
        pts
    };
    let piece = |pts: Vec<Point>| (pts.len() >= 2).then(|| Polyline::new(pts, false));

    if !target.closed {
        let lo = cuts.iter().rev().find(|&&s| s < s0).copied().unwrap_or(0.0);
        let hi = cuts.iter().find(|&&s| s > s0).copied().unwrap_or(total);
        let mut kept = Vec::new();
        if lo > 1e-6 {
            kept.extend(piece(sub(0.0, lo)));
        }
        if hi < total - 1e-6 {
            kept.extend(piece(sub(hi, total)));
        }
        return Trim { removed: sub(lo, hi), kept };
    }
    // Geschlossen: ein einzelner Schnittpunkt trennt nichts ab
    if cuts.len() < 2 {
        return whole();
    }
    let lo = cuts.iter().rev().find(|&&s| s < s0).or(cuts.last()).copied().unwrap_or(0.0);
    let hi = cuts.iter().find(|&&s| s > s0).or(cuts.first()).copied().unwrap_or(total);
    let wrap = |a: f64, b: f64| {
        if b > a {
            sub(a, b)
        } else {
            let mut pts = sub(a, total);
            pts.extend(sub(0.0, b).into_iter().skip(1));
            pts
        }
    };
    Trim { removed: wrap(lo, hi), kept: piece(wrap(hi, lo)).into_iter().collect() }
}

/// Ersetzt im Pfad den `polyline`-ten abgeflachten Linienzug (der Teilpfad mit Strecken an dieser Stelle, so zählt
/// `flatten`) durch die Stücke `kept`. Die Stücke liegen in Dokumentkoordinaten, `to_local` rechnet sie ins
/// Objektsystem zurück; die übrigen Teilpfade bleiben unverändert, auch ihre Kurven. `None`, wenn es den Linienzug
/// nicht gibt.
pub fn replace_polyline(
    path: &VectorPath,
    polyline: usize,
    kept: &[Polyline],
    to_local: &Affine,
) -> Option<VectorPath> {
    let index =
        path.subpaths.iter().enumerate().filter(|(_, sp)| !sp.segments.is_empty()).nth(polyline).map(|(i, _)| i)?;
    let pieces: Vec<SubPath> = kept
        .iter()
        .flat_map(|k| {
            let pts: Vec<Point> = k.points.iter().map(|p| to_local.apply(*p)).collect();
            VectorPath::polygon(&pts, k.closed).subpaths
        })
        .collect();
    let mut subpaths = path.subpaths.clone();
    subpaths.splice(index..=index, pieces);
    subpaths.retain(|sp| !sp.segments.is_empty());
    Some(VectorPath { subpaths })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(a: (f64, f64), b: (f64, f64)) -> Polyline {
        Polyline::new(vec![Point::new(a.0, a.1), Point::new(b.0, b.1)], false)
    }

    fn length(pts: &[Point]) -> f64 {
        pts.windows(2).map(|w| w[0].distance(w[1])).sum()
    }

    #[test]
    fn trims_line_between_crossings() {
        // Waagerechte 0…10, gekreuzt bei x = 3 und x = 7
        let target = line((0.0, 0.0), (10.0, 0.0));
        let (c1, c2) = (line((3.0, -1.0), (3.0, 1.0)), line((7.0, -1.0), (7.0, 1.0)));
        let t = trim_at(&target, &[&c1, &c2], Point::new(5.0, 0.2));
        assert!((length(&t.removed) - 4.0).abs() < 1e-9);
        assert_eq!(t.kept.len(), 2);
        assert!((length(&t.kept[0].points) - 3.0).abs() < 1e-9);
        assert!((length(&t.kept[1].points) - 3.0).abs() < 1e-9);
        // Klick auf das Endstück: nur eines bleibt
        let end = trim_at(&target, &[&c1, &c2], Point::new(9.0, 0.0));
        assert_eq!(end.kept.len(), 1);
        assert!((length(&end.kept[0].points) - 7.0).abs() < 1e-9);
        // ohne Schnittpunkte verschwindet alles
        assert!(trim_at(&target, &[], Point::new(5.0, 0.0)).kept.is_empty());
    }

    #[test]
    fn trims_overlapping_squares() {
        // Zwei überlappende Quadrate: das Stück des ersten im zweiten entfernen
        let sq = |x: f64, y: f64| {
            Polyline::new(
                vec![
                    Point::new(x, y),
                    Point::new(x + 10.0, y),
                    Point::new(x + 10.0, y + 10.0),
                    Point::new(x, y + 10.0),
                ],
                true,
            )
        };
        let (a, b) = (sq(0.0, 0.0), sq(5.0, 5.0));
        let t = trim_at(&a, &[&b], Point::new(10.0, 8.0));
        // Ecke (10,10) mit je 5 mm in beide Richtungen
        assert!((length(&t.removed) - 10.0).abs() < 1e-9, "{}", length(&t.removed));
        assert_eq!(t.kept.len(), 1);
        assert!((length(&t.kept[0].points) - 30.0).abs() < 1e-9);
        assert!(!t.kept[0].closed);
        let ends = (t.kept[0].points[0], *t.kept[0].points.last().unwrap());
        assert!(ends.0.distance(Point::new(5.0, 10.0)) < 1e-9 || ends.1.distance(Point::new(5.0, 10.0)) < 1e-9);
    }

    #[test]
    fn replaces_trimmed_subpath_in_object_space() {
        // Objekt: Kreis (Kurven) und Quadrat, doppelt so groß und verschoben
        let mut path = VectorPath::ellipse(Point::new(20.0, 5.0), 4.0, 4.0);
        path.append(VectorPath::rect(0.0, 0.0, 10.0, 10.0, 0.0));
        let t = Affine::scale(2.0, 2.0).then(&Affine::translate(100.0, 50.0));
        let world = path.flatten(&t, 0.01);
        assert_eq!(world.len(), 2);
        // Quer durch das Quadrat (Weltmaß 20 mm) eine Linie bei x = 110
        let cutter = Polyline::new(vec![Point::new(110.0, 40.0), Point::new(110.0, 80.0)], false);
        let trim = trim_at(&world[1], &[&cutter], Point::new(100.0, 60.0));
        assert!((length(&trim.removed) - 40.0).abs() < 1e-6, "{}", length(&trim.removed));
        let out = replace_polyline(&path, 1, &trim.kept, &t.inverse().unwrap()).unwrap();
        // der Kreis bleibt mit seinen Kurven erhalten
        assert_eq!(out.subpaths[0], path.subpaths[0]);
        let after = out.flatten(&t, 0.01);
        assert_eq!(after.len(), 2);
        assert!(!after[1].closed);
        assert!((length(&after[1].points) - 40.0).abs() < 1e-6);
        // nicht vorhandener Linienzug
        assert!(replace_polyline(&path, 5, &[], &Affine::IDENTITY).is_none());
        // alles entfernt: leerer Pfad
        let single = VectorPath::rect(0.0, 0.0, 1.0, 1.0, 0.0);
        assert!(replace_polyline(&single, 0, &[], &Affine::IDENTITY).unwrap().subpaths.is_empty());
    }

    #[test]
    fn trims_self_intersection() {
        // Schleife: Linienzug kreuzt sich selbst bei (5, 0)
        let target = Polyline::new(
            vec![
                Point::new(0.0, 0.0),
                Point::new(10.0, 0.0),
                Point::new(10.0, 5.0),
                Point::new(5.0, 5.0),
                Point::new(5.0, -5.0),
            ],
            false,
        );
        let t = trim_at(&target, &[], Point::new(10.0, 2.0));
        // Schleife (5,0)→(10,0)→(10,5)→(5,5)→(5,0) = 5 + 5 + 5 + 5
        assert!((length(&t.removed) - 20.0).abs() < 1e-9, "{}", length(&t.removed));
        assert_eq!(t.kept.len(), 2);
    }
}
