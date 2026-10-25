//! Werkzeugwege im Dokument-Koordinatensystem: Linien, Füllungen und Rasterbilder.

use vl_core::{CutSettings, DitherMode, Point, Polyline};
use vl_raster::RasterResult;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    /// Eilgang, Laser aus.
    Rapid(Point),
    /// Vorschubbewegung; `power` ist der Anteil an der Maximalleistung (0 = Laser aus).
    Move { to: Point, power: f32 },
}

impl Op {
    pub fn target(&self) -> Point {
        match *self {
            Op::Rapid(p) | Op::Move { to: p, .. } => p,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Line,
    Fill,
    Raster,
}

/// Zusammenhängender Abschnitt mit einheitlichem Vorschub, z. B. ein Durchgang einer Ebene.
#[derive(Clone, Debug)]
pub struct Block {
    pub layer: usize,
    pub kind: BlockKind,
    pub pass: u32,
    pub feed: f64,
    pub air: bool,
    pub ops: Vec<Op>,
    /// Index in `Toolpath::rasters` (nur bei Rasterblöcken).
    pub raster: Option<usize>,
    /// Z-Höhe für diesen Block (nur mit Z-Achse ausgegeben).
    pub z: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct RasterPreview {
    pub layer: usize,
    pub result: RasterResult,
}

#[derive(Clone, Debug, Default)]
pub struct Toolpath {
    pub blocks: Vec<Block>,
    pub rasters: Vec<RasterPreview>,
}

impl Toolpath {
    pub fn is_empty(&self) -> bool {
        self.blocks.iter().all(|b| b.ops.is_empty())
    }

    pub fn op_count(&self) -> usize {
        self.blocks.iter().map(|b| b.ops.len()).sum()
    }

    pub fn last_point(&self) -> Option<Point> {
        self.blocks.iter().rev().find_map(|b| b.ops.last().map(|o| o.target()))
    }
}

/// Konturen abfahren (Polylinien bereits in Reihenfolge und Startpunkt optimiert).
pub fn line_ops(polys: &[Polyline], power: f32) -> Vec<Op> {
    let mut ops = Vec::new();
    for poly in polys {
        let Some(&first) = poly.points.first() else { continue };
        ops.push(Op::Rapid(first));
        for &p in &poly.points[1..] {
            ops.push(Op::Move { to: p, power });
        }
        if poly.closed {
            ops.push(Op::Move { to: first, power });
        }
    }
    ops
}

/// Linienmodus: wie `line_ops`, zusätzlich mit Überschnitt (geschlossene Konturen laufen `overcut` mm über den
/// Startpunkt hinaus, höchstens eine Runde) und Perforation (abwechselnd `cut` mm schneiden und `gap` mm im Vorschub
/// mit ausgeschaltetem Laser; jede Kontur beginnt mit einem Schnitt).
pub fn cut_ops(polys: &[Polyline], power: f32, overcut: f64, perforation: Option<(f64, f64)>) -> Vec<Op> {
    cut_ops_with_leads(polys, power, overcut, perforation, None)
}

/// Ein- und Auslauf an geschlossenen Konturen: Länge in mm und Winkel zur Kontur in Grad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Leads {
    pub lead_in: f64,
    pub lead_out: f64,
    pub angle_deg: f64,
}

/// Einheitsnormale der Strecke `a`→`b`, die zum Abfall zeigt: bei Außenkonturen nach außen, bei Löchern nach
/// innen. Geprüft wird neben der Streckenmitte, damit Ecken nicht stören.
fn waste_normal(ring: &Polyline, a: Point, b: Point, outside: bool) -> Point {
    let d = b - a;
    let n = Point::new(d.y, -d.x) * (1.0 / d.length().max(1e-12));
    let probe = a.lerp(b, 0.5) + n * 1e-3;
    let inside = vl_core::path::point_in_polylines(probe, std::slice::from_ref(ring));
    if inside == outside { -n } else { n }
}

/// Wie [`cut_ops`], dazu wahlweise Ein- und Auslauf an geschlossenen Konturen.
pub fn cut_ops_with_leads(
    polys: &[Polyline],
    power: f32,
    overcut: f64,
    perforation: Option<(f64, f64)>,
    leads: Option<Leads>,
) -> Vec<Op> {
    let depths = if leads.is_some() { vl_core::ops::nesting_depths(polys) } else { Vec::new() };
    let mut ops = Vec::new();
    for (index, poly) in polys.iter().enumerate() {
        let Some(&first) = poly.points.first() else { continue };
        let mut pts = poly.points.clone();
        if poly.closed {
            pts.push(first);
            if overcut > 0.0 {
                let ring = pts.clone();
                let mut left = overcut.min(poly.length());
                for w in ring.windows(2) {
                    let len = w[0].distance(w[1]);
                    if len >= left {
                        if len > 1e-9 {
                            pts.push(w[0] + (w[1] - w[0]) * (left / len));
                        }
                        break;
                    }
                    pts.push(w[1]);
                    left -= len;
                }
            }
            if let Some(l) = leads {
                let outside = depths[index].is_multiple_of(2);
                let (s, c) = l.angle_deg.to_radians().sin_cos();
                let segment = |w: &[Point]| (w[0].distance(w[1]) > 1e-9).then_some((w[0], w[1]));
                if l.lead_out > 0.0
                    && let Some((a, b)) = pts.windows(2).rev().find_map(segment)
                {
                    let dir = (b - a) * (1.0 / a.distance(b));
                    let n = waste_normal(poly, a, b, outside);
                    let end = pts[pts.len() - 1];
                    pts.push(end + dir * (l.lead_out * c) + n * (l.lead_out * s));
                }
                if l.lead_in > 0.0
                    && let Some((a, b)) = pts.windows(2).find_map(segment)
                {
                    let dir = (b - a) * (1.0 / a.distance(b));
                    let n = waste_normal(poly, a, b, outside);
                    pts.insert(0, first - dir * (l.lead_in * c) + n * (l.lead_in * s));
                }
            }
        }
        ops.push(Op::Rapid(pts[0]));
        let Some((cut, gap)) = perforation else {
            ops.extend(pts[1..].iter().map(|&p| Op::Move { to: p, power }));
            continue;
        };
        let (mut on, mut left) = (true, cut);
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let len = a.distance(b);
            let mut t = 0.0;
            while len - t > 1e-9 {
                let step = left.min(len - t);
                t += step;
                left -= step;
                ops.push(Op::Move { to: a + (b - a) * (t / len), power: if on { power } else { 0.0 } });
                if left <= 1e-9 {
                    on = !on;
                    left = if on { cut } else { gap };
                }
            }
        }
    }
    ops
}

#[derive(Clone, Copy)]
struct Edge {
    y0: f64,
    y1: f64,
    x0: f64,
    slope: f64,
}

/// Abstand, ab dem zwischen Füllzeilen ein Eilgang statt einer Leerfahrt im Vorschub benutzt wird.
const LINK_RAPID_DISTANCE: f64 = 10.0;

/// Zeilenweises Füllen geschlossener Polylinien (Gerade-Ungerade-Regel). `scan_shift` verschiebt jede Zeile gegen
/// ihre Fahrtrichtung (Scan-Versatz, siehe `MachineProfile::scan_shift`).
pub fn hatch_ops(
    polys: &[Polyline],
    interval: f64,
    angle_deg: f64,
    bidirectional: bool,
    overscan: f64,
    scan_shift: f64,
    power: f32,
) -> Vec<Op> {
    hatch_ops_on_grid(polys, interval, angle_deg, bidirectional, overscan, scan_shift, power, None)
}

/// Wie [`hatch_ops`]; mit `grid` liegen die Zeilen auf dem festen Raster `grid + (k + ½)·interval` (im gedrehten
/// System), damit getrennt gefüllte Formen dieselben Zeilen haben.
#[allow(clippy::too_many_arguments)]
pub fn hatch_ops_on_grid(
    polys: &[Polyline],
    interval: f64,
    angle_deg: f64,
    bidirectional: bool,
    overscan: f64,
    scan_shift: f64,
    power: f32,
    grid: Option<f64>,
) -> Vec<Op> {
    let interval = interval.max(0.01);
    let angle = angle_deg.to_radians();
    let mut edges = Vec::new();
    let (mut ymin, mut ymax) = (f64::INFINITY, f64::NEG_INFINITY);
    for poly in polys.iter().filter(|p| p.closed && p.points.len() >= 3) {
        let pts: Vec<Point> = poly.points.iter().map(|p| p.rotated(-angle)).collect();
        let n = pts.len();
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            ymin = ymin.min(a.y);
            ymax = ymax.max(a.y);
            if (a.y - b.y).abs() < 1e-12 {
                continue;
            }
            let (lo, hi) = if a.y < b.y { (a, b) } else { (b, a) };
            edges.push(Edge { y0: lo.y, y1: hi.y, x0: lo.x, slope: (hi.x - lo.x) / (hi.y - lo.y) });
        }
    }
    if edges.is_empty() {
        return Vec::new();
    }
    edges.sort_by(|a, b| a.y0.total_cmp(&b.y0));

