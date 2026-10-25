//! Knotenbearbeitung von Vektorpfaden: Knoten und Kurvengriffe verschieben, einfügen, löschen,
//! Segmente in Linien oder Kurven umwandeln, Pfade auftrennen, Knoten glätten, offene Enden verlängern.
//!
//! Knoten `idx` eines Teilpfades: 0 = Startpunkt, `i` > 0 = Endpunkt von Segment `i - 1`.
//! Bei geschlossenen Teilpfaden, deren letztes Segment auf den Startpunkt zurückführt, gilt dieser letzte
//! Endpunkt als derselbe Knoten wie der Start.

use crate::geom::Point;
use crate::path::{Polyline, Segment, SubPath, VectorPath, distance_to_segment};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeRef {
    pub sub: usize,
    pub idx: usize,
}

/// Kurvengriff: `which` = 0 für den ersten, 1 für den zweiten Kontrollpunkt eines Segments.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HandleRef {
    pub sub: usize,
    pub seg: usize,
    pub which: u8,
}

const SAME: f64 = 1e-9;

fn closes_on_start(sp: &SubPath) -> bool {
    sp.closed && sp.segments.last().is_some_and(|s| s.end().distance(sp.start) < SAME)
}

/// Sichtbare Knoten (ohne den doppelten Schlussknoten geschlossener Pfade).
pub fn nodes(path: &VectorPath) -> Vec<(NodeRef, Point)> {
    let mut out = Vec::new();
    for (si, sp) in path.subpaths.iter().enumerate() {
        out.push((NodeRef { sub: si, idx: 0 }, sp.start));
        let n = sp.segments.len();
        let last_dup = closes_on_start(sp);
        for (i, seg) in sp.segments.iter().enumerate() {
            if last_dup && i + 1 == n {
                continue;
            }
            out.push((NodeRef { sub: si, idx: i + 1 }, seg.end()));
        }
    }
    out
}

/// Kurvengriffe der Segmente, die an den Knoten `node` grenzen: (Griff, Griffposition, zugehöriger Knoten).
pub fn handles_near(path: &VectorPath, node: NodeRef) -> Vec<(HandleRef, Point, Point)> {
    let Some(sp) = path.subpaths.get(node.sub) else { return Vec::new() };
    let n = sp.segments.len();
    let mut segs = Vec::new();
    // ausgehendes Segment
    if node.idx < n {
        segs.push(node.idx);
    }
    // eingehendes Segment (bei Knoten 0 eines geschlossenen Pfades das letzte)
    if node.idx > 0 {
        segs.push(node.idx - 1);
    } else if closes_on_start(sp) && n > 0 {
        segs.push(n - 1);
    }
    let mut out = Vec::new();
    for seg in segs {
        let from = if seg == 0 { sp.start } else { sp.segments[seg - 1].end() };
        match sp.segments[seg] {
            Segment::Cubic(c1, c2, to) => {
                out.push((HandleRef { sub: node.sub, seg, which: 0 }, c1, from));
                out.push((HandleRef { sub: node.sub, seg, which: 1 }, c2, to));
            }
            Segment::Quad(c, to) => {
                let anchor = if seg == node.idx { from } else { to };
                out.push((HandleRef { sub: node.sub, seg, which: 0 }, c, anchor));
            }
            Segment::Line(_) => {}
        }
    }
    out
}

fn set_end(seg: &mut Segment, p: Point) {
    match seg {
        Segment::Line(e) | Segment::Quad(_, e) | Segment::Cubic(_, _, e) => *e = p,
    }
}

