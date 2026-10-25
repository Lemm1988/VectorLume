//! Ecken abrunden oder abschrägen: Ecken zwischen zwei geraden Segmenten werden durch einen
//! tangentialen Kreisbogen bzw. eine Fase ersetzt. Kurven bleiben unverändert.

use crate::geom::Point;
use crate::nodes::NodeRef;
use crate::path::{PathBuilder, Segment, SubPath, VectorPath};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CornerStyle {
    /// Kreisbogen mit dem angegebenen Radius
    Round,
    /// Gerade Fase; der Wert ist der Rückversatz entlang jeder Kante
    Chamfer,
}

/// Ersetzt Ecken mit mindestens `min_turn_deg` Richtungsänderung. `size` ist der Radius (rund) bzw. der
/// Rückversatz je Kante (Fase); er wird gekürzt, damit benachbarte Ecken sich die Kante teilen können.
/// `only` beschränkt auf diese Knoten (Zählweise wie in [`crate::nodes`]). Liefert die Zahl der Ecken.
pub fn round_corners(
    path: &VectorPath,
    style: CornerStyle,
    size: f64,
    min_turn_deg: f64,
    only: Option<&[NodeRef]>,
) -> (VectorPath, usize) {
    let mut out = VectorPath::default();
    let mut count = 0;
    if size <= 0.0 {
        return (path.clone(), 0);
    }
    for (si, sp) in path.subpaths.iter().enumerate() {
        let (b, n) = subpath(sp, si, style, size, min_turn_deg.to_radians(), only);
        out.append(b);
        count += n;
    }
    (out, count)
}

/// Ecke an einem Knoten: Berührpunkte auf eingehender und ausgehender Kante.
struct Corner {
    t1: Point,
    t2: Point,
    vertex: Point,
}

fn subpath(
    sp: &SubPath,
    si: usize,
    style: CornerStyle,
    size: f64,
    min_turn: f64,
    only: Option<&[NodeRef]>,
) -> (VectorPath, usize) {
    // Geschlossene Teilpfade enden ausdrücklich auf dem Start, damit jeder Knoten zwei Kanten hat
    let mut segs = sp.segments.clone();
    if sp.closed && sp.end().distance(sp.start) > 1e-9 {
        segs.push(Segment::Line(sp.start));
    }
    let n = segs.len();
    let start_of = |i: usize| if i == 0 { sp.start } else { segs[i - 1].end() };
    // Knoten k liegt zwischen Segment k-1 (eingehend) und Segment k (ausgehend)
    let incoming = |k: usize| if k == 0 { sp.closed.then(|| n - 1) } else { Some(k - 1) };
    let outgoing = |k: usize| (k < n).then_some(k);
    let wanted = |k: usize| {
        only.is_none_or(|o| o.iter().any(|r| r.sub == si && (r.idx == k || (sp.closed && k == 0 && r.idx == n))))
    };
    let line_len = |i: usize| match segs[i] {
        Segment::Line(p) => Some(start_of(i).distance(p)),
        _ => None,
    };

    // 1. Kandidaten: Ecken zwischen zwei Linien mit genügend Richtungsänderung
    let nodes = if sp.closed { n } else { n + 1 };
    let mut half_angle = vec![None::<f64>; nodes];
    for (k, slot) in half_angle.iter_mut().enumerate() {
        let (Some(i), Some(o)) = (incoming(k), outgoing(k)) else { continue };
        let (Some(li), Some(lo)) = (line_len(i), line_len(o)) else { continue };
        if li < 1e-9 || lo < 1e-9 || !wanted(k) {
            continue;
        }
        let v = start_of(o);
        let u1 = (start_of(i) - v) * (1.0 / li);
        let u2 = (segs[o].end() - v) * (1.0 / lo);
        let theta = u1.dot(u2).clamp(-1.0, 1.0).acos();
        if std::f64::consts::PI - theta >= min_turn.max(1e-6) && theta > 1e-6 {
            *slot = Some(theta / 2.0);
        }
    }

    // 2. Berührpunkte; eine Kante zwischen zwei Ecken wird geteilt
    let mut corners: Vec<Option<Corner>> = (0..nodes).map(|_| None).collect();
    let mut count = 0;
    for k in 0..nodes {
        let Some(half) = half_angle[k] else { continue };
        let (i, o) = (incoming(k).unwrap(), outgoing(k).unwrap());
        let (li, lo) = (line_len(i).unwrap(), line_len(o).unwrap());
        // Kante i läuft von Knoten i nach k, Kante o von k nach o + 1
        let other_out = (o + 1) % nodes;
        let share_in = if i != k && half_angle[i].is_some() { 0.5 } else { 1.0 };
        let share_out = if other_out != k && half_angle[other_out].is_some() { 0.5 } else { 1.0 };
        let wanted_d = match style {
            CornerStyle::Round => size / half.tan(),
            CornerStyle::Chamfer => size,
        };
        let d = wanted_d.min(li * share_in).min(lo * share_out);
        if d < 1e-6 {
            continue;
        }
        let v = start_of(o);
        let t1 = v + (start_of(i) - v) * (d / li);
        let t2 = v + (segs[o].end() - v) * (d / lo);
        corners[k] = Some(Corner { t1, t2, vertex: v });
        count += 1;
    }
    if count == 0 {
        return (VectorPath { subpaths: vec![sp.clone()] }, 0);
    }

    // 3. Neu aufbauen
    let mut b = PathBuilder::new();
    b.move_to(corners[0].as_ref().map_or(sp.start, |c| c.t2));
    for (i, seg) in segs.iter().enumerate() {
        let end_node = if sp.closed { (i + 1) % n } else { i + 1 };
        match corners[end_node].as_ref() {
            Some(c) => {
                b.line_to(c.t1);
                match style {
                    CornerStyle::Chamfer => b.line_to(c.t2),
                    CornerStyle::Round => arc(&mut b, c),
                }
            }
            None => match *seg {
                Segment::Line(p) => b.line_to(p),
                Segment::Quad(c, p) => b.quad_to(c, p),
                Segment::Cubic(c1, c2, p) => b.cubic_to(c1, c2, p),
            },
        }
    }
    if sp.closed {
        b.close();
    }
    (b.finish(), count)
}

