//! Verzerren: Formen perspektivisch in ein Viereck oder in eine Hülle mit gebogenen Kanten einpassen.
//!
//! Ausgangspunkt ist das Rechteck `src` (Umriss der Auswahl). Die **Perspektive** (vier Ecken, projektive Abbildung)
//! hält Geraden gerade. Die **Hülle** (Ecken und Kantenmitten, quadratisches Serendipity-Element wie in der
//! Finite-Elemente-Methode) biegt die Kanten zu Parabeln. Kurven – bei der Hülle auch Linien – werden in kubische
//! Stücke zerlegt, bis sie höchstens `tol` vom exakten Bild abweichen.

use crate::geom::{Point, Rect};
use crate::path::{Segment, SubPath, VectorPath};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarpKind {
    /// Vier Ecken, Geraden bleiben gerade.
    Perspective,
    /// Ecken und Kantenmitten, Kanten werden gebogen.
    Envelope,
}

impl WarpKind {
    pub const ALL: [WarpKind; 2] = [WarpKind::Perspective, WarpKind::Envelope];

    pub fn label(self) -> &'static str {
        match self {
            WarpKind::Perspective => "Perspektive",
            WarpKind::Envelope => "Hülle",
        }
    }
}

/// Lage der Griffe im Einheitsquadrat (u nach rechts, v nach unten): Ecken oben links, oben rechts, unten rechts,
/// unten links, dann Kantenmitten oben, rechts, unten, links.
const REST_UV: [(f64, f64); 8] =
    [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)];

/// Maximale Teilungstiefe je Segment (2^12 Stücke).
const MAX_DEPTH: u32 = 12;

#[derive(Clone, Debug, PartialEq)]
pub struct Warp {
    pub kind: WarpKind,
    pub src: Rect,
    /// Zielpositionen der Griffe in der Reihenfolge von `REST_UV`; die Kantenmitten gelten nur für die Hülle.
    pub handles: [Point; 8],
}

impl Warp {
    pub fn new(src: Rect, kind: WarpKind) -> Self {
        Self { kind, src, handles: Self::rest(&src) }
    }

    /// Griffe in Ruhelage (keine Verzerrung).
    pub fn rest(src: &Rect) -> [Point; 8] {
        REST_UV.map(|(u, v)| Point::new(src.min.x + u * src.width(), src.min.y + v * src.height()))
    }

    /// Anzahl der wirksamen Griffe.
    pub fn handle_count(&self) -> usize {
        match self.kind {
            WarpKind::Perspective => 4,
            WarpKind::Envelope => 8,
        }
    }

    pub fn reset(&mut self) {
        self.handles = Self::rest(&self.src);
    }

    /// Wechselt die Art; die aktuelle Form bleibt so gut wie möglich erhalten (Kantenmitten aus der Perspektive).
    pub fn set_kind(&mut self, kind: WarpKind) {
        if kind == WarpKind::Envelope && self.kind == WarpKind::Perspective {
            let rest = Self::rest(&self.src);
            let mids = [4, 5, 6, 7].map(|i| self.map(rest[i]));
            self.handles[4..].copy_from_slice(&mids);
        }
        self.kind = kind;
    }

    pub fn is_identity(&self) -> bool {
        let rest = Self::rest(&self.src);
        let eps = 1e-9 * (1.0 + self.src.width() + self.src.height());
        (0..self.handle_count()).all(|i| self.handles[i].distance(rest[i]) <= eps)
    }

    fn uv(&self, p: Point) -> (f64, f64) {
        let (w, h) = (self.src.width().max(1e-12), self.src.height().max(1e-12));
        ((p.x - self.src.min.x) / w, (p.y - self.src.min.y) / h)
    }

    /// Koeffizienten der projektiven Abbildung Einheitsquadrat → Viereck (Heckbert), `None` bei entartetem Viereck.
    fn homography(&self) -> Option<[f64; 8]> {
        let [p0, p1, p2, p3, ..] = self.handles;
        let (sx, sy) = (p0.x - p1.x + p2.x - p3.x, p0.y - p1.y + p2.y - p3.y);
        let (dx1, dx2, dy1, dy2) = (p1.x - p2.x, p3.x - p2.x, p1.y - p2.y, p3.y - p2.y);
        let den = dx1 * dy2 - dx2 * dy1;
        if den.abs() < 1e-12 {
            return None;
        }
        let g = (sx * dy2 - dx2 * sy) / den;
        let h = (dx1 * sy - sx * dy1) / den;
        Some([
            p1.x - p0.x + g * p1.x,
            p3.x - p0.x + h * p3.x,
            p0.x,
            p1.y - p0.y + g * p1.y,
            p3.y - p0.y + h * p3.y,
            p0.y,
            g,
            h,
        ])
    }

