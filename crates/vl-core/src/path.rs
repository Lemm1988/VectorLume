//! Vektorpfade aus Linien und Bézierkurven sowie deren Umwandlung in Polylinien.

use crate::geom::{Affine, Point, Rect};
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

/// Kappa für die Annäherung eines Viertelkreises durch eine kubische Bézierkurve.
const KAPPA: f64 = 0.552_284_749_830_793_4;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Segment {
    Line(Point),
    Quad(Point, Point),
    Cubic(Point, Point, Point),
}

impl Segment {
    pub fn end(&self) -> Point {
        match *self {
            Segment::Line(p) | Segment::Quad(_, p) | Segment::Cubic(_, _, p) => p,
        }
    }

    fn transformed(&self, t: &Affine) -> Segment {
        match *self {
            Segment::Line(p) => Segment::Line(t.apply(p)),
            Segment::Quad(a, p) => Segment::Quad(t.apply(a), t.apply(p)),
            Segment::Cubic(a, b, p) => Segment::Cubic(t.apply(a), t.apply(b), t.apply(p)),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SubPath {
    pub start: Point,
    pub segments: Vec<Segment>,
    pub closed: bool,
}

impl SubPath {
    pub fn end(&self) -> Point {
        self.segments.last().map(|s| s.end()).unwrap_or(self.start)
    }
}

/// Ein Vektorpfad aus beliebig vielen Teilpfaden (z. B. Buchstabe „O“ = zwei Teilpfade).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VectorPath {
    pub subpaths: Vec<SubPath>,
}

/// Abgeflachter Teilpfad. Bei geschlossenen Polylinien wird der Startpunkt nicht wiederholt.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Polyline {
    pub points: Vec<Point>,
    pub closed: bool,
}

impl Polyline {
    pub fn new(points: Vec<Point>, closed: bool) -> Self {
        Self { points, closed }
    }

    pub fn bounds(&self) -> Rect {
        let mut r = Rect::EMPTY;
        for &p in &self.points {
            r.include(p);
        }
        r
    }

    /// Kanten als Punktpaare; bei geschlossenen Polylinien inklusive Schlusskante.
    pub fn edges(&self) -> impl Iterator<Item = (Point, Point)> + '_ {
        let n = self.points.len();
        let count = if self.closed && n > 2 { n } else { n.saturating_sub(1) };
        (0..count).map(move |i| (self.points[i], self.points[(i + 1) % n]))
    }

    pub fn length(&self) -> f64 {
        self.edges().map(|(a, b)| a.distance(b)).sum()
    }

    /// Vorzeichenbehaftete Fläche (Schuhbandformel).
    pub fn signed_area(&self) -> f64 {
        if self.points.len() < 3 {
            return 0.0;
        }
        let n = self.points.len();
        (0..n).map(|i| self.points[i].cross(self.points[(i + 1) % n])).sum::<f64>() * 0.5
    }

    pub fn transform(&mut self, t: &Affine) {
        for p in &mut self.points {
            *p = t.apply(*p);
        }
    }

    /// Kleinster Abstand eines Punktes zur Polylinie.
    pub fn distance_to(&self, p: Point) -> f64 {
        if self.points.len() == 1 {
            return self.points[0].distance(p);
        }
        self.edges().map(|(a, b)| distance_to_segment(p, a, b)).fold(f64::INFINITY, f64::min)
    }
}

pub fn distance_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len_sq = ab.dot(ab);
    if len_sq <= 1e-18 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Gerade-Ungerade-Regel über alle geschlossenen Polylinien.
pub fn point_in_polylines(p: Point, polys: &[Polyline]) -> bool {
    let mut inside = false;
    for poly in polys.iter().filter(|pl| pl.closed && pl.points.len() > 2) {
        for (a, b) in poly.edges() {
            if (a.y > p.y) != (b.y > p.y) {
                let x = a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y);
                if p.x < x {
                    inside = !inside;
                }
            }
        }
    }
    inside
}

pub fn bounds_of(polys: &[Polyline]) -> Rect {
    polys.iter().fold(Rect::EMPTY, |r, p| r.union(&p.bounds()))
}