/// Tangentialer Kreisbogen von `t1` nach `t2` (gleich weit von der Ecke entfernt).
fn arc(b: &mut PathBuilder, c: &Corner) {
    let (a, e) = (c.t1 - c.vertex, c.t2 - c.vertex);
    let d = a.length();
    let sum = a * (1.0 / d) + e * (1.0 / e.length());
    let bis = sum * (1.0 / sum.length());
    let half = (a.dot(e) / (d * e.length())).clamp(-1.0, 1.0).acos() / 2.0;
    let r = d * half.tan();
    let center = c.vertex + bis * (r / half.sin());
    let start = (c.t1 - center).y.atan2((c.t1 - center).x);
    let end = (c.t2 - center).y.atan2((c.t2 - center).x);
    let mut sweep = end - start;
    // kürzerer Weg (der Bogen ist immer kleiner als ein Halbkreis)
    if sweep > std::f64::consts::PI {
        sweep -= std::f64::consts::TAU;
    } else if sweep < -std::f64::consts::PI {
        sweep += std::f64::consts::TAU;
    }
    b.arc(center, r, r, 0.0, start, sweep);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> VectorPath {
        VectorPath::polygon(
            &[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)],
            true,
        )
    }

    fn area(p: &VectorPath) -> f64 {
        p.flatten(&crate::geom::Affine::IDENTITY, 0.001).iter().map(|l| l.signed_area().abs()).sum()
    }

    #[test]
    fn rounds_all_square_corners() {
        let (p, n) = round_corners(&square(), CornerStyle::Round, 2.0, 10.0, None);
        assert_eq!(n, 4);
        // Fläche: 100 − 4·(r² − π r²/4)
        let want = 100.0 - 4.0 * (4.0 - std::f64::consts::PI);
        assert!((area(&p) - want).abs() < 0.01, "{}", area(&p));
        assert!(p.subpaths[0].closed);
        // Der Pfad bleibt geschlossen und beginnt auf der unteren Kante
        assert!((p.subpaths[0].start.y).abs() < 1e-9 && (p.subpaths[0].start.x - 2.0).abs() < 1e-9);
    }

    #[test]
    fn chamfer_and_radius_limit() {
        let (p, n) = round_corners(&square(), CornerStyle::Chamfer, 1.0, 10.0, None);
        assert_eq!(n, 4);
        assert!((area(&p) - (100.0 - 4.0 * 0.5)).abs() < 1e-6);
        // Riesiger Radius wird auf die halbe Kante begrenzt → Kreis mit r = 5
        let (p, _) = round_corners(&square(), CornerStyle::Round, 50.0, 10.0, None);
        assert!((area(&p) - std::f64::consts::PI * 25.0).abs() < 0.05, "{}", area(&p));
    }

    #[test]
    fn only_selected_node_and_open_paths() {
        let only = [NodeRef { sub: 0, idx: 2 }];
        let (p, n) = round_corners(&square(), CornerStyle::Chamfer, 1.0, 10.0, Some(&only));
        assert_eq!(n, 1);
        assert!((area(&p) - 99.5).abs() < 1e-6);
        // Offener Winkel: nur der mittlere Knoten ist eine Ecke, die Enden bleiben
        let l = VectorPath::polygon(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0)], false);
        let (p, n) = round_corners(&l, CornerStyle::Round, 3.0, 10.0, None);
        assert_eq!(n, 1);
        let sp = &p.subpaths[0];
        assert_eq!(sp.start, Point::new(0.0, 0.0));
        assert_eq!(sp.end(), Point::new(10.0, 10.0));
        // flache Knicks unter dem Mindestwinkel bleiben
        let flat = VectorPath::polygon(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(20.0, 1.0)], false);
        assert_eq!(round_corners(&flat, CornerStyle::Round, 3.0, 10.0, None).1, 0);
    }
}