/// Verschiebt einen Knoten; angrenzende Kurvengriffe wandern mit.
pub fn move_node(path: &mut VectorPath, node: NodeRef, to: Point) {
    let Some(sp) = path.subpaths.get_mut(node.sub) else { return };
    let n = sp.segments.len();
    let old = if node.idx == 0 { sp.start } else { sp.segments[node.idx - 1].end() };
    let d = to - old;
    let dup = closes_on_start(sp);
    // eingehendes Segment: Endpunkt und zweiter Griff
    let incoming = if node.idx > 0 {
        Some(node.idx - 1)
    } else if dup && n > 0 {
        Some(n - 1)
    } else {
        None
    };
    if let Some(i) = incoming {
        match &mut sp.segments[i] {
            Segment::Cubic(_, c2, e) => {
                *c2 += d;
                *e = to;
            }
            seg => set_end(seg, to),
        }
    }
    if node.idx == 0 {
        sp.start = to;
    }
    // ausgehendes Segment: erster Griff
    if node.idx < n
        && let Segment::Cubic(c1, _, _) = &mut sp.segments[node.idx]
    {
        *c1 += d;
    }
}

pub fn move_handle(path: &mut VectorPath, h: HandleRef, to: Point) {
    let Some(seg) = path.subpaths.get_mut(h.sub).and_then(|s| s.segments.get_mut(h.seg)) else { return };
    match (seg, h.which) {
        (Segment::Cubic(c1, _, _), 0) | (Segment::Quad(c1, _), 0) => *c1 = to,
        (Segment::Cubic(_, c2, _), 1) => *c2 = to,
        _ => {}
    }
}

fn segment_start(sp: &SubPath, seg: usize) -> Point {
    if seg == 0 { sp.start } else { sp.segments[seg - 1].end() }
}

/// Punkt auf einem Segment bei Parameter t.
pub fn segment_point(from: Point, seg: &Segment, t: f64) -> Point {
    let mt = 1.0 - t;
    match *seg {
        Segment::Line(e) => from.lerp(e, t),
        Segment::Quad(c, e) => from * (mt * mt) + c * (2.0 * mt * t) + e * (t * t),
        Segment::Cubic(c1, c2, e) => {
            from * (mt * mt * mt) + c1 * (3.0 * mt * mt * t) + c2 * (3.0 * mt * t * t) + e * (t * t * t)
        }
    }
}

/// Teilt ein Segment bei t (de Casteljau) in zwei.
fn split(from: Point, seg: Segment, t: f64) -> (Segment, Segment) {
    match seg {
        Segment::Line(e) => (Segment::Line(from.lerp(e, t)), Segment::Line(e)),
        Segment::Quad(c, e) => {
            let a = from.lerp(c, t);
            let b = c.lerp(e, t);
            let m = a.lerp(b, t);
            (Segment::Quad(a, m), Segment::Quad(b, e))
        }
        Segment::Cubic(c1, c2, e) => {
            let a = from.lerp(c1, t);
            let b = c1.lerp(c2, t);
            let c = c2.lerp(e, t);
            let ab = a.lerp(b, t);
            let bc = b.lerp(c, t);
            let m = ab.lerp(bc, t);
            (Segment::Cubic(a, ab, m), Segment::Cubic(bc, c, e))
        }
    }
}

/// Macht die Schlusskante geschlossener Teilpfade explizit, damit auf ihr Knoten eingefügt werden können.
fn explicit_close(sp: &mut SubPath) {
    if sp.closed && !sp.segments.is_empty() && !closes_on_start(sp) {
        sp.segments.push(Segment::Line(sp.start));
    }
}

/// Nächstgelegene Stelle auf dem Pfad: (Teilpfad, Segment, t, Abstand). Schlusskanten zählen mit.
pub fn nearest_on_path(path: &VectorPath, p: Point) -> Option<(usize, usize, f64, f64)> {
    let mut best: Option<(usize, usize, f64, f64)> = None;
    for (si, sp) in path.subpaths.iter().enumerate() {
        let mut sp = sp.clone();
        explicit_close(&mut sp);
        for (gi, seg) in sp.segments.iter().enumerate() {
            let from = segment_start(&sp, gi);
            // grob abtasten, dann um das beste Stück verfeinern
            let samples = if matches!(seg, Segment::Line(_)) { 1 } else { 48 };
            let mut prev = from;
            for k in 1..=samples {
                let t1 = k as f64 / samples as f64;
                let q = segment_point(from, seg, t1);
                let d = distance_to_segment(p, prev, q);
                if best.is_none_or(|b| d < b.3) {
                    // t innerhalb des Teilstücks per Projektion schätzen
                    let ab = q - prev;
                    let len_sq = ab.dot(ab).max(1e-18);
                    let local = ((p - prev).dot(ab) / len_sq).clamp(0.0, 1.0);
                    let t = (k as f64 - 1.0 + local) / samples as f64;
                    best = Some((si, gi, t, d));
                }
                prev = q;
            }
        }
    }
    best
}