    fn map_uv(&self, u: f64, v: f64, hom: Option<&[f64; 8]>) -> Point {
        match self.kind {
            WarpKind::Perspective => match hom {
                Some([a, b, c, d, e, f, g, h]) => {
                    let w = g * u + h * v + 1.0;
                    Point::new((a * u + b * v + c) / w, (d * u + e * v + f) / w)
                }
                // entartet: bilinear zwischen den Ecken (wird ohnehin als ungültig gemeldet)
                None => {
                    let [p0, p1, p2, p3, ..] = self.handles;
                    p0.lerp(p1, u).lerp(p3.lerp(p2, u), v)
                }
            },
            WarpKind::Envelope => {
                let (x, y) = (2.0 * u - 1.0, 2.0 * v - 1.0);
                let corner = |cx: f64, cy: f64| 0.25 * (1.0 + x * cx) * (1.0 + y * cy) * (x * cx + y * cy - 1.0);
                let n = [
                    corner(-1.0, -1.0),
                    corner(1.0, -1.0),
                    corner(1.0, 1.0),
                    corner(-1.0, 1.0),
                    0.5 * (1.0 - x * x) * (1.0 - y),
                    0.5 * (1.0 + x) * (1.0 - y * y),
                    0.5 * (1.0 - x * x) * (1.0 + y),
                    0.5 * (1.0 - x) * (1.0 - y * y),
                ];
                n.iter().zip(&self.handles).fold(Point::ZERO, |acc, (w, p)| acc + *p * *w)
            }
        }
    }

    /// Bild eines Punkts.
    pub fn map(&self, p: Point) -> Point {
        let (u, v) = self.uv(p);
        self.map_uv(u, v, self.homography().as_ref())
    }

    /// Ist die Abbildung umkehrbar (nichts klappt um)? Perspektive: konvexes Viereck; Hülle: Jacobi-Determinante
    /// auf einem Prüfraster überall mit demselben Vorzeichen.
    pub fn is_valid(&self) -> bool {
        match self.kind {
            WarpKind::Perspective => {
                let c = &self.handles[..4];
                let crosses: Vec<f64> = (0..4)
                    .map(|i| {
                        let (a, b, d) = (c[i], c[(i + 1) % 4], c[(i + 2) % 4]);
                        (b - a).cross(d - b)
                    })
                    .collect();
                let scale = (self.src.width() * self.src.height()).max(1e-12) * 1e-6;
                self.homography().is_some()
                    && (crosses.iter().all(|&k| k > scale) || crosses.iter().all(|&k| k < -scale))
            }
            WarpKind::Envelope => {
                const N: usize = 12;
                let d = 1e-4;
                let mut sign = 0.0;
                for i in 0..=N {
                    for j in 0..=N {
                        let (u, v) = (i as f64 / N as f64, j as f64 / N as f64);
                        let du = self.map_uv(u + d, v, None) - self.map_uv(u - d, v, None);
                        let dv = self.map_uv(u, v + d, None) - self.map_uv(u, v - d, None);
                        let det = du.cross(dv);
                        if det.abs() < 1e-12 || (sign != 0.0 && det.signum() != sign) {
                            return false;
                        }
                        sign = det.signum();
                    }
                }
                true
            }
        }
    }

    /// Bild eines Punkts im Einheitsquadrat des Ausgangsrechtecks (u nach rechts, v nach unten).
    pub fn map_unit(&self, u: f64, v: f64) -> Point {
        self.map_uv(u, v, self.homography().as_ref())
    }

    /// Umriss des verzerrten Rechtecks (für die Anzeige), `n` Punkte je Kante.
    pub fn outline(&self, n: usize) -> Vec<Point> {
        let hom = self.homography();
        let n = n.max(1);
        let mut pts = Vec::with_capacity(4 * n);
        let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        for k in 0..4 {
            let (a, b) = (corners[k], corners[(k + 1) % 4]);
            for i in 0..n {
                let t = i as f64 / n as f64;
                pts.push(self.map_uv(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, hom.as_ref()));
            }
        }
        pts
    }