impl VectorPath {
    pub fn is_empty(&self) -> bool {
        self.subpaths.iter().all(|s| s.segments.is_empty())
    }

    pub fn segment_count(&self) -> usize {
        self.subpaths.iter().map(|s| s.segments.len()).sum()
    }

    pub fn line(a: Point, b: Point) -> Self {
        let mut pb = PathBuilder::new();
        pb.move_to(a);
        pb.line_to(b);
        pb.finish()
    }

    pub fn polygon(points: &[Point], closed: bool) -> Self {
        let mut pb = PathBuilder::new();
        if let Some((&first, rest)) = points.split_first() {
            pb.move_to(first);
            for &p in rest {
                pb.line_to(p);
            }
            if closed {
                pb.close();
            }
        }
        pb.finish()
    }

    /// Rechteck mit optionalem Eckenradius.
    pub fn rect(x: f64, y: f64, w: f64, h: f64, radius: f64) -> Self {
        let r = radius.max(0.0).min(w.abs() / 2.0).min(h.abs() / 2.0);
        let mut pb = PathBuilder::new();
        if r <= 1e-9 {
            pb.move_to(Point::new(x, y));
            pb.line_to(Point::new(x + w, y));
            pb.line_to(Point::new(x + w, y + h));
            pb.line_to(Point::new(x, y + h));
            pb.close();
            return pb.finish();
        }
        let k = r * KAPPA;
        let (x0, y0, x1, y1) = (x, y, x + w, y + h);
        pb.move_to(Point::new(x0 + r, y0));
        pb.line_to(Point::new(x1 - r, y0));
        pb.cubic_to(Point::new(x1 - r + k, y0), Point::new(x1, y0 + r - k), Point::new(x1, y0 + r));
        pb.line_to(Point::new(x1, y1 - r));
        pb.cubic_to(Point::new(x1, y1 - r + k), Point::new(x1 - r + k, y1), Point::new(x1 - r, y1));
        pb.line_to(Point::new(x0 + r, y1));
        pb.cubic_to(Point::new(x0 + r - k, y1), Point::new(x0, y1 - r + k), Point::new(x0, y1 - r));
        pb.line_to(Point::new(x0, y0 + r));
        pb.cubic_to(Point::new(x0, y0 + r - k), Point::new(x0 + r - k, y0), Point::new(x0 + r, y0));
        pb.close();
        pb.finish()
    }

    pub fn ellipse(center: Point, rx: f64, ry: f64) -> Self {
        let mut pb = PathBuilder::new();
        pb.ellipse(center, rx, ry);
        pb.finish()
    }

    /// Regelmäßiges Vieleck, erste Ecke oben.
    pub fn regular_polygon(center: Point, radius: f64, sides: usize) -> Self {
        let n = sides.max(3);
        let pts: Vec<Point> = (0..n)
            .map(|i| {
                let a = -PI / 2.0 + TAU * i as f64 / n as f64;
                center + Point::new(a.cos(), a.sin()) * radius
            })
            .collect();
        Self::polygon(&pts, true)
    }

    pub fn star(center: Point, outer: f64, inner: f64, spikes: usize) -> Self {
        let n = spikes.max(3) * 2;
        let pts: Vec<Point> = (0..n)
            .map(|i| {
                let a = -PI / 2.0 + TAU * i as f64 / n as f64;
                let r = if i % 2 == 0 { outer } else { inner };
                center + Point::new(a.cos(), a.sin()) * r
            })
            .collect();
        Self::polygon(&pts, true)
    }

    pub fn transformed(&self, t: &Affine) -> VectorPath {
        VectorPath {
            subpaths: self
                .subpaths
                .iter()
                .map(|s| SubPath {
                    start: t.apply(s.start),
                    segments: s.segments.iter().map(|seg| seg.transformed(t)).collect(),
                    closed: s.closed,
                })
                .collect(),
        }
    }

    pub fn append(&mut self, other: VectorPath) {
        self.subpaths.extend(other.subpaths);
    }