    let mut ops = Vec::new();
    let mut active: Vec<Edge> = Vec::new();
    let mut next_edge = 0;
    let mut xs: Vec<f64> = Vec::new();
    let mut row_count = 0usize;
    let mut cursor: Option<Point> = None;
    let rot = |p: Point| p.rotated(angle);
    let base = match grid {
        Some(g) => g + ((ymin - g) / interval).floor() * interval,
        None => ymin,
    };
    let rows = ((ymax - base) / interval).floor() as usize + 1;

    for row in 0..rows {
        let y = base + (row as f64 + 0.5) * interval;
        if y >= ymax {
            break;
        }
        while next_edge < edges.len() && edges[next_edge].y0 <= y {
            active.push(edges[next_edge]);
            next_edge += 1;
        }
        active.retain(|e| e.y1 > y);
        xs.clear();
        xs.extend(active.iter().filter(|e| e.y0 <= y).map(|e| e.x0 + (y - e.y0) * e.slope));
        if xs.len() < 2 {
            continue;
        }
        xs.sort_by(f64::total_cmp);
        let mut segs: Vec<(f64, f64)> =
            xs.as_chunks::<2>().0.iter().map(|c| (c[0], c[1])).filter(|s| s.1 - s.0 > 1e-6).collect();
        if segs.is_empty() {
            continue;
        }
        let forward = !bidirectional || row_count.is_multiple_of(2);
        if !forward {
            segs.reverse();
            for s in &mut segs {
                *s = (s.1, s.0);
            }
        }
        let dir = if forward { 1.0 } else { -1.0 };
        let at = |x: f64| rot(Point::new(x - dir * scan_shift, y));
        let start = at(segs[0].0 - dir * overscan);
        match cursor {
            Some(c) if c.distance(start) <= LINK_RAPID_DISTANCE => ops.push(Op::Move { to: start, power: 0.0 }),
            _ => ops.push(Op::Rapid(start)),
        }
        for &(a, b) in &segs {
            ops.push(Op::Move { to: at(a), power: 0.0 });
            ops.push(Op::Move { to: at(b), power });
        }
        let end = at(segs.last().unwrap().1 + dir * overscan);
        if overscan > 0.0 {
            ops.push(Op::Move { to: end, power: 0.0 });
        }
        cursor = Some(end);
        row_count += 1;
    }
    ops
}