    /// Verzerrt einen Pfad (in denselben Koordinaten wie `src`); Abweichung höchstens `tol`.
    pub fn apply(&self, path: &VectorPath, tol: f64) -> VectorPath {
        let hom = self.homography();
        let f = |p: Point| {
            let (u, v) = self.uv(p);
            self.map_uv(u, v, hom.as_ref())
        };
        let tol = tol.max(1e-5);
        let subpaths = path
            .subpaths
            .iter()
            .map(|sp| {
                let mut segments = Vec::with_capacity(sp.segments.len());
                let mut a = sp.start;
                let closing = (self.kind == WarpKind::Envelope && sp.closed && sp.end().distance(sp.start) > 1e-9)
                    .then_some(Segment::Line(sp.start));
                for seg in sp.segments.iter().chain(closing.iter()) {
                    match (self.kind, *seg) {
                        (WarpKind::Perspective, Segment::Line(b)) => segments.push(Segment::Line(f(b))),
                        _ => map_segment(&f, a, seg, 0.0, 1.0, tol, 0, &mut segments),
                    }
                    a = seg.end();
                }
                SubPath { start: f(sp.start), segments, closed: sp.closed }
            })
            .collect();
        VectorPath { subpaths }
    }
}

/// Punkt auf dem Segment ab `a` beim Parameter `t`.
fn eval(a: Point, seg: &Segment, t: f64) -> Point {
    let s = 1.0 - t;
    match *seg {
        Segment::Line(b) => a.lerp(b, t),
        Segment::Quad(c, b) => a * (s * s) + c * (2.0 * s * t) + b * (t * t),
        Segment::Cubic(c1, c2, b) => {
            a * (s * s * s) + c1 * (3.0 * s * s * t) + c2 * (3.0 * s * t * t) + b * (t * t * t)
        }
    }
}

fn cubic_at(p: [Point; 4], t: f64) -> Point {
    eval(p[0], &Segment::Cubic(p[1], p[2], p[3]), t)
}

