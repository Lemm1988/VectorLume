//! Geometrie-Operationen auf Polylinien: Boolesche Operationen, Verschweißen, Versatz, Versatz-Füllen und
//! Laschen. Die Polygon-Arithmetik übernimmt `i_overlay` (robust auch bei Selbstüberschneidungen).

use crate::geom::Point;
use crate::path::Polyline;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::stroke::offset::StrokeOffset;
use i_overlay::mesh::float::style::{LineCap, LineJoin, OutlineStyle, StrokeStyle};
use serde::{Deserialize, Serialize};

type Contour = Vec<[f64; 2]>;
type Shapes = Vec<Vec<Contour>>;

/// Maximale Abweichung bei der Annäherung runder Ecken (mm).
const ARC_TOLERANCE: f64 = 0.01;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    Union,
    Difference,
    Intersection,
    Xor,
}

impl BoolOp {
    pub fn label(self) -> &'static str {
        match self {
            BoolOp::Union => "Vereinigen",
            BoolOp::Difference => "Abziehen",
            BoolOp::Intersection => "Schnittmenge",
            BoolOp::Xor => "Ausschließen",
        }
    }
}

/// Eckenform beim Versatz.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Corner {
    #[default]
    Round,
    Sharp,
    Bevel,
}

impl Corner {
    pub const ALL: [Corner; 3] = [Corner::Round, Corner::Sharp, Corner::Bevel];

    pub fn label(self) -> &'static str {
        match self {
            Corner::Round => "Rund",
            Corner::Sharp => "Spitz",
            Corner::Bevel => "Abgeschrägt",
        }
    }

    fn join(self, distance: f64) -> LineJoin<f64> {
        match self {
            // Parameter = Segmentlänge / Radius; aus der Sehnenabweichung r·θ²/8 ≤ Toleranz.
            Corner::Round => LineJoin::Round((8.0 * ARC_TOLERANCE / distance.abs().max(1e-6)).sqrt()),
            // Spitzen unter 20° werden abgeschrägt, sonst würden sie sehr weit hinausragen.
            Corner::Sharp => LineJoin::Miter(20f64.to_radians()),
            Corner::Bevel => LineJoin::Bevel,
        }
    }
}

fn contours(polys: &[Polyline]) -> Vec<Contour> {
    polys
        .iter()
        .filter(|p| p.closed && p.points.len() >= 3)
        .map(|p| p.points.iter().map(|q| [q.x, q.y]).collect())
        .collect()
}

fn to_polylines(shapes: Shapes) -> Vec<Polyline> {
    shapes
        .into_iter()
        .flatten()
        .filter(|c| c.len() >= 3)
        .map(|c| Polyline::new(c.into_iter().map(|[x, y]| Point::new(x, y)).collect(), true))
        .collect()
}

/// Bereinigt geschlossene Polylinien nach der Gerade-Ungerade-Regel (wie beim Füllen):
/// Selbstüberschneidungen werden aufgelöst, Außenkonturen und Löcher einheitlich orientiert.
fn normalize(polys: &[Polyline]) -> Shapes {
    let c = contours(polys);
    if c.is_empty() { Vec::new() } else { c.simplify_shape(FillRule::EvenOdd) }
}

/// Boolesche Operation zwischen zwei Flächen (jeweils nach der Gerade-Ungerade-Regel gefüllt).
pub fn boolean(subject: &[Polyline], clip: &[Polyline], op: BoolOp) -> Vec<Polyline> {
    let s = contours(subject);
    let c = contours(clip);
    let rule = match op {
        BoolOp::Union => OverlayRule::Union,
        BoolOp::Difference => OverlayRule::Difference,
        BoolOp::Intersection => OverlayRule::Intersect,
        BoolOp::Xor => OverlayRule::Xor,
    };
    if s.is_empty() && c.is_empty() {
        return Vec::new();
    }
    to_polylines(s.overlay(&c, rule, FillRule::EvenOdd))
}