/// Stempel-Rampe: Brennstrecken werden unterteilt, die Leistung steigt mit dem Abstand zum nächsten Rand von `min`
/// bis `max` (Anteile 0…1) über die Länge `ramp`. Gleiche Stufen (0,5 %) werden zusammengefasst.
pub fn apply_ramp(ops: Vec<Op>, edges_of: &[Polyline], ramp: f64, min: f32, max: f32) -> Vec<Op> {
    if ramp <= 0.0 || edges_of.is_empty() {
        return ops;
    }
    let grid = EdgeGrid::new(edges_of, ramp);
    let step = (ramp / 16.0).clamp(0.02, 1.0);
    let level = |p: Point| {
        let t = (grid.distance(p, ramp) / ramp) as f32;
        let v = min + (max - min) * t.clamp(0.0, 1.0);
        (v * 200.0).round() / 200.0
    };
    let mut out = Vec::with_capacity(ops.len());
    let mut cur = Point::ZERO;
    for op in ops {
        match op {
            Op::Move { to, power } if power > 0.0 => {
                let len = cur.distance(to);
                let n = (len / step).ceil().max(1.0) as usize;
                let mut pending: Option<(Point, f32)> = None;
                for k in 0..n {
                    let (a, b) = (cur.lerp(to, k as f64 / n as f64), cur.lerp(to, (k + 1) as f64 / n as f64));
                    let p = level(a.lerp(b, 0.5));
                    pending = match pending {
                        Some((_, q)) if q == p => Some((b, p)),
                        Some((end, q)) => {
                            out.push(Op::Move { to: end, power: q });
                            Some((b, p))
                        }
                        None => Some((b, p)),
                    };
                }
                if let Some((end, q)) = pending {
                    out.push(Op::Move { to: end, power: q });
                }
            }
            other => out.push(other),
        }
        cur = op.target();
    }
    out
}