/// Fügt bei (Segment, t) einen Knoten ein und gibt ihn zurück.
pub fn insert_node(path: &mut VectorPath, sub: usize, seg: usize, t: f64) -> Option<NodeRef> {
    let sp = path.subpaths.get_mut(sub)?;
    explicit_close(sp);
    if seg >= sp.segments.len() {
        return None;
    }
    let t = t.clamp(0.01, 0.99);
    let from = segment_start(sp, seg);
    let (a, b) = split(from, sp.segments[seg], t);
    sp.segments[seg] = a;
    sp.segments.insert(seg + 1, b);
    Some(NodeRef { sub, idx: seg + 1 })
}

/// Löscht einen Knoten; die angrenzenden Segmente werden zusammengefasst. Teilpfade mit weniger als
/// zwei Knoten verschwinden.
pub fn delete_node(path: &mut VectorPath, node: NodeRef) {
    let Some(sp) = path.subpaths.get_mut(node.sub) else { return };
    let n = sp.segments.len();
    let dup = closes_on_start(sp);
    if node.idx == 0 {
        if dup && n >= 2 {
            // Startknoten eines geschlossenen Pfades: Start auf den nächsten Knoten legen und
            // erstes mit letztem Segment verbinden.
            let first = sp.segments.remove(0);
            let new_start = first.end();
            let last = sp.segments.len() - 1;
            sp.segments[last] = merge(sp.segments[last], first, new_start);
            sp.start = new_start;
        } else if n >= 1 {
            sp.start = sp.segments.remove(0).end();
        }
    } else if node.idx == n {
        sp.segments.pop();
    } else if node.idx < n {
        let a = sp.segments[node.idx - 1];
        let b = sp.segments.remove(node.idx);
        sp.segments[node.idx - 1] = merge(a, b, b.end());
    }
    let count = sp.segments.len() + if closes_on_start(sp) { 0 } else { 1 };
    if sp.segments.is_empty() || count < 2 {
        path.subpaths.remove(node.sub);
    }
}

/// Fasst zwei aufeinanderfolgende Segmente zu einem zusammen (Kurve, falls eines davon gekrümmt ist).
fn merge(a: Segment, b: Segment, end: Point) -> Segment {
    match (a, b) {
        (Segment::Line(_), Segment::Line(_)) => Segment::Line(end),
        _ => {
            let c1 = match a {
                Segment::Cubic(c1, _, _) | Segment::Quad(c1, _) => c1,
                Segment::Line(e) => e,
            };
            let c2 = match b {
                Segment::Cubic(_, c2, _) | Segment::Quad(c2, _) => c2,
                Segment::Line(_) => end,
            };
            Segment::Cubic(c1, c2, end)
        }
    }
}

/// Wandelt das Segment, das am Knoten endet (bzw. bei Knoten 0 das ausgehende), in eine Linie oder Kurve um.
pub fn convert_segment(path: &mut VectorPath, node: NodeRef, to_curve: bool) {
    let Some(sp) = path.subpaths.get_mut(node.sub) else { return };
    explicit_close(sp);
    let n = sp.segments.len();
    if n == 0 {
        return;
    }
    let seg = if node.idx == 0 { 0 } else { (node.idx - 1).min(n - 1) };
    let from = segment_start(sp, seg);
    let end = sp.segments[seg].end();
    sp.segments[seg] = if to_curve {
        match sp.segments[seg] {
            Segment::Line(e) => Segment::Cubic(from.lerp(e, 1.0 / 3.0), from.lerp(e, 2.0 / 3.0), e),
            Segment::Quad(c, e) => Segment::Cubic(from.lerp(c, 2.0 / 3.0), e.lerp(c, 2.0 / 3.0), e),
            s => s,
        }
    } else {
        Segment::Line(end)
    };
}