/// Verschweißt mehrere Objekte zu einer Fläche. Jedes Objekt wird für sich nach der Gerade-Ungerade-Regel
/// ausgewertet (Löcher eines Buchstabens bleiben), die Objekte untereinander werden vereinigt.
pub fn weld(objects: &[Vec<Polyline>]) -> Vec<Polyline> {
    let mut all: Vec<Contour> = Vec::new();
    for o in objects {
        all.extend(normalize(o).into_iter().flatten());
    }
    if all.is_empty() {
        return Vec::new();
    }
    // Nach der Normalisierung sind alle Außenkonturen gleich orientiert: Nicht-Null-Regel = Vereinigung.
    to_polylines(all.simplify_shape(FillRule::NonZero))
}

/// Vereinigt Konturen nach der Nicht-Null-Regel (wie Schriften gefüllt werden): überlappende Buchstaben werden eine
/// Fläche, Löcher bleiben.
pub fn union_nonzero(polys: &[Polyline]) -> Vec<Polyline> {
    let c = contours(polys);
    if c.is_empty() { Vec::new() } else { to_polylines(c.simplify_shape(FillRule::NonZero)) }
}

/// Versatz: positive Werte vergrößern Flächen (Löcher werden kleiner), negative verkleinern sie.
/// Offene Pfade werden mit dem Betrag des Abstands umrandet.
pub fn offset(polys: &[Polyline], distance: f64, corner: Corner) -> Vec<Polyline> {
    let mut out = Vec::new();
    if distance.abs() < 1e-9 {
        return polys.to_vec();
    }
    let shapes = normalize(polys);
    if !shapes.is_empty() {
        let style = OutlineStyle::new(distance).line_join(corner.join(distance));
        out.extend(to_polylines(shapes.outline(&style)));
    }
    let d = distance.abs();
    for p in polys.iter().filter(|p| !p.closed && p.points.len() >= 2) {
        let path: Contour = p.points.iter().map(|q| [q.x, q.y]).collect();
        let cap = match corner {
            Corner::Round => LineCap::Round((8.0 * ARC_TOLERANCE / d).sqrt()),
            Corner::Sharp => LineCap::Square,
            Corner::Bevel => LineCap::Butt,
        };
        let style = StrokeStyle::new(2.0 * d).line_join(corner.join(d)).start_cap(cap.clone()).end_cap(cap);
        out.extend(to_polylines(path.stroke(style, false)));
    }
    out
}

/// Zerlegt geschlossene Konturen in Inseln: jede Außenkontur (gerade Verschachtelungstiefe) mit ihren direkten
/// Löchern. Eine Form in einem Loch ist wieder eine eigene Insel. Offene Linienzüge entfallen.
pub fn islands(polys: &[Polyline]) -> Vec<Vec<Polyline>> {
    let rings: Vec<&Polyline> = polys.iter().filter(|p| p.closed && p.points.len() >= 3).collect();
    let n = rings.len();
    let (depth, parent) = nesting(&rings);
    let mut island_of: Vec<Option<usize>> = vec![None; n];
    let mut out: Vec<Vec<Polyline>> = Vec::new();
    for i in (0..n).filter(|&i| depth[i].is_multiple_of(2)) {
        island_of[i] = Some(out.len());
        out.push(vec![rings[i].clone()]);
    }
    for i in (0..n).filter(|&i| !depth[i].is_multiple_of(2)) {
        match parent[i].and_then(|p| island_of[p]) {
            Some(k) => out[k].push(rings[i].clone()),
            None => out.push(vec![rings[i].clone()]),
        }
    }
    out
}