/// Bildet das Stück `t0..t1` des Segments ab: kubische Kurve durch die Bilder bei 0, ⅓, ⅔ und 1, bei zu großer
/// Abweichung halbiert. Fast gerade Stücke werden Linien.
#[allow(clippy::too_many_arguments)]
fn map_segment(
    f: &impl Fn(Point) -> Point,
    a: Point,
    seg: &Segment,
    t0: f64,
    t1: f64,
    tol: f64,
    depth: u32,
    out: &mut Vec<Segment>,
) {
    let at = |k: f64| f(eval(a, seg, t0 + (t1 - t0) * k));
    let q = [at(0.0), at(1.0 / 3.0), at(2.0 / 3.0), at(1.0)];
    // Kontrollpunkte so, dass die Kurve bei ⅓ und ⅔ durch q1 und q2 geht
    let r1 = q[1] * 27.0 - q[0] * 8.0 - q[3];
    let r2 = q[2] * 27.0 - q[0] - q[3] * 8.0;
    let c = [q[0], (r1 * 2.0 - r2) * (1.0 / 18.0), (r2 * 2.0 - r1) * (1.0 / 18.0), q[3]];
    if depth < MAX_DEPTH {
        let err = [1.0 / 6.0, 0.5, 5.0 / 6.0].iter().map(|&k| cubic_at(c, k).distance(at(k))).fold(0.0, f64::max);
        if err > tol {
            let mid = 0.5 * (t0 + t1);
            map_segment(f, a, seg, t0, mid, tol, depth + 1, out);
            map_segment(f, a, seg, mid, t1, tol, depth + 1, out);
            return;
        }
    }
    let chord = c[3] - c[0];
    let len = chord.length();
    let off = |p: Point| if len < 1e-12 { p.distance(c[0]) } else { (p - c[0]).cross(chord).abs() / len };
    if off(c[1]).max(off(c[2])) <= 0.1 * tol {
        out.push(Segment::Line(c[3]));
    } else {
        out.push(Segment::Cubic(c[1], c[2], c[3]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Affine;

    fn src() -> Rect {
        Rect::from_xywh(10.0, 20.0, 100.0, 50.0)
    }

    #[test]
    fn rest_position_is_identity() {
        for kind in WarpKind::ALL {
            let w = Warp::new(src(), kind);
            assert!(w.is_identity() && w.is_valid());
            for p in [Point::new(10.0, 20.0), Point::new(60.0, 45.0), Point::new(33.0, 61.0)] {
                assert!(w.map(p).distance(p) < 1e-9, "{kind:?} {p:?}");
            }
            let circle = VectorPath::ellipse(Point::new(60.0, 45.0), 20.0, 20.0);
            let out = w.apply(&circle, 0.01);
            let (a, b) = (circle.flatten(&Affine::IDENTITY, 0.01), out.flatten(&Affine::IDENTITY, 0.01));
            for p in &b[0].points {
                assert!((p.distance(Point::new(60.0, 45.0)) - 20.0).abs() < 0.03, "{kind:?}");
            }
            assert_eq!(a.len(), b.len());
        }
    }

    #[test]
    fn perspective_hits_corners_and_keeps_lines_straight() {
        let mut w = Warp::new(src(), WarpKind::Perspective);
        w.handles[1] = Point::new(90.0, 30.0);
        w.handles[2] = Point::new(90.0, 60.0);
        assert!(w.is_valid());
        assert!(w.map(Point::new(110.0, 20.0)).distance(Point::new(90.0, 30.0)) < 1e-9);
        assert!(w.map(Point::new(110.0, 70.0)).distance(Point::new(90.0, 60.0)) < 1e-9);
        // Diagonale bleibt eine einzige Linie
        let line = VectorPath::line(Point::new(10.0, 20.0), Point::new(110.0, 70.0));
        let out = w.apply(&line, 0.01);
        match out.subpaths[0].segments[..] {
            [Segment::Line(p)] => assert!(p.distance(Point::new(90.0, 60.0)) < 1e-9),
            ref other => panic!("{other:?}"),
        }
        // Mitte rückt zur kurzen Seite: Perspektive, nicht bilinear
        let m = w.map(Point::new(60.0, 45.0));
        assert!(m.x > 50.0 + 1.0, "{m:?}");
        // überkreuzte Ecken sind ungültig
        w.handles.swap(1, 2);
        assert!(!w.is_valid());
    }

    #[test]
    fn envelope_bends_edges_within_tolerance() {
        let mut w = Warp::new(src(), WarpKind::Envelope);
        w.handles[4] = Point::new(60.0, 0.0); // obere Kantenmitte 20 mm nach oben
        assert!(w.is_valid());
        let rect = VectorPath::rect(10.0, 20.0, 100.0, 50.0, 0.0);
        let out = w.apply(&rect, 0.01);
        // obere Kante wird zur Parabel durch die drei Griffe
        let top_mid = w.map(Point::new(60.0, 20.0));
        assert!(top_mid.distance(Point::new(60.0, 0.0)) < 1e-9);
        let poly = &out.flatten(&Affine::IDENTITY, 0.005)[0];
        let min_y = poly.points.iter().map(|p| p.y).fold(f64::MAX, f64::min);
        assert!((min_y - 0.0).abs() < 0.02, "{min_y}");
        // jeder Punkt der verzerrten Kontur liegt auf dem exakten Bild
        for k in 0..=40 {
            let x = 10.0 + 100.0 * k as f64 / 40.0;
            let exact = w.map(Point::new(x, 20.0));
            let d = poly.distance_to(exact);
            assert!(d < 0.02, "{x} {d}");
        }
        // die untere Kante bleibt gerade (Linien)
        assert!(out.subpaths[0].segments.iter().any(|s| matches!(s, Segment::Line(_))));
        // umgeklappt: ungültig
        w.handles[4] = Point::new(60.0, 200.0);
        assert!(!w.is_valid());
    }

    #[test]
    fn switching_to_envelope_keeps_shape() {
        let mut w = Warp::new(src(), WarpKind::Perspective);
        w.handles[1] = Point::new(90.0, 30.0);
        w.handles[2] = Point::new(90.0, 60.0);
        let before = w.map(Point::new(60.0, 20.0));
        w.set_kind(WarpKind::Envelope);
        assert!(w.map(Point::new(60.0, 20.0)).distance(before) < 1e-9);
    }
}