/// Öffnet einen geschlossenen Teilpfad bzw. schließt einen offenen.
pub fn toggle_closed(path: &mut VectorPath, sub: usize) {
    if let Some(sp) = path.subpaths.get_mut(sub) {
        if sp.closed {
            explicit_close(sp);
            sp.closed = false;
        } else if sp.segments.len() >= 2 {
            sp.closed = true;
        }
    }
}

/// Trennt den Pfad am Knoten auf: ein geschlossener Teilpfad wird dort geöffnet (Anfang und Ende liegen dann auf
/// dem Knoten), ein offener in zwei Teilpfade geteilt. Gibt den Anfangsknoten des neuen Stücks zurück; an den
/// Enden offener Pfade gibt es nichts aufzutrennen.
pub fn break_at_node(path: &mut VectorPath, node: NodeRef) -> Option<NodeRef> {
    let sp = path.subpaths.get_mut(node.sub)?;
    if sp.closed {
        explicit_close(sp);
        if node.idx >= sp.segments.len() {
            return None;
        }
        sp.start = segment_start(sp, node.idx);
        sp.segments.rotate_left(node.idx);
        sp.closed = false;
        return Some(NodeRef { sub: node.sub, idx: 0 });
    }
    if node.idx == 0 || node.idx >= sp.segments.len() {
        return None;
    }
    let start = segment_start(sp, node.idx);
    let segments = sp.segments.split_off(node.idx);
    path.subpaths.insert(node.sub + 1, SubPath { start, segments, closed: false });
    Some(NodeRef { sub: node.sub + 1, idx: 0 })
}

fn as_cubic(from: Point, seg: Segment) -> (Point, Point, Point) {
    match seg {
        Segment::Line(e) => (from.lerp(e, 1.0 / 3.0), from.lerp(e, 2.0 / 3.0), e),
        Segment::Quad(c, e) => (from.lerp(c, 2.0 / 3.0), e.lerp(c, 2.0 / 3.0), e),
        Segment::Cubic(c1, c2, e) => (c1, c2, e),
    }
}

/// Macht den Knoten glatt: die Griffe der beiden angrenzenden Segmente liegen danach auf einer Geraden parallel
/// zur Verbindung der Nachbarknoten (gerade Segmente werden zu Kurven, Grifflängen bleiben). Endknoten offener
/// Pfade haben nur ein Segment und bleiben unverändert (`false`).
pub fn smooth_node(path: &mut VectorPath, node: NodeRef) -> bool {
    let Some(sp) = path.subpaths.get_mut(node.sub) else { return false };
    explicit_close(sp);
    let n = sp.segments.len();
    let (inc, out) = match node.idx {
        0 if sp.closed && n >= 2 => (n - 1, 0),
        i if i > 0 && i < n => (i - 1, i),
        _ => return false,
    };
    let (a, p, b) = (segment_start(sp, inc), segment_start(sp, out), sp.segments[out].end());
    let span = b - a;
    if span.length() < SAME {
        return false;
    }
    let d = span * (1.0 / span.length());
    let (c1, c2, e) = as_cubic(a, sp.segments[inc]);
    let len = (c2 - p).length();
    let len = if len > SAME { len } else { (p - a).length() / 3.0 };
    sp.segments[inc] = Segment::Cubic(c1, p - d * len, e);
    let (c1, c2, e) = as_cubic(p, sp.segments[out]);
    let len = (c1 - p).length();
    let len = if len > SAME { len } else { (b - p).length() / 3.0 };
    sp.segments[out] = Segment::Cubic(p + d * len, c2, e);
    true
}