/// Douglas-Peucker ohne Rekursion: Punkte, die höchstens `tol` von der Verbindung ihrer Nachbarn abweichen,
/// entfallen. Bei geschlossenen Polygonen dient der vom Start am weitesten entfernte Punkt als zweiter Anker.
pub fn simplify_points(pts: &[Point], closed: bool, tol: f64) -> Vec<Point> {
    let n = pts.len();
    if n < if closed { 4 } else { 3 } || tol <= 0.0 {
        return pts.to_vec();
    }
    let at = |i: usize| pts[i % n];
    let mut keep = vec![false; n + 1];
    let mut stack = Vec::new();
    keep[0] = true;
    if closed {
        let far =
            (1..n).max_by(|&a, &b| pts[0].distance_sq(pts[a]).total_cmp(&pts[0].distance_sq(pts[b]))).unwrap_or(n / 2);
        keep[far] = true;
        keep[n] = true;
        stack.extend([(0usize, far), (far, n)]);
    } else {
        keep[n - 1] = true;
        stack.push((0usize, n - 1));
    }
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (pa, pb) = (at(a), at(b));
        let mut best = (0.0, a);
        for i in a + 1..b {
            let d = crate::path::distance_to_segment(at(i), pa, pb);
            if d > best.0 {
                best = (d, i);
            }
        }
        if best.0 > tol {
            keep[best.1] = true;
            stack.push((a, best.1));
            stack.push((best.1, b));
        }
    }
    (0..n).filter(|&i| keep[i]).map(|i| pts[i]).collect()
}

/// Punktfolge → Pfad. Mit `smoothing` (0…1) werden flache Knicke zu Catmull-Rom-Kurven; Knicke über
/// `corner_deg` bleiben Ecken, ebenso die Enden offener Linienzüge.
pub fn smooth_points(pts: &[Point], closed: bool, smoothing: f64, corner_deg: f64) -> crate::path::VectorPath {
    let n = pts.len();
    let mut pb = crate::path::PathBuilder::new();
    if n == 0 {
        return pb.finish();
    }
    pb.move_to(pts[0]);
    let k = smoothing.clamp(0.0, 1.0);
    if k <= 0.0 || n < 3 {
        for p in &pts[1..] {
            pb.line_to(*p);
        }
        if closed {
            pb.close();
        }
        return pb.finish();
    }
    let limit = corner_deg.to_radians();
    let corner = |i: usize| {
        if !closed && (i == 0 || i == n - 1) {
            return true;
        }
        let (a, b, c) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
        let (u, v) = (b - a, c - b);
        u.cross(v).atan2(u.dot(v)).abs() > limit
    };
    let corners: Vec<bool> = (0..n).map(corner).collect();
    // Tangente je Knoten (null an Ecken)
    let tangent = |i: usize| {
        if corners[i] { Point::ZERO } else { (pts[(i + 1) % n] - pts[(i + n - 1) % n]) * (k / 6.0) }
    };
    let segments = if closed { n } else { n - 1 };
    for i in 0..segments {
        let j = (i + 1) % n;
        let (p0, p1) = (pts[i], pts[j]);
        if corners[i] && corners[j] {
            pb.line_to(p1);
        } else {
            pb.cubic_to(p0 + tangent(i), p1 - tangent(j), p1);
        }
    }
    if closed {
        pb.close();
    }
    pb.finish()
}

/// Verschachtelungstiefe je Linienzug (gerade = Außenkontur eines Teils, ungerade = Loch); offene zählen nicht
/// als Hülle und haben Tiefe 0.
pub fn nesting_depths(polys: &[Polyline]) -> Vec<usize> {
    let idx: Vec<usize> = (0..polys.len()).filter(|&i| polys[i].closed && polys[i].points.len() >= 3).collect();
    let rings: Vec<&Polyline> = idx.iter().map(|&i| &polys[i]).collect();
    let (depth, _) = nesting(&rings);
    let mut out = vec![0; polys.len()];
    for (k, &i) in idx.iter().enumerate() {
        out[i] = depth[k];
    }
    out
}

/// Äußerste geschlossene Konturen (in keiner anderen enthalten).
pub fn outermost(polys: &[Polyline]) -> Vec<Polyline> {
    let rings: Vec<&Polyline> = polys.iter().filter(|p| p.closed && p.points.len() >= 3).collect();
    let (depth, _) = nesting(&rings);
    rings.iter().zip(depth).filter(|(_, d)| *d == 0).map(|(r, _)| (*r).clone()).collect()
}