    /// Wandelt den Pfad nach Anwendung von `t` in Polylinien mit maximaler Abweichung `tol` (mm) um.
    pub fn flatten(&self, t: &Affine, tol: f64) -> Vec<Polyline> {
        let tol = tol.max(1e-4);
        let mut out = Vec::with_capacity(self.subpaths.len());
        for sp in &self.subpaths {
            if sp.segments.is_empty() {
                continue;
            }
            let mut pts = Vec::with_capacity(sp.segments.len() + 1);
            let mut cur = t.apply(sp.start);
            pts.push(cur);
            for seg in &sp.segments {
                match seg.transformed(t) {
                    Segment::Line(p) => pts.push(p),
                    Segment::Quad(c, p) => flatten_quad(cur, c, p, tol, &mut pts),
                    Segment::Cubic(c1, c2, p) => flatten_cubic(cur, c1, c2, p, tol, &mut pts),
                }
                cur = *pts.last().unwrap();
            }
            dedup_points(&mut pts);
            let mut closed = sp.closed;
            if pts.len() > 2 && pts[0].distance(*pts.last().unwrap()) < 1e-6 {
                pts.pop();
                closed = true;
            }
            if closed && pts.len() < 3 {
                closed = false;
            }
            if pts.len() >= 2 || (pts.len() == 1 && !closed) {
                out.push(Polyline::new(pts, closed));
            }
        }
        out
    }

    pub fn bounds(&self, t: &Affine) -> Rect {
        bounds_of(&self.flatten(t, 0.01))
    }
}

fn dedup_points(pts: &mut Vec<Point>) {
    pts.dedup_by(|b, a| a.distance_sq(*b) < 1e-14);
}

fn flatten_quad(p0: Point, p1: Point, p2: Point, tol: f64, out: &mut Vec<Point>) {
    let dd = (p0 - p1 * 2.0 + p2).length();
    let n = ((0.25 * dd / tol).sqrt().ceil() as usize).clamp(1, 1000);
    for i in 1..=n {
        let t = i as f64 / n as f64;
        let mt = 1.0 - t;
        out.push(p0 * (mt * mt) + p1 * (2.0 * mt * t) + p2 * (t * t));
    }
}

fn flatten_cubic(p0: Point, p1: Point, p2: Point, p3: Point, tol: f64, out: &mut Vec<Point>) {
    // Wang-Formel: Anzahl Teilstücke, damit die Sehnenabweichung unter `tol` bleibt.
    let dd = (p0 - p1 * 2.0 + p2).length().max((p1 - p2 * 2.0 + p3).length());
    let n = ((0.75 * dd / tol).sqrt().ceil() as usize).clamp(1, 2000);
    for i in 1..=n {
        let t = i as f64 / n as f64;
        let mt = 1.0 - t;
        out.push(p0 * (mt * mt * mt) + p1 * (3.0 * mt * mt * t) + p2 * (3.0 * mt * t * t) + p3 * (t * t * t));
    }
}

/// Hilfsklasse zum Aufbau von Pfaden (für Importer, Schrift und Formen).
#[derive(Default)]
pub struct PathBuilder {
    path: VectorPath,
    current: Option<SubPath>,
}