/// Endpunkt und Richtung (nach außen) eines offenen Teilpfades am Anfang bzw. Ende.
fn end_tangent(sp: &SubPath, at_start: bool) -> Option<(Point, Point)> {
    let (tip, inner) = if at_start {
        let seg = sp.segments.first()?;
        let inner = match *seg {
            Segment::Line(e) => vec![e],
            Segment::Quad(c, e) => vec![c, e],
            Segment::Cubic(c1, c2, e) => vec![c1, c2, e],
        };
        (sp.start, inner)
    } else {
        let seg = sp.segments.last()?;
        let from = segment_start(sp, sp.segments.len() - 1);
        let inner = match *seg {
            Segment::Line(_) => vec![from],
            Segment::Quad(c, _) => vec![c, from],
            Segment::Cubic(c1, c2, _) => vec![c2, c1, from],
        };
        (seg.end(), inner)
    };
    let q = inner.into_iter().find(|q| q.distance(tip) > SAME)?;
    let d = tip - q;
    Some((tip, d * (1.0 / d.length())))
}

/// Nächster Schnitt des Strahls `o + t·d` (t > 0) mit den Kanten der Hindernisse.
fn ray_hit(o: Point, d: Point, obstacles: &[Polyline]) -> Option<Point> {
    let mut best: Option<f64> = None;
    for (p, q) in obstacles.iter().flat_map(|poly| poly.edges()) {
        let e = q - p;
        let den = d.cross(e);
        if den.abs() < 1e-12 {
            continue;
        }
        let w = p - o;
        let (t, u) = (w.cross(e) / den, w.cross(d) / den);
        if t > 1e-6 && (-1e-9..=1.0 + 1e-9).contains(&u) && best.is_none_or(|b| t < b) {
            best = Some(t);
        }
    }
    best.map(|t| o + d * t)
}

/// Ist der Knoten ein Ende eines offenen Teilpfades?
pub fn is_open_end(path: &VectorPath, node: NodeRef) -> bool {
    path.subpaths
        .get(node.sub)
        .is_some_and(|sp| !sp.closed && !sp.segments.is_empty() && (node.idx == 0 || node.idx == sp.segments.len()))
}