/// Kanten in einem Raster mit Zellgröße `cell` für schnelle Abstandsabfragen bis `cell`.
struct EdgeGrid {
    cell: f64,
    map: std::collections::HashMap<(i64, i64), Vec<(Point, Point)>>,
}

impl EdgeGrid {
    fn new(polys: &[Polyline], cell: f64) -> Self {
        let mut map: std::collections::HashMap<(i64, i64), Vec<(Point, Point)>> = Default::default();
        let key = |p: Point| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
        for poly in polys {
            for (a, b) in poly.edges() {
                // Stützpunkte im Abstand einer halben Zelle; Abfragen schauen eine Zelle weiter
                let n = (a.distance(b) / (cell / 2.0)).ceil().max(1.0) as usize;
                let mut last = None;
                for k in 0..=n {
                    let c = key(a.lerp(b, k as f64 / n as f64));
                    if last != Some(c) {
                        let list = map.entry(c).or_default();
                        if list.last() != Some(&(a, b)) {
                            list.push((a, b));
                        }
                        last = Some(c);
                    }
                }
            }
        }
        Self { cell, map }
    }

    /// Abstand zur nächsten Kante, höchstens `limit`.
    fn distance(&self, p: Point, limit: f64) -> f64 {
        let r = ((limit / self.cell).ceil() as i64) + 1;
        let (cx, cy) = ((p.x / self.cell).floor() as i64, (p.y / self.cell).floor() as i64);
        let mut best = limit;
        for x in cx - r..=cx + r {
            for y in cy - r..=cy + r {
                for &(a, b) in self.map.get(&(x, y)).into_iter().flatten() {
                    best = best.min(vl_core::path::distance_to_segment(p, a, b));
                }
            }
        }
        best
    }
}

/// Leistungswert für eine Rasterzelle.
fn cell_power(v: f32, settings: &CutSettings) -> f32 {
    if v <= 0.002 {
        return 0.0;
    }
    let pmax = (settings.power_max / 100.0) as f32;
    let pmin = (settings.power_min / 100.0) as f32;
    let p = if settings.dither == DitherMode::Grayscale { pmin + (pmax - pmin) * v } else { pmax };
    // Auf Promille runden, damit benachbarte Zellen zu einer Bewegung zusammengefasst werden.
    (p * 1000.0).round() / 1000.0
}

/// Weiße Lücken in einer Bildzeile, die (neben Aus- und Anlauf) mindestens so lang sind, werden mit
/// `fast_whitespace` im Eilgang überfahren; kürzere lohnen wegen der Beschleunigung nicht.
pub const FAST_WHITESPACE_MIN_GAP: f64 = 5.0;

