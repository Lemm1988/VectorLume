//! Aufräumen importierter Zeichnungen: offene Pfadstücke verbinden und schließen, doppelte Linien entfernen
//! (sonst schneidet der Laser dieselbe Linie zweimal).

use crate::geom::Point;
use crate::path::{Segment, SubPath, VectorPath};
use std::collections::{HashMap, HashSet};

impl SubPath {
    /// Derselbe Teilpfad in umgekehrter Richtung (Kurven bleiben erhalten).
    pub fn reversed(&self) -> SubPath {
        let mut segments = Vec::with_capacity(self.segments.len());
        let mut from = self.start;
        let mut starts = Vec::with_capacity(self.segments.len());
        for s in &self.segments {
            starts.push(from);
            from = s.end();
        }
        for (s, &to) in self.segments.iter().zip(&starts).rev() {
            segments.push(match *s {
                Segment::Line(_) => Segment::Line(to),
                Segment::Quad(c, _) => Segment::Quad(c, to),
                Segment::Cubic(c1, c2, _) => Segment::Cubic(c2, c1, to),
            });
        }
        SubPath { start: self.end(), segments, closed: self.closed }
    }
}

/// Ersetzt den Endpunkt eines Segments (Lücken bis zur Toleranz werden so geschlossen).
fn with_end(s: Segment, p: Point) -> Segment {
    match s {
        Segment::Line(_) => Segment::Line(p),
        Segment::Quad(c, _) => Segment::Quad(c, p),
        Segment::Cubic(a, b, _) => Segment::Cubic(a, b, p),
    }
}

/// Endpunkte offener Teilpfade, nach Rasterzellen der Toleranz einsortiert.
struct Ends {
    cell: f64,
    map: HashMap<(i64, i64), Vec<(usize, bool)>>,
}

impl Ends {
    fn key(&self, p: Point) -> (i64, i64) {
        ((p.x / self.cell).floor() as i64, (p.y / self.cell).floor() as i64)
    }

    fn insert(&mut self, p: Point, entry: (usize, bool)) {
        let k = self.key(p);
        self.map.entry(k).or_default().push(entry);
    }

    /// Nächstgelegener freier Endpunkt innerhalb `tol` (Index, `true` = Ende des Teilpfads).
    fn nearest(&self, p: Point, tol: f64, subs: &[SubPath], used: &[bool]) -> Option<(usize, bool)> {
        let (kx, ky) = self.key(p);
        let mut best: Option<((usize, bool), f64)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &(i, at_end) in self.map.get(&(kx + dx, ky + dy)).into_iter().flatten() {
                    if used[i] {
                        continue;
                    }
                    let q = if at_end { subs[i].end() } else { subs[i].start };
                    let d = q.distance(p);
                    if d <= tol && best.is_none_or(|(_, bd)| d < bd) {
                        best = Some(((i, at_end), d));
                    }
                }
            }
        }
        best.map(|(e, _)| e)
    }
}

/// Ergebnis von [`join_paths`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JoinResult {
    /// Ein Teilpfad je zusammenhängendem Linienzug (geschlossene unverändert).
    pub subpaths: Vec<SubPath>,
    /// Anzahl der Verbindungen zwischen Stücken.
    pub joined: usize,
    /// Anzahl der dabei geschlossenen Linienzüge.
    pub closed: usize,
}

