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

#[derive(Clone, Copy)]
struct Edge {
    y0: f64,
    y1: f64,
    x0: f64,
    slope: f64,
}

/// Abstand, ab dem zwischen Füllzeilen ein Eilgang statt einer Leerfahrt im Vorschub benutzt wird.
const LINK_RAPID_DISTANCE: f64 = 10.0;

/// Zeilenweises Füllen geschlossener Polylinien (Gerade-Ungerade-Regel).
pub fn hatch_ops(
    polys: &[Polyline],
    interval: f64,
    angle_deg: f64,
    bidirectional: bool,
    overscan: f64,
    power: f32,
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
    let rows = ((ymax - ymin) / interval).floor() as usize + 1;

    for row in 0..rows {
        let y = ymin + (row as f64 + 0.5) * interval;
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
        let start = rot(Point::new(segs[0].0 - dir * overscan, y));
        match cursor {
            Some(c) if c.distance(start) <= LINK_RAPID_DISTANCE => ops.push(Op::Move { to: start, power: 0.0 }),
            _ => ops.push(Op::Rapid(start)),
        }
        for &(a, b) in &segs {
            ops.push(Op::Move { to: rot(Point::new(a, y)), power: 0.0 });
            ops.push(Op::Move { to: rot(Point::new(b, y)), power });
        }
        let end = rot(Point::new(segs.last().unwrap().1 + dir * overscan, y));
        if overscan > 0.0 {
            ops.push(Op::Move { to: end, power: 0.0 });
        }
        cursor = Some(end);
        row_count += 1;
    }
    ops
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

/// Rasterbild zeilenweise gravieren; leere Zeilen und leere Ränder werden übersprungen.
pub fn raster_ops(r: &RasterResult, settings: &CutSettings) -> Vec<Op> {
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

        let forward = !settings.bidirectional || emitted.is_multiple_of(2);
        let (start_x, edge_x) =
            if forward { (x_at(first) - overscan, x_at(first)) } else { (x_at(last + 1) + overscan, x_at(last + 1)) };
        let start = Point::new(start_x, y);
        if emitted == 0 {
            ops.push(Op::Rapid(start))
        } else {
            ops.push(Op::Move { to: start, power: 0.0 })
        }
        if overscan > 0.0 {
            ops.push(Op::Move { to: Point::new(edge_x, y), power: 0.0 });
        }
        if forward {
            for &(_, e, p) in &runs {
                ops.push(Op::Move { to: Point::new(x_at(e + 1), y), power: p });
            }
            if overscan > 0.0 {
                ops.push(Op::Move { to: Point::new(x_at(last + 1) + overscan, y), power: 0.0 });
            }
        } else {
            for &(s, _, p) in runs.iter().rev() {
                ops.push(Op::Move { to: Point::new(x_at(s), y), power: p });
            }
            if overscan > 0.0 {
                ops.push(Op::Move { to: Point::new(x_at(first) - overscan, y), power: 0.0 });
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
    fn hatch_square_covers_area() {
        let polys = VectorPath::rect(0.0, 0.0, 10.0, 10.0, 0.0).flatten(&Affine::IDENTITY, 0.01);
        let ops = hatch_ops(&polys, 0.5, 0.0, true, 1.0, 0.5);
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
            let ops = hatch_ops(&polys, 0.05, angle, true, 0.0, 1.0);
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
        let ops = raster_ops(&r, &settings);
        // Zeile 1: Start, ein Lauf über zwei Zellen
        assert_eq!(ops[0], Op::Rapid(Point::new(1.0, 1.5)));
        assert_eq!(ops[1], Op::Move { to: Point::new(3.0, 1.5), power: 0.3 });
        // Zeile 2 rückwärts: Start rechts, dann Brennen, Pause, Brennen
        assert_eq!(ops[2].target(), Point::new(4.0, 2.5));
        assert!((burned_length(&ops) - 4.0).abs() < 1e-9);
    }
}