/// Rasterbild zeilenweise gravieren; leere Zeilen und leere Ränder werden übersprungen. `scan_shift` wie bei
/// [`hatch_ops`]. Mit `fast_whitespace` werden lange weiße Lücken innerhalb einer Zeile verlassen (Auslauf um den
/// Overscan), im Eilgang überfahren und mit Anlauf wieder betreten.
pub fn raster_ops(r: &RasterResult, settings: &CutSettings, scan_shift: f64, fast_whitespace: bool) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut emitted = 0usize;
    let overscan = settings.overscan.max(0.0);
    let x_at = |c: usize| r.origin.x + c as f64 * r.dx;
    let mut row_powers: Vec<f32> = Vec::with_capacity(r.cols);
    for row in 0..r.rows {
        row_powers.clear();
        row_powers.extend((0..r.cols).map(|c| cell_power(r.get(c, row), settings)));
        let Some(first) = row_powers.iter().position(|&p| p > 0.0) else { continue };
        let last = row_powers.iter().rposition(|&p| p > 0.0).unwrap();
        let y = r.origin.y + (row as f64 + 0.5) * r.dy;

        let mut runs: Vec<(usize, usize, f32)> = Vec::new();
        let mut c = first;
        while c <= last {
            let p = row_powers[c];
            let mut e = c;
            while e < last && row_powers[e + 1] == p {
                e += 1;
            }
            runs.push((c, e, p));
            c = e + 1;
        }

        // Gebrannte Strecken (von, bis, Leistung); Punktbreiten-Korrektur nur an Übergängen zu „aus“
        let half_dot = (settings.dot_width.max(0.0) / 2.0).min(r.dx);
        let mut segs: Vec<(f64, f64, f32)> = Vec::with_capacity(runs.len());
        for (k, &(s, e, p)) in runs.iter().enumerate() {
            if p <= 0.0 {
                continue;
            }
            let (mut a, mut b) = (x_at(s), x_at(e + 1));
            if half_dot > 0.0 {
                if k == 0 || runs[k - 1].2 <= 0.0 {
                    a += half_dot;
                }
                if k + 1 == runs.len() || runs[k + 1].2 <= 0.0 {
                    b -= half_dot;
                }
                // Einzelne Punkte bleiben als kurzer Punkt erhalten
                let min = (r.dx * 0.1).max(0.005);
                if b - a < min {
                    let c = (x_at(s) + x_at(e + 1)) / 2.0;
                    (a, b) = (c - min / 2.0, c + min / 2.0);
                }
            }
            segs.push((a, b, p));
        }

        let forward = !settings.bidirectional || emitted.is_multiple_of(2);
        let dir = if forward { 1.0 } else { -1.0 };
        let at = |x: f64| Point::new(x - dir * scan_shift, y);
        let (start_x, edge_x) =
            if forward { (x_at(first) - overscan, x_at(first)) } else { (x_at(last + 1) + overscan, x_at(last + 1)) };
        let start = at(start_x);
        if emitted == 0 {
            ops.push(Op::Rapid(start))
        } else {
            ops.push(Op::Move { to: start, power: 0.0 })
        }
        if overscan > 0.0 {
            ops.push(Op::Move { to: at(edge_x), power: 0.0 });
        }
        let mut cursor = edge_x;
        // Lücke von `from` nach `to` überbrücken (beide Richtungen)
        let gap = |ops: &mut Vec<Op>, from: f64, to: f64| {
            let len = (to - from).abs();
            if fast_whitespace && len > 2.0 * overscan + FAST_WHITESPACE_MIN_GAP {
                let s = (to - from).signum();
                if overscan > 0.0 {
                    ops.push(Op::Move { to: at(from + s * overscan), power: 0.0 });
                }
                ops.push(Op::Rapid(at(to - s * overscan)));
                if overscan > 0.0 {
                    ops.push(Op::Move { to: at(to), power: 0.0 });
                }
            } else {
                ops.push(Op::Move { to: at(to), power: 0.0 });
            }
        };
        if forward {
            for &(a, b, p) in &segs {
                if a > cursor + 1e-9 {
                    gap(&mut ops, cursor, a);
                }
                ops.push(Op::Move { to: at(b), power: p });
                cursor = b;
            }
            if overscan > 0.0 {
                ops.push(Op::Move { to: at(x_at(last + 1) + overscan), power: 0.0 });
            }
        } else {
            for &(a, b, p) in segs.iter().rev() {
                if b < cursor - 1e-9 {
                    gap(&mut ops, cursor, b);
                }
                ops.push(Op::Move { to: at(a), power: p });
                cursor = a;
            }
            if overscan > 0.0 {
                ops.push(Op::Move { to: at(x_at(first) - overscan), power: 0.0 });
            }
        }
        emitted += 1;
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::{Affine, VectorPath};

    fn burned_length(ops: &[Op]) -> f64 {
        let mut cur = Point::ZERO;
        let mut sum = 0.0;
        for op in ops {
            if let Op::Move { to, power } = *op
                && power > 0.0
            {
                sum += cur.distance(to);
            }
            cur = op.target();
        }
        sum
    }

    #[test]
    fn leads_start_and_end_in_the_waste() {
        // Teil 0…20 mit Loch 5…15
        let mut p = VectorPath::rect(0.0, 0.0, 20.0, 20.0, 0.0);
        p.append(VectorPath::rect(5.0, 5.0, 10.0, 10.0, 0.0));
        let polys = p.flatten(&Affine::IDENTITY, 0.01);
        let leads = Leads { lead_in: 2.0, lead_out: 1.5, angle_deg: 90.0 };
        let ops = cut_ops_with_leads(&polys, 1.0, 0.0, None, Some(leads));
        let plain = cut_ops(&polys, 1.0, 0.0, None);
        assert!((burned_length(&ops) - burned_length(&plain) - 2.0 * 3.5).abs() < 1e-6);
        let inside = |p: Point, ring: &Polyline| vl_core::path::point_in_polylines(p, std::slice::from_ref(ring));
        let rapids: Vec<Point> = ops
            .iter()
            .filter_map(|o| match o {
                Op::Rapid(p) => Some(*p),
                _ => None,
            })
            .collect();
        assert_eq!(rapids.len(), 2);
        for (ring, start) in polys.iter().zip(&rapids) {
            let first = ring.points[0];
            assert!((start.distance(first) - 2.0).abs() < 1e-9);
            let hole = ring.bounds().width() < 15.0;
            // Außenkontur: Einlauf außen; Loch: Einlauf im Loch
            assert_eq!(inside(*start, ring), hole, "{start:?}");
        }
        // Auslauf endet ebenfalls im Abfall
        let end = ops.last().unwrap().target();
        let last_ring = &polys[1];
        assert!(inside(end, last_ring));
        // ohne Ein-/Auslauf unverändert
        assert_eq!(cut_ops_with_leads(&polys, 1.0, 0.0, None, None), plain);
    }

    #[test]
    fn overcut_and_perforation() {
        let square = Polyline::new(
            vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)],
            true,
        );
        let sq = std::slice::from_ref(&square);
        // Überschnitt: nach der Runde 3 mm weiter auf der ersten Kante
        let ops = cut_ops(sq, 0.5, 3.0, None);
        assert_eq!(ops.last().unwrap().target(), Point::new(3.0, 0.0));
        assert!((burned_length(&ops) - 43.0).abs() < 1e-9);
        // höchstens eine weitere Runde; ohne Optionen wie line_ops
        assert!((burned_length(&cut_ops(sq, 0.5, 100.0, None)) - 80.0).abs() < 1e-9);
        assert_eq!(cut_ops(sq, 0.5, 0.0, None), line_ops(sq, 0.5));
        // offene Linie 10 mm, 2 mm schneiden / 1 mm Lücke: 2 + 2 + 2 + 1 mm
        let line = Polyline::new(vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0)], false);
        let ops = cut_ops(std::slice::from_ref(&line), 0.5, 0.0, Some((2.0, 1.0)));
        assert!((burned_length(&ops) - 7.0).abs() < 1e-9);
        assert_eq!(ops.len(), 1 + 7);
        assert_eq!(ops.last().unwrap().target(), Point::new(10.0, 0.0));
        // Perforation läuft über Ecken weiter: 40 mm Umfang = 13 volle Perioden + 1 mm Schnitt
        assert!((burned_length(&cut_ops(sq, 0.5, 0.0, Some((2.0, 1.0)))) - 27.0).abs() < 1e-9);
    }

    #[test]
    fn hatch_square_covers_area() {
        let polys = VectorPath::rect(0.0, 0.0, 10.0, 10.0, 0.0).flatten(&Affine::IDENTITY, 0.01);
        let ops = hatch_ops(&polys, 0.5, 0.0, true, 1.0, 0.0, 0.5);
        // 20 Zeilen à 10 mm
        assert!((burned_length(&ops) - 200.0).abs() < 1e-6);
        assert!(matches!(ops[0], Op::Rapid(_)));
    }

    #[test]
    fn hatch_respects_holes_and_angle() {
        let mut ring = VectorPath::rect(0.0, 0.0, 10.0, 10.0, 0.0);
        ring.append(VectorPath::rect(2.0, 2.0, 6.0, 6.0, 0.0));
        let polys = ring.flatten(&Affine::IDENTITY, 0.01);
        let area = 100.0 - 36.0;
        for angle in [0.0, 30.0, 90.0] {
            let ops = hatch_ops(&polys, 0.05, angle, true, 0.0, 0.0, 1.0);
            let covered = burned_length(&ops) * 0.05;
            assert!((covered - area).abs() / area < 0.03, "Winkel {angle}: {covered}");
        }
    }

    #[test]
    fn raster_skips_blank_rows_and_merges_runs() {
        let r = RasterResult {
            origin: Point::ZERO,
            dx: 1.0,
            dy: 1.0,
            cols: 4,
            rows: 3,
            intensity: vec![0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0],
        };
        let settings = CutSettings { overscan: 0.0, dither: DitherMode::Threshold, ..CutSettings::default() };
        let ops = raster_ops(&r, &settings, 0.0, false);
        // Zeile 1: Start, ein Lauf über zwei Zellen
        assert_eq!(ops[0], Op::Rapid(Point::new(1.0, 1.5)));
        assert_eq!(ops[1], Op::Move { to: Point::new(3.0, 1.5), power: 0.3 });
        // Zeile 2 rückwärts: Start rechts, dann Brennen, Pause, Brennen
        assert_eq!(ops[2].target(), Point::new(4.0, 2.5));
        assert!((burned_length(&ops) - 4.0).abs() < 1e-9);

        // Scan-Versatz: vorwärts nach links, rückwärts nach rechts verschoben
        let settings = CutSettings { bidirectional: true, ..settings };
        let shifted = raster_ops(&r, &settings, 0.1, false);
        assert_eq!(shifted[0], Op::Rapid(Point::new(0.9, 1.5)));
        assert_eq!(shifted[2].target(), Point::new(4.1, 2.5));
    }

    #[test]
    fn ramp_raises_power_away_from_edges() {
        // Rahmen 0…20 × 0…10 mit Loch 8…12 × 4…6
        let mut p = VectorPath::rect(0.0, 0.0, 20.0, 10.0, 0.0);
        p.append(VectorPath::rect(8.0, 4.0, 4.0, 2.0, 0.0));
        let polys = p.flatten(&Affine::IDENTITY, 0.01);
        let ops = hatch_ops(&polys, 0.5, 0.0, false, 0.0, 0.0, 1.0);
        let burned = burned_length(&ops);
        let power_at = |ops: &[Op], x: f64, y: f64| -> f32 {
            let mut cur = Point::ZERO;
            for op in ops {
                if let Op::Move { to, power } = *op
                    && power > 0.0
                    && (cur.y - y).abs() < 0.3
                    && (to.y - y).abs() < 0.3
                    && cur.x.min(to.x) <= x
                    && x <= cur.x.max(to.x)
                {
                    return power;
                }
                cur = op.target();
            }
            panic!("keine Brennstrecke bei {x}/{y}");
        };
        let ramped = apply_ramp(ops.clone(), &polys, 2.0, 0.2, 1.0);
        assert!((burned_length(&ramped) - burned).abs() < 1e-6);
        // am Rand nahe Mindestleistung, innen voll, am Loch wieder schwächer
        assert!(power_at(&ramped, 0.3, 2.25) < 0.35);
        assert!((power_at(&ramped, 4.0, 2.25) - 1.0).abs() < 1e-6);
        assert!(power_at(&ramped, 7.6, 4.75) < 0.4);
        // nur Außenkante: am Loch volle Leistung
        let outer = vl_core::ops::outermost(&polys);
        assert_eq!(outer.len(), 1);
        let ramped = apply_ramp(ops, &outer, 2.0, 0.2, 1.0);
        assert!(power_at(&ramped, 0.3, 2.25) < 0.35);
        assert!((power_at(&ramped, 7.6, 4.75) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn dot_width_shortens_burned_runs() {
        // eine Zeile: an an aus an aus aus an
        let r = RasterResult {
            origin: Point::ZERO,
            dx: 1.0,
            dy: 1.0,
            cols: 7,
            rows: 1,
            intensity: vec![1.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0],
        };
        let base = CutSettings { overscan: 0.0, dither: DitherMode::Threshold, ..CutSettings::default() };
        assert!((burned_length(&raster_ops(&r, &base, 0.0, false)) - 4.0).abs() < 1e-9);
        let corrected = CutSettings { dot_width: 0.4, ..base.clone() };
        let ops = raster_ops(&r, &corrected, 0.0, false);
        assert!((burned_length(&ops) - 2.8).abs() < 1e-9, "{ops:?}");
        // Brennstrecken liegen innerhalb der Pixel, um die halbe Punktbreite eingerückt
        let burns: Vec<(f64, f64)> = ops
            .windows(2)
            .filter_map(|w| match w[1] {
                Op::Move { to, power } if power > 0.0 => Some((w[0].target().x, to.x)),
                _ => None,
            })
            .collect();
        assert_eq!(burns.len(), 3);
        assert!((burns[0].0 - 0.2).abs() < 1e-9 && (burns[0].1 - 1.8).abs() < 1e-9, "{burns:?}");
        assert!((burns[2].0 - 6.2).abs() < 1e-9 && (burns[2].1 - 6.8).abs() < 1e-9, "{burns:?}");
        // Punktbreite größer als ein Pixel: kurzer Punkt in der Mitte bleibt
        let wide = CutSettings { dot_width: 3.0, ..base };
        let ops = raster_ops(&r, &wide, 0.0, false);
        assert!(burned_length(&ops) > 0.0);
    }

    #[test]
    fn scan_shift_moves_rows_against_travel() {
        let polys = VectorPath::rect(0.0, 0.0, 10.0, 2.0, 0.0).flatten(&Affine::IDENTITY, 0.01);
        let ops = hatch_ops(&polys, 1.0, 0.0, true, 0.0, 0.1, 1.0);
        let burn_ends: Vec<Point> = ops
            .iter()
            .filter_map(|o| match *o {
                Op::Move { to, power } if power > 0.0 => Some(to),
                _ => None,
            })
            .collect();
        // Zeile 1 vorwärts (0 → 10) endet bei 9,9; Zeile 2 rückwärts (10 → 0) endet bei 0,1
        assert_eq!(burn_ends.len(), 2);
        assert!((burn_ends[0].x - 9.9).abs() < 1e-9 && (burn_ends[1].x - 0.1).abs() < 1e-9);
        assert!((burned_length(&ops) - 20.0).abs() < 1e-9);
    }

    #[test]
    fn fast_whitespace_rapids_over_long_gaps() {
        // Zeile: 2 mm schwarz, 20 mm weiß, 2 mm schwarz (Zellen 0,5 mm)
        let cols = 48;
        let intensity: Vec<f32> = (0..cols).map(|c| if !(4..44).contains(&c) { 1.0 } else { 0.0 }).collect();
        let r = RasterResult { origin: Point::new(0.0, 0.0), dx: 0.5, dy: 0.5, cols, rows: 1, intensity };
        let settings = CutSettings { overscan: 2.0, bidirectional: false, ..CutSettings::default() };
        let slow = raster_ops(&r, &settings, 0.0, false);
        assert!(slow.iter().skip(1).all(|o| matches!(o, Op::Move { .. })), "ohne Option nur Vorschub");
        let fast = raster_ops(&r, &settings, 0.0, true);
        let rapids: Vec<Point> =
            fast.iter().skip(1).filter_map(|o| if let Op::Rapid(p) = o { Some(*p) } else { None }).collect();
        // Auslauf bis 4 mm, Eilgang bis 20 mm, Anlauf bis 22 mm
        assert_eq!(rapids, vec![Point::new(20.0, 0.25)]);
        assert!(fast.contains(&Op::Move { to: Point::new(4.0, 0.25), power: 0.0 }));
        assert!(fast.contains(&Op::Move { to: Point::new(22.0, 0.25), power: 0.0 }));
        // gebrannte Strecken bleiben gleich
        assert!((burned_length(&slow) - burned_length(&fast)).abs() < 1e-9);
        // kurze Lücken bleiben Vorschub
        let short = RasterResult {
            cols: 16,
            intensity: (0..16).map(|c| if !(4..12).contains(&c) { 1.0 } else { 0.0 }).collect(),
            ..r
        };
        assert!(!raster_ops(&short, &settings, 0.0, true).iter().skip(1).any(|o| matches!(o, Op::Rapid(_))));
    }
}