/// Verbindet offene Teilpfade, deren Enden höchstens `tol` voneinander entfernt sind (Richtung wird bei Bedarf
/// umgekehrt), und schließt Linienzüge, deren Ende wieder beim Anfang liegt. Lücken verschwinden, weil das
/// angehängte Stück am Kettenende beginnt (Abweichung höchstens `tol`).
pub fn join_paths(subpaths: Vec<SubPath>, tol: f64) -> JoinResult {
    let tol = tol.max(1e-6);
    let (closed, open): (Vec<SubPath>, Vec<SubPath>) =
        subpaths.into_iter().filter(|s| !s.segments.is_empty()).partition(|s| s.closed);
    let mut ends = Ends { cell: tol, map: HashMap::new() };
    for (i, s) in open.iter().enumerate() {
        ends.insert(s.start, (i, false));
        ends.insert(s.end(), (i, true));
    }
    let mut used = vec![false; open.len()];
    let mut out = JoinResult { subpaths: closed, ..Default::default() };
    for i in 0..open.len() {
        if used[i] {
            continue;
        }
        used[i] = true;
        let mut chain = open[i].clone();
        let is_loop = |c: &SubPath| c.segments.len() >= 2 && c.end().distance(c.start) <= tol;
        // erst am Ende anhängen, dann am Anfang (über die umgekehrte Kette)
        for _ in 0..2 {
            while !is_loop(&chain)
                && let Some((j, at_end)) = ends.nearest(chain.end(), tol, &open, &used)
            {
                used[j] = true;
                let next = if at_end { open[j].reversed() } else { open[j].clone() };
                chain.segments.extend(next.segments);
                out.joined += 1;
            }
            chain = chain.reversed();
        }
        if is_loop(&chain) {
            // wie beim Zeichnen: eine Schlusslinie zum Anfang entfällt, Kurven enden genau am Anfang
            let last = chain.segments.len() - 1;
            match chain.segments[last] {
                Segment::Line(_) => {
                    chain.segments.pop();
                }
                s => chain.segments[last] = with_end(s, chain.start),
            }
            chain.closed = true;
            out.closed += 1;
        }
        out.subpaths.push(chain);
    }
    out
}

/// Schlüssel eines Segments unabhängig von der Laufrichtung, gerundet auf `q` mm.
fn segment_key(start: Point, seg: &Segment, q: f64) -> Vec<i64> {
    let r = |p: Point| [(p.x / q).round() as i64, (p.y / q).round() as i64];
    let pts: Vec<Point> = match *seg {
        Segment::Line(p) => vec![start, p],
        Segment::Quad(c, p) => vec![start, c, p],
        Segment::Cubic(a, b, p) => vec![start, a, b, p],
    };
    let fwd: Vec<i64> = pts.iter().flat_map(|&p| r(p)).collect();
    let rev: Vec<i64> = pts.iter().rev().flat_map(|&p| r(p)).collect();
    fwd.min(rev)
}