/// Verschachtelungstiefe jedes Rings und der kleinste ihn enthaltende Ring.
fn nesting(rings: &[&Polyline]) -> (Vec<usize>, Vec<Option<usize>>) {
    let bounds: Vec<crate::geom::Rect> = rings.iter().map(|p| p.bounds()).collect();
    let area: Vec<f64> = rings.iter().map(|p| p.signed_area().abs()).collect();
    let n = rings.len();
    let mut depth = vec![0usize; n];
    let mut parent: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        let probe = rings[i].points[0];
        for j in 0..n {
            if i == j
                || area[j] < area[i]
                || !bounds[j].contains_rect(&bounds[i])
                || !crate::path::point_in_polylines(probe, std::slice::from_ref(rings[j]))
            {
                continue;
            }
            depth[i] += 1;
            if parent[i].is_none_or(|p| area[j] < area[p]) {
                parent[i] = Some(j);
            }
        }
    }
    (depth, parent)
}

/// Konvexe Hülle (Andrew-Verfahren), gegen den Uhrzeigersinn im mathematischen Sinn, ohne doppelte und
/// kollineare Punkte. Bei weniger als drei verschiedenen Punkten kommen diese unverändert zurück.
pub fn convex_hull(points: &[Point]) -> Vec<Point> {
    let mut pts: Vec<Point> = points.to_vec();
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup_by(|a, b| (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9);
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: Point, a: Point, b: Point| (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
    // untere Kette von links nach rechts, obere zurück; der jeweils letzte Punkt beginnt die andere Kette
    let half = |it: &mut dyn Iterator<Item = Point>| {
        let mut h: Vec<Point> = Vec::new();
        for p in it {
            while h.len() >= 2 && cross(h[h.len() - 2], h[h.len() - 1], p) <= 1e-12 {
                h.pop();
            }
            h.push(p);
        }
        h.pop();
        h
    };
    let mut hull = half(&mut pts.iter().copied());
    hull.extend(half(&mut pts.iter().rev().copied()));
    hull
}

/// Konzentrische Linien nach innen im Abstand `interval` (Modus „Versatz-Füllen“), von außen nach innen.
/// Die erste Linie liegt einen halben Abstand innerhalb der Kontur.
pub fn offset_fill(polys: &[Polyline], interval: f64, max_rings: usize) -> Vec<Polyline> {
    let interval = interval.max(0.01);
    let shapes = normalize(polys);
    if shapes.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for k in 0..max_rings {
        let d = -(k as f64 + 0.5) * interval;
        let style = OutlineStyle::new(d).line_join(Corner::Round.join(d));
        let ring = to_polylines(shapes.outline(&style));
        if ring.is_empty() {
            break;
        }
        out.extend(ring);
    }
    out
}

/// Laschen: unterbricht eine geschlossene Kontur an `count` gleichmäßig verteilten Stellen um je `width` mm.
/// Die Kontur beginnt am bisherigen Startpunkt; Laschen liegen jeweils mittig zwischen zwei Teilungspunkten.
pub fn split_for_tabs(poly: &Polyline, count: usize, width: f64) -> Vec<Polyline> {
    let length = poly.length();
    if !poly.closed || count == 0 || width <= 0.0 || length <= count as f64 * width * 2.0 {
        return vec![poly.clone()];
    }
    let step = length / count as f64;
    // Laschenbereiche [a, b) entlang der Kontur
    let gaps: Vec<(f64, f64)> = (0..count)
        .map(|i| {
            let mid = (i as f64 + 0.5) * step;
            (mid - width / 2.0, mid + width / 2.0)
        })
        .collect();
    let mut pts: Vec<Point> = poly.points.clone();
    pts.push(poly.points[0]);
    let mut pieces = Vec::new();
    let mut cur: Vec<Point> = vec![pts[0]];
    let mut pos = 0.0;
    let mut gap_idx = 0;
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let seg = a.distance(b);
        let end = pos + seg;
        // Alle Laschengrenzen auf diesem Segment abarbeiten
        while let Some(&(g0, g1)) = gaps.get(gap_idx) {
            if !cur.is_empty() && g0 < end {
                let t = ((g0 - pos) / seg).clamp(0.0, 1.0);
                cur.push(a.lerp(b, t));
                pieces.push(Polyline::new(std::mem::take(&mut cur), false));
            }
            if cur.is_empty() && g1 <= end {
                let t = ((g1 - pos) / seg).clamp(0.0, 1.0);
                cur.push(a.lerp(b, t));
                gap_idx += 1;
                continue;
            }
            break;
        }
        if !cur.is_empty() {
            cur.push(b);
        }
        pos = end;
    }
    if cur.len() >= 2 {
        pieces.push(Polyline::new(cur, false));
    }
    // Das Stück vor der ersten Lasche schließt am Startpunkt an das letzte Stück an.
    if pieces.len() >= 2 {
        let first = pieces.remove(0);
        pieces.last_mut().unwrap().points.extend_from_slice(&first.points[1..]);
    }
    pieces.retain(|p| p.points.len() >= 2 && p.length() > 1e-6);
    pieces
}

/// Verteilt `count` Punkte gleichmäßig entlang einer Polylinie (geschlossen: rundherum, offen: von Anfang bis
/// Ende) und liefert je Punkt die Richtung der Linie dort (Bogenmaß).
pub fn points_along(poly: &Polyline, count: usize) -> Vec<(Point, f64)> {
    let mut pts = poly.points.clone();
    if poly.closed && pts.len() > 2 {
        pts.push(pts[0]);
    }
    let edges: Vec<(Point, Point, f64)> =
        pts.windows(2).map(|w| (w[0], w[1], w[0].distance(w[1]))).filter(|e| e.2 > 1e-12).collect();
    let total: f64 = edges.iter().map(|e| e.2).sum();
    if count == 0 || edges.is_empty() {
        return Vec::new();
    }
    let step = if poly.closed {
        total / count as f64
    } else if count > 1 {
        total / (count - 1) as f64
    } else {
        0.0
    };
    let mut out = Vec::with_capacity(count);
    let (mut edge, mut walked) = (0, 0.0);
    for i in 0..count {
        let s = (step * i as f64).min(total);
        while edge + 1 < edges.len() && walked + edges[edge].2 < s - 1e-9 {
            walked += edges[edge].2;
            edge += 1;
        }
        let (a, b, len) = edges[edge];
        let t = ((s - walked) / len).clamp(0.0, 1.0);
        let d = b - a;
        out.push((a.lerp(b, t), d.y.atan2(d.x)));
    }
    out
}

/// Länge einer Polylinie (geschlossen: inklusive Schlusskante).
pub fn polyline_length(poly: &Polyline) -> f64 {
    let n = poly.points.len();
    let open: f64 = poly.points.windows(2).map(|w| w[0].distance(w[1])).sum();
    if poly.closed && n > 2 { open + poly.points[n - 1].distance(poly.points[0]) } else { open }
}

/// Punkt und Richtung (Bogenmaß) bei Bogenlänge `s`. Geschlossene Linien laufen weiter herum, offene werden über
/// die Enden hinaus geradlinig verlängert.
pub fn position_at(poly: &Polyline, s: f64) -> Option<(Point, f64)> {
    let mut pts = poly.points.clone();
    if poly.closed && pts.len() > 2 {
        pts.push(pts[0]);
    }
    let edges: Vec<(Point, Point, f64)> =
        pts.windows(2).map(|w| (w[0], w[1], w[0].distance(w[1]))).filter(|e| e.2 > 1e-12).collect();
    let total: f64 = edges.iter().map(|e| e.2).sum();
    if edges.is_empty() {
        return None;
    }
    let s = if poly.closed { s.rem_euclid(total) } else { s };
    let dir = |a: Point, b: Point| (b.y - a.y).atan2(b.x - a.x);
    if s <= 0.0 {
        let (a, b, len) = edges[0];
        return Some((a + (b - a) * (s / len), dir(a, b)));
    }
    let mut walked = 0.0;
    for &(a, b, len) in &edges {
        if s <= walked + len {
            return Some((a.lerp(b, (s - walked) / len), dir(a, b)));
        }
        walked += len;
    }
    let (a, b, len) = *edges.last().unwrap();
    Some((b + (b - a) * ((s - total) / len), dir(a, b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplify_keeps_shape_within_tolerance() {
        // Kreis mit 400 Punkten, offene Zickzacklinie
        let circle: Vec<Point> =
            (0..400).map(|i| Point::new(10.0, 0.0).rotated(i as f64 / 400.0 * std::f64::consts::TAU)).collect();
        let s = simplify_points(&circle, true, 0.05);
        assert!(s.len() < 60 && s.len() > 8, "{}", s.len());
        for p in &circle {
            let poly = Polyline::new(s.clone(), true);
            assert!(poly.distance_to(*p) <= 0.05 + 1e-9);
        }
        let line: Vec<Point> = (0..=10).map(|i| Point::new(i as f64, if i % 2 == 0 { 0.0 } else { 0.01 })).collect();
        assert_eq!(simplify_points(&line, false, 0.05), [line[0], line[10]]);
        assert_eq!(simplify_points(&line, false, 0.001).len(), 11);
        // Glätten: offene Enden bleiben, geschlossene Form wird zu Kurven
        let smooth = smooth_points(&s, true, 1.0, 65.0);
        assert!(smooth.subpaths[0].closed);
        let open = smooth_points(&line, false, 1.0, 65.0);
        assert_eq!(open.subpaths[0].start, line[0]);
        assert_eq!(open.subpaths[0].end(), line[10]);
    }

    #[test]
    fn islands_pair_outlines_with_holes() {
        use crate::geom::Affine;
        use crate::path::VectorPath;
        // Rahmen mit Loch, darin eine Insel; daneben ein einzelnes Quadrat
        let mut p = VectorPath::rect(0.0, 0.0, 30.0, 30.0, 0.0);
        p.append(VectorPath::rect(5.0, 5.0, 20.0, 20.0, 0.0));
        p.append(VectorPath::rect(10.0, 10.0, 10.0, 10.0, 0.0));
        p.append(VectorPath::rect(50.0, 0.0, 5.0, 5.0, 0.0));
        p.append(VectorPath::line(Point::new(0.0, 40.0), Point::new(10.0, 40.0)));
        let polys = p.flatten(&Affine::IDENTITY, 0.01);
        let mut sizes: Vec<usize> = islands(&polys).iter().map(|i| i.len()).collect();
        sizes.sort();
        assert_eq!(sizes, [1, 1, 2]);
        let frame = islands(&polys).into_iter().find(|i| i.len() == 2).unwrap();
        assert!((frame[0].bounds().width() - 30.0).abs() < 1e-9 && (frame[1].bounds().width() - 20.0).abs() < 1e-9);
    }
    use crate::path::point_in_polylines;

    #[test]
    fn convex_hull_of_points() {
        let p = |x: f64, y: f64| Point::new(x, y);
        let pts =
            [p(2.0, 2.0), p(0.0, 0.0), p(4.0, 0.0), p(2.0, 0.0), p(4.0, 4.0), p(1.0, 3.0), p(0.0, 4.0), p(4.0, 4.0)];
        assert_eq!(convex_hull(&pts), [p(0.0, 0.0), p(4.0, 0.0), p(4.0, 4.0), p(0.0, 4.0)]);
        assert_eq!(convex_hull(&[p(1.0, 1.0), p(1.0, 1.0), p(2.0, 1.0)]).len(), 2);
        assert!(convex_hull(&[]).is_empty());
    }

    #[test]
    fn position_at_wraps_and_extends() {
        let square = Polyline::new(
            vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)],
            true,
        );
        assert_eq!(polyline_length(&square), 40.0);
        let (p, a) = position_at(&square, 45.0).unwrap();
        assert!(p.distance(Point::new(5.0, 0.0)) < 1e-9 && a.abs() < 1e-9);
        let line = Polyline::new(vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0)], false);
        let (p, _) = position_at(&line, 12.0).unwrap();
        assert!(p.distance(Point::new(12.0, 0.0)) < 1e-9);
    }

    #[test]
    fn points_along_open_and_closed() {
        let line = Polyline::new(vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0)], false);
        let pts = points_along(&line, 3);
        assert_eq!(pts.len(), 3);
        assert!(pts[0].0.distance(Point::new(0.0, 0.0)) < 1e-9 && pts[0].1.abs() < 1e-9);
        assert!(pts[1].0.distance(Point::new(10.0, 0.0)) < 1e-9);
        assert!(pts[2].0.distance(Point::new(10.0, 10.0)) < 1e-9);
        assert!((pts[2].1 - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        let square = Polyline::new(
            vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)],
            true,
        );
        let pts = points_along(&square, 8);
        assert_eq!(pts.len(), 8);
        assert!(pts[7].0.distance(Point::new(0.0, 5.0)) < 1e-9, "{:?}", pts[7]);
    }

    fn square(x: f64, y: f64, s: f64) -> Polyline {
        Polyline::new(
            vec![Point::new(x, y), Point::new(x + s, y), Point::new(x + s, y + s), Point::new(x, y + s)],
            true,
        )
    }

    fn area(polys: &[Polyline]) -> f64 {
        // Löcher sind entgegengesetzt orientiert, die Summe ist die Nettofläche.
        polys.iter().map(|p| p.signed_area()).sum::<f64>().abs()
    }

    #[test]
    fn boolean_operations_on_overlapping_squares() {
        let a = [square(0.0, 0.0, 10.0)];
        let b = [square(5.0, 5.0, 10.0)];
        assert!((area(&boolean(&a, &b, BoolOp::Union)) - 175.0).abs() < 1e-6);
        assert!((area(&boolean(&a, &b, BoolOp::Intersection)) - 25.0).abs() < 1e-6);
        assert!((area(&boolean(&a, &b, BoolOp::Difference)) - 75.0).abs() < 1e-6);
        assert!((area(&boolean(&a, &b, BoolOp::Xor)) - 150.0).abs() < 1e-6);
    }

    #[test]
    fn weld_keeps_holes_of_single_objects() {
        // Rahmen mit Loch (ein Objekt) und ein Quadrat, das nur den Rand überlappt.
        let frame = vec![square(0.0, 0.0, 20.0), square(5.0, 5.0, 10.0)];
        let other = vec![square(18.0, 8.0, 6.0)];
        let w = weld(&[frame, other]);
        assert!(!point_in_polylines(Point::new(10.0, 10.0), &w), "Loch muss bleiben");
        assert!(point_in_polylines(Point::new(22.0, 10.0), &w));
        assert!((area(&w) - (400.0 - 100.0 + 6.0 * 6.0 - 2.0 * 6.0)).abs() < 1e-6);
    }

    #[test]
    fn offset_grows_and_shrinks() {
        let sq = [square(0.0, 0.0, 10.0)];
        let grown = offset(&sq, 1.0, Corner::Sharp);
        assert!((area(&grown) - 144.0).abs() < 1e-3, "{}", area(&grown));
        let shrunk = offset(&sq, -1.0, Corner::Sharp);
        assert!((area(&shrunk) - 64.0).abs() < 1e-3);
        let round = offset(&sq, 1.0, Corner::Round);
        let expected = 100.0 + 4.0 * 10.0 + std::f64::consts::PI;
        assert!((area(&round) - expected).abs() < 0.05, "{}", area(&round));
        // Offene Linie wird zu einer geschlossenen Umrandung
        let line = [Polyline::new(vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0)], false)];
        let around = offset(&line, 1.0, Corner::Bevel);
        assert_eq!(around.len(), 1);
        assert!((area(&around) - 20.0).abs() < 1e-3);
    }

    #[test]
    fn offset_fill_rings() {
        let rings = offset_fill(&[square(0.0, 0.0, 10.0)], 1.0, 1000);
        assert_eq!(rings.len(), 5);
        let b = rings[0].bounds();
        assert!((b.min.x - 0.5).abs() < 1e-3 && (b.max.x - 9.5).abs() < 1e-3, "{b:?}");
    }

    #[test]
    fn tabs_leave_gaps() {
        let sq = square(0.0, 0.0, 10.0);
        let pieces = split_for_tabs(&sq, 4, 1.0);
        assert_eq!(pieces.len(), 4, "{pieces:?}");
        let cut: f64 = pieces.iter().map(|p| p.length()).sum();
        assert!((cut - 36.0).abs() < 1e-9, "{cut}");
        // Zu kurze Kontur bleibt unverändert
        assert_eq!(split_for_tabs(&square(0.0, 0.0, 1.0), 4, 1.0).len(), 1);
    }
}