/// Verlängert ein offenes Pfadende in Richtung seiner Tangente bis zur ersten Linie aus `obstacles` (gleiche
/// Koordinaten wie `path`, der Pfad selbst darf darin vorkommen). Ein gerades Endsegment wird verlängert, an
/// eine Kurve wird eine Linie angehängt. Gibt den neuen Endknoten zurück, `None` ohne Treffer.
pub fn extend_end(path: &mut VectorPath, node: NodeRef, obstacles: &[Polyline]) -> Option<NodeRef> {
    if !is_open_end(path, node) {
        return None;
    }
    let sp = &mut path.subpaths[node.sub];
    let at_start = node.idx == 0;
    let (tip, d) = end_tangent(sp, at_start)?;
    let hit = ray_hit(tip, d, obstacles)?;
    if at_start {
        if !matches!(sp.segments[0], Segment::Line(_)) {
            sp.segments.insert(0, Segment::Line(sp.start));
        }
        sp.start = hit;
        return Some(NodeRef { sub: node.sub, idx: 0 });
    }
    match sp.segments.last_mut() {
        Some(Segment::Line(e)) => *e = hit,
        _ => sp.segments.push(Segment::Line(hit)),
    }
    Some(NodeRef { sub: node.sub, idx: sp.segments.len() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Affine;

    fn square() -> VectorPath {
        VectorPath::rect(0.0, 0.0, 10.0, 10.0, 0.0)
    }

    #[test]
    fn rect_has_four_nodes_and_moves_them() {
        let mut p = square();
        assert_eq!(nodes(&p).len(), 4);
        move_node(&mut p, NodeRef { sub: 0, idx: 2 }, Point::new(20.0, 20.0));
        let b = p.bounds(&Affine::IDENTITY);
        assert_eq!((b.max.x, b.max.y), (20.0, 20.0));
    }

    #[test]
    fn closed_start_node_moves_closing_segment_too() {
        // Rechteck mit Eckenradius: letzte Kurve endet auf dem Startpunkt
        let mut p = VectorPath::rect(0.0, 0.0, 20.0, 10.0, 2.0);
        let count = nodes(&p).len();
        assert_eq!(count, 8);
        move_node(&mut p, NodeRef { sub: 0, idx: 0 }, Point::new(2.0, -5.0));
        let sp = &p.subpaths[0];
        assert_eq!(sp.segments.last().unwrap().end(), Point::new(2.0, -5.0));
    }

    #[test]
    fn insert_and_delete_round_trip() {
        let mut p = square();
        // Punkt auf der Schlusskante (links) einfügen
        let (sub, seg, t, d) = nearest_on_path(&p, Point::new(-0.1, 5.0)).unwrap();
        assert!(d < 0.2);
        let node = insert_node(&mut p, sub, seg, t).unwrap();
        assert_eq!(nodes(&p).len(), 5);
        let pos = nodes(&p).into_iter().find(|(n, _)| *n == node).unwrap().1;
        assert!(pos.distance(Point::new(0.0, 5.0)) < 0.05, "{pos:?}");
        delete_node(&mut p, node);
        assert_eq!(nodes(&p).len(), 4);
        let area = p.flatten(&Affine::IDENTITY, 0.01)[0].signed_area().abs();
        assert!((area - 100.0).abs() < 1e-6);
    }

    #[test]
    fn cubic_split_keeps_shape() {
        let mut p = VectorPath::ellipse(Point::new(0.0, 0.0), 10.0, 10.0);
        let before = p.flatten(&Affine::IDENTITY, 0.001)[0].length();
        insert_node(&mut p, 0, 1, 0.3).unwrap();
        let after = p.flatten(&Affine::IDENTITY, 0.001)[0].length();
        assert!((before - after).abs() < 0.01, "{before} {after}");
    }

    #[test]
    fn convert_and_delete_start_of_closed_path() {
        let mut p = square();
        convert_segment(&mut p, NodeRef { sub: 0, idx: 1 }, true);
        assert!(matches!(p.subpaths[0].segments[0], Segment::Cubic(..)));
        convert_segment(&mut p, NodeRef { sub: 0, idx: 1 }, false);
        assert!(matches!(p.subpaths[0].segments[0], Segment::Line(_)));
        // Dreieck übrig lassen
        let mut q = square();
        explicit_close(&mut q.subpaths[0]);
        delete_node(&mut q, NodeRef { sub: 0, idx: 0 });
        assert_eq!(nodes(&q).len(), 3);
        let area = q.flatten(&Affine::IDENTITY, 0.01)[0].signed_area().abs();
        assert!((area - 50.0).abs() < 1e-6, "{area}");
    }

    #[test]
    fn break_opens_closed_and_splits_open_paths() {
        let mut p = square();
        let n = break_at_node(&mut p, NodeRef { sub: 0, idx: 2 }).unwrap();
        let sp = &p.subpaths[n.sub];
        assert!(!sp.closed);
        assert_eq!(sp.start, Point::new(10.0, 10.0));
        assert_eq!(sp.end(), Point::new(10.0, 10.0));
        assert_eq!(sp.segments.len(), 4, "alle vier Kanten bleiben");

        // offener Pfad mit vier Kanten am mittleren Knoten teilen
        let n = break_at_node(&mut p, NodeRef { sub: 0, idx: 2 }).unwrap();
        assert_eq!(p.subpaths.len(), 2);
        assert_eq!((p.subpaths[0].segments.len(), p.subpaths[1].segments.len()), (2, 2));
        assert_eq!(p.subpaths[0].end(), p.subpaths[n.sub].start);
        assert_eq!(break_at_node(&mut p, NodeRef { sub: 0, idx: 0 }), None, "Enden bleiben");
    }

    #[test]
    fn smooth_makes_handles_collinear() {
        let mut p = square();
        let node = NodeRef { sub: 0, idx: 1 };
        assert!(smooth_node(&mut p, node));
        let sp = &p.subpaths[0];
        let (Segment::Cubic(_, c2, e), Segment::Cubic(c1, _, _)) = (sp.segments[0], sp.segments[1]) else {
            panic!("Kurven erwartet: {:?}", sp.segments)
        };
        assert_eq!(e, Point::new(10.0, 0.0));
        assert!((c2 - e).cross(c1 - e).abs() < 1e-9 && (c2 - e).dot(c1 - e) < 0.0, "Griffe gegenüber");
        // Startknoten des geschlossenen Pfades: letzte und erste Kante
        assert!(smooth_node(&mut p, NodeRef { sub: 0, idx: 0 }));
        assert!(matches!(p.subpaths[0].segments.last(), Some(Segment::Cubic(..))));
        let mut open = VectorPath::default();
        open.subpaths.push(SubPath {
            start: Point::new(0.0, 0.0),
            segments: vec![Segment::Line(Point::new(5.0, 0.0))],
            closed: false,
        });
        assert!(!smooth_node(&mut open, NodeRef { sub: 0, idx: 1 }), "Endknoten bleibt");
    }

    #[test]
    fn extend_reaches_next_line() {
        // Waagerechte Linie 0…5 zeigt auf eine senkrechte Wand bei x = 20, zurück auf eine bei x = -7
        let mut p = VectorPath::default();
        p.subpaths.push(SubPath {
            start: Point::new(0.0, 0.0),
            segments: vec![Segment::Line(Point::new(5.0, 0.0))],
            closed: false,
        });
        let walls = [
            Polyline::new(vec![Point::new(20.0, -5.0), Point::new(20.0, 5.0)], false),
            Polyline::new(vec![Point::new(30.0, -5.0), Point::new(30.0, 5.0)], false),
            Polyline::new(vec![Point::new(-7.0, -5.0), Point::new(-7.0, 5.0)], false),
        ];
        let mut obstacles = walls.to_vec();
        obstacles.extend(p.flatten(&Affine::IDENTITY, 0.01));
        let end = extend_end(&mut p, NodeRef { sub: 0, idx: 1 }, &obstacles).unwrap();
        assert_eq!(end, NodeRef { sub: 0, idx: 1 });
        assert_eq!(p.subpaths[0].end(), Point::new(20.0, 0.0));
        extend_end(&mut p, NodeRef { sub: 0, idx: 0 }, &obstacles).unwrap();
        assert_eq!(p.subpaths[0].start, Point::new(-7.0, 0.0));
        assert_eq!(p.subpaths[0].segments.len(), 1, "gerade Linie wird nur länger");
        assert!(extend_end(&mut p, NodeRef { sub: 0, idx: 0 }, &walls[..1]).is_none(), "nach links nichts mehr");

        // Kurvenende: eine Linie wird angehängt, Richtung = Tangente am Ende
        let mut c = VectorPath::default();
        c.subpaths.push(SubPath {
            start: Point::new(0.0, 0.0),
            segments: vec![Segment::Cubic(Point::new(2.0, 4.0), Point::new(4.0, 4.0), Point::new(6.0, 2.0))],
            closed: false,
        });
        let wall = [Polyline::new(vec![Point::new(20.0, -50.0), Point::new(20.0, 50.0)], false)];
        let end = extend_end(&mut c, NodeRef { sub: 0, idx: 1 }, &wall).unwrap();
        assert_eq!(end.idx, 2);
        let hit = c.subpaths[0].end();
        assert!((hit.x - 20.0).abs() < 1e-9 && (hit.y + 12.0).abs() < 1e-9, "{hit:?}");
        assert!(!is_open_end(&square(), NodeRef { sub: 0, idx: 0 }));
    }
}