/// Entfernt Segmente, die bereits vorkamen (auch in umgekehrter Richtung oder in einem früheren Pfad). Pfade werden
/// an entfernten Stellen aufgetrennt. Liefert die bereinigten Pfade (in derselben Reihenfolge, ggf. leer) und die
/// Anzahl entfernter Segmente.
pub fn remove_duplicate_segments(paths: &[VectorPath], tol: f64) -> (Vec<VectorPath>, usize) {
    let q = tol.max(1e-6);
    let mut seen: HashSet<Vec<i64>> = HashSet::new();
    let mut removed = 0;
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        let mut result = VectorPath::default();
        for sp in &path.subpaths {
            // geschlossene Teilpfade mit ausdrücklicher Schlusslinie prüfen
            let mut segs = sp.segments.clone();
            if sp.closed && sp.end().distance(sp.start) > 1e-9 {
                segs.push(Segment::Line(sp.start));
            }
            let mut pieces: Vec<SubPath> = Vec::new();
            let mut cur: Option<SubPath> = None;
            let mut from = sp.start;
            let mut all_kept = true;
            for seg in segs {
                let key = segment_key(from, &seg, q);
                if seen.insert(key) {
                    cur.get_or_insert_with(|| SubPath { start: from, segments: Vec::new(), closed: false })
                        .segments
                        .push(seg);
                } else {
                    removed += 1;
                    all_kept = false;
                    pieces.extend(cur.take());
                }
                from = seg.end();
            }
            pieces.extend(cur.take());
            if all_kept {
                result.subpaths.push(sp.clone());
                continue;
            }
            if sp.closed && pieces.len() >= 2 {
                // Ring an einer Lücke aufgetrennt: letztes und erstes Stück hängen zusammen
                let last = pieces.pop().unwrap_or_default();
                if last.end().distance(pieces[0].start) <= 1e-9 {
                    let first = std::mem::take(&mut pieces[0]);
                    let mut merged = last;
                    merged.segments.extend(first.segments);
                    pieces[0] = merged;
                } else {
                    pieces.push(last);
                }
            }
            result.subpaths.extend(pieces);
        }
        out.push(result);
    }
    (out, removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Affine;

    fn line(a: (f64, f64), b: (f64, f64)) -> SubPath {
        SubPath { start: Point::new(a.0, a.1), segments: vec![Segment::Line(Point::new(b.0, b.1))], closed: false }
    }

    #[test]
    fn reverse_keeps_curves() {
        let s = SubPath {
            start: Point::new(0.0, 0.0),
            segments: vec![
                Segment::Line(Point::new(10.0, 0.0)),
                Segment::Cubic(Point::new(12.0, 0.0), Point::new(12.0, 5.0), Point::new(10.0, 5.0)),
            ],
            closed: false,
        };
        let r = s.reversed();
        assert_eq!(r.start, Point::new(10.0, 5.0));
        assert_eq!(r.segments[0], Segment::Cubic(Point::new(12.0, 5.0), Point::new(12.0, 0.0), Point::new(10.0, 0.0)));
        assert_eq!(r.end(), Point::new(0.0, 0.0));
        assert_eq!(r.reversed(), s);
    }

    #[test]
    fn joins_pieces_into_closed_triangle() {
        // drei Stücke mit kleinen Lücken, eines falsch herum
        let pieces =
            vec![line((0.0, 0.0), (10.0, 0.0)), line((5.0, 8.66), (10.05, 0.02)), line((5.03, 8.7), (0.04, 0.0))];
        let r = join_paths(pieces.clone(), 0.1);
        assert_eq!((r.subpaths.len(), r.joined, r.closed), (1, 2, 1));
        let s = &r.subpaths[0];
        // zwei Linien plus Schlusslinie
        assert!(s.closed);
        assert_eq!(s.segments.len(), 2, "{s:?}");
        let poly = &VectorPath { subpaths: vec![s.clone()] }.flatten(&Affine::IDENTITY, 0.01)[0];
        assert!((poly.length() - 30.0).abs() < 0.2, "{}", poly.length());
        // zu große Lücken bleiben offen
        let r = join_paths(pieces, 0.01);
        assert_eq!((r.subpaths.len(), r.joined, r.closed), (3, 0, 0));
        // geschlossene Teilpfade bleiben unverändert
        let sq = VectorPath::rect(0.0, 0.0, 5.0, 5.0, 0.0).subpaths;
        assert_eq!(join_paths(sq.clone(), 0.1).subpaths, sq);
    }

    #[test]
    fn joins_long_chains_in_any_order() {
        // 100 Stücke eines Kreises, durcheinander und teils umgekehrt
        let circle = VectorPath::ellipse(Point::new(0.0, 0.0), 20.0, 20.0).flatten(&Affine::IDENTITY, 0.01);
        let pts = &circle[0].points;
        let n = pts.len();
        let mut pieces: Vec<SubPath> = (0..n)
            .map(|i| {
                let (a, b) = (pts[i], pts[(i + 1) % n]);
                let s = line((a.x, a.y), (b.x, b.y));
                if i % 3 == 0 { s.reversed() } else { s }
            })
            .collect();
        pieces.reverse();
        pieces.swap(0, n / 2);
        let r = join_paths(pieces, 0.001);
        assert_eq!((r.subpaths.len(), r.closed), (1, 1));
        assert_eq!(r.subpaths[0].segments.len(), n - 1);
    }

    #[test]
    fn removes_duplicate_lines() {
        let sq = VectorPath::rect(0.0, 0.0, 10.0, 10.0, 0.0);
        let edge = VectorPath { subpaths: vec![line((10.0, 0.0), (0.0, 0.0))] };
        let other = VectorPath { subpaths: vec![line((20.0, 0.0), (30.0, 0.0))] };
        let (out, removed) = remove_duplicate_segments(&[sq.clone(), sq.clone(), edge, other.clone()], 0.01);
        assert_eq!(removed, 5);
        assert_eq!(out[0], sq);
        assert!(out[1].is_empty() && out[2].is_empty());
        assert_eq!(out[3], other);
        // doppelte Kante in einem Ring: Rest bleibt ein zusammenhängender offener Linienzug
        let (out, removed) =
            remove_duplicate_segments(&[VectorPath { subpaths: vec![line((5.0, 0.0), (0.0, 0.0))] }, sq], 0.01);
        assert_eq!(removed, 0, "Teilstück ist kein Duplikat der ganzen Kante");
        assert_eq!(out[1].subpaths.len(), 1);
    }
}