impl PathBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn move_to(&mut self, p: Point) {
        self.flush();
        self.current = Some(SubPath { start: p, segments: Vec::new(), closed: false });
    }

    fn ensure(&mut self, p: Point) -> &mut SubPath {
        if self.current.is_none() {
            self.current = Some(SubPath { start: p, segments: Vec::new(), closed: false });
        }
        self.current.as_mut().unwrap()
    }

    pub fn current_point(&self) -> Option<Point> {
        self.current.as_ref().map(|s| s.end())
    }

    pub fn line_to(&mut self, p: Point) {
        self.ensure(p).segments.push(Segment::Line(p));
    }

    pub fn quad_to(&mut self, c: Point, p: Point) {
        self.ensure(c).segments.push(Segment::Quad(c, p));
    }

    pub fn cubic_to(&mut self, c1: Point, c2: Point, p: Point) {
        self.ensure(c1).segments.push(Segment::Cubic(c1, c2, p));
    }

    pub fn close(&mut self) {
        if let Some(mut sp) = self.current.take() {
            if let Some(Segment::Line(p)) = sp.segments.last() {
                if p.distance(sp.start) < 1e-9 {
                    sp.segments.pop();
                }
            }
            sp.closed = true;
            if !sp.segments.is_empty() {
                self.path.subpaths.push(sp);
            }
        }
    }

    /// Ellipsenbogen um `center` mit Halbachsen `rx`/`ry` und Achsdrehung `rotation`, von `start`
    /// über `sweep` (Bogenmaß, Vorzeichen = Richtung). Beginnt einen neuen Teilpfad, falls nötig.
    pub fn arc(&mut self, center: Point, rx: f64, ry: f64, rotation: f64, start: f64, sweep: f64) {
        let rot = Affine::rotate(rotation);
        let pt = |a: f64| center + rot.apply_vector(Point::new(rx * a.cos(), ry * a.sin()));
        let p0 = pt(start);
        match self.current_point() {
            Some(cp) if cp.distance(p0) < 1e-9 => {}
            Some(_) => self.line_to(p0),
            None => self.move_to(p0),
        }
        let parts = ((sweep.abs() / (PI / 2.0)).ceil() as usize).max(1);
        let step = sweep / parts as f64;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        for i in 0..parts {
            let a0 = start + step * i as f64;
            let a1 = a0 + step;
            let d0 = rot.apply_vector(Point::new(-rx * a0.sin(), ry * a0.cos()));
            let d1 = rot.apply_vector(Point::new(-rx * a1.sin(), ry * a1.cos()));
            let (q0, q3) = (pt(a0), pt(a1));
            self.cubic_to(q0 + d0 * k, q3 - d1 * k, q3);
        }
    }

    pub fn ellipse(&mut self, center: Point, rx: f64, ry: f64) {
        self.move_to(Point::new(center.x + rx, center.y));
        self.arc(center, rx, ry, 0.0, 0.0, TAU);
        self.close();
    }

    fn flush(&mut self) {
        if let Some(sp) = self.current.take() {
            if !sp.segments.is_empty() {
                self.path.subpaths.push(sp);
            }
        }
    }

    pub fn finish(mut self) -> VectorPath {
        self.flush();
        self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_flattens_to_four_points() {
        let p = VectorPath::rect(0.0, 0.0, 10.0, 5.0, 0.0);
        let polys = p.flatten(&Affine::IDENTITY, 0.01);
        assert_eq!(polys.len(), 1);
        assert!(polys[0].closed);
        assert_eq!(polys[0].points.len(), 4);
        assert!((polys[0].signed_area().abs() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn circle_is_accurate() {
        let r = 25.0;
        let p = VectorPath::ellipse(Point::new(50.0, 50.0), r, r);
        let polys = p.flatten(&Affine::IDENTITY, 0.005);
        let max_err =
            polys[0].points.iter().map(|q| (q.distance(Point::new(50.0, 50.0)) - r).abs()).fold(0.0, f64::max);
        assert!(max_err < 0.02, "Abweichung {max_err}");
        let area = polys[0].signed_area().abs();
        assert!((area - PI * r * r).abs() / (PI * r * r) < 0.002);
    }

    #[test]
    fn rounded_rect_bounds() {
        let p = VectorPath::rect(1.0, 2.0, 30.0, 20.0, 5.0);
        let b = p.bounds(&Affine::IDENTITY);
        assert!((b.min.x - 1.0).abs() < 1e-6 && (b.max.y - 22.0).abs() < 1e-6);
    }

    #[test]
    fn point_in_ring_uses_even_odd() {
        let mut ring = VectorPath::rect(0.0, 0.0, 10.0, 10.0, 0.0);
        ring.append(VectorPath::rect(3.0, 3.0, 4.0, 4.0, 0.0));
        let polys = ring.flatten(&Affine::IDENTITY, 0.01);
        assert!(point_in_polylines(Point::new(1.0, 1.0), &polys));
        assert!(!point_in_polylines(Point::new(5.0, 5.0), &polys));
        assert!(!point_in_polylines(Point::new(11.0, 5.0), &polys));
    }

    #[test]
    fn arc_quarter_ends_correctly() {
        let mut pb = PathBuilder::new();
        pb.arc(Point::ZERO, 10.0, 10.0, 0.0, 0.0, PI / 2.0);
        let path = pb.finish();
        let end = path.subpaths[0].end();
        assert!(end.distance(Point::new(0.0, 10.0)) < 1e-9);
    }
}
