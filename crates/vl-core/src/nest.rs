//! Verschachteln: Teile platzsparend auf eine Platte legen (eigenes Rasterverfahren).
//!
//! Platte und Teile werden auf ein Zellraster gelegt; ein Teil belegt jede Zelle, die es berührt, damit sich nichts
//! überlappt. Platzierte Teile sperren ihre Zellen samt Mindestabstand. Jedes weitere Teil (größte zuerst) sucht die
//! erste freie Lage an der gewählten Ecke; bei mehreren erlaubten Drehungen gewinnt die Lage, die am wenigsten weit
//! in die Platte reicht. Löcher größerer Teile bleiben frei und können kleine Teile aufnehmen.

use crate::geom::{Affine, Point, Rect};
use crate::ops::{Corner, offset, outermost};
use crate::path::{Polyline, bounds_of};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NestOptions {
    /// Mindestabstand zwischen den Teilen (mm).
    pub spacing: f64,
    /// Abstand zum Plattenrand (mm).
    pub margin: f64,
    /// Erlaubte Drehungen: 1 = keine, 2 = 180°, 4 = 90°-Schritte, 8 = 45°-Schritte.
    pub rotations: u32,
    /// Kleine Teile dürfen in Löcher größerer Teile.
    pub use_holes: bool,
    /// Ecke, an der gepackt wird (Dokumentkoordinaten, y nach unten).
    pub right: bool,
    pub bottom: bool,
}

impl Default for NestOptions {
    fn default() -> Self {
        Self { spacing: 2.0, margin: 2.0, rotations: 4, use_holes: true, right: false, bottom: false }
    }
}

/// Zellen höchstens so vieler Spalten/Zeilen über die längere Plattenseite.
const MAX_CELLS: f64 = 1000.0;

/// Bitraster: Zeilen aus 64-Bit-Wörtern.
#[derive(Clone)]
struct Bits {
    w: usize,
    h: usize,
    words: usize,
    data: Vec<u64>,
}

impl Bits {
    fn new(w: usize, h: usize) -> Self {
        let words = w.div_ceil(64).max(1);
        Bits { w, h, words, data: vec![0; words * h] }
    }

    fn set(&mut self, x: i64, y: i64) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.data[y as usize * self.words + x as usize / 64] |= 1 << (x as usize % 64);
        }
    }

    fn get(&self, x: usize, y: usize) -> bool {
        self.data[y * self.words + x / 64] & (1 << (x % 64)) != 0
    }

    fn row(&self, y: usize) -> &[u64] {
        &self.data[y * self.words..(y + 1) * self.words]
    }

    fn count(&self) -> usize {
        self.data.iter().map(|w| w.count_ones() as usize).sum()
    }
}

/// Markiert jede Zelle, durch die die Strecke läuft (Zellgröße 1, Koordinaten in Zellen).
fn mark_segment(bits: &mut Bits, a: Point, b: Point) {
    let (mut x, mut y) = (a.x.floor() as i64, a.y.floor() as i64);
    let (ex, ey) = (b.x.floor() as i64, b.y.floor() as i64);
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let step = |d: f64| if d > 0.0 { 1 } else { -1 };
    let (sx, sy) = (step(dx), step(dy));
    let next = |p: f64, cell: i64, d: f64| {
        if d > 0.0 {
            ((cell + 1) as f64 - p) / d
        } else if d < 0.0 {
            (cell as f64 - p) / d
        } else {
            f64::INFINITY
        }
    };
    let (mut tx, mut ty) = (next(a.x, x, dx), next(a.y, y, dy));
    let (ddx, ddy) = (
        if dx != 0.0 { 1.0 / dx.abs() } else { f64::INFINITY },
        if dy != 0.0 { 1.0 / dy.abs() } else { f64::INFINITY },
    );
    bits.set(x, y);
    for _ in 0..(ex - x).abs() + (ey - y).abs() {
        if tx < ty {
            tx += ddx;
            x += sx;
        } else {
            ty += ddy;
            y += sy;
        }
        bits.set(x, y);
    }
}

fn ring_segments(p: &Polyline) -> impl Iterator<Item = (Point, Point)> + '_ {
    let n = p.points.len();
    let m = if p.closed { n } else { n.saturating_sub(1) };
    (0..m).map(move |i| (p.points[i], p.points[(i + 1) % n]))
}

/// Zellen, deren Mitte nach der Gerade-Ungerade-Regel innerhalb der geschlossenen Linienzüge liegt.
fn fill_centers(bits: &mut Bits, polys: &[Polyline]) {
    let mut xs = Vec::new();
    for row in 0..bits.h {
        let y = row as f64 + 0.5;
        xs.clear();
        for p in polys.iter().filter(|p| p.closed && p.points.len() >= 3) {
            for (a, b) in ring_segments(p) {
                if (a.y <= y) != (b.y <= y) {
                    xs.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
                }
            }
        }
        xs.sort_by(f64::total_cmp);
        for [x0, x1] in xs.as_chunks::<2>().0 {
            let from = (x0 - 0.5).ceil().max(0.0) as i64;
            let to = (x1 - 0.5).floor() as i64;
            for x in from..=to {
                bits.set(x, row as i64);
            }
        }
    }
}

/// Alle Zellen, die die Linienzüge berühren oder einschließen (Koordinaten bereits in Zellen).
fn rasterize(polys: &[Polyline], fill: &[Polyline], w: usize, h: usize) -> Bits {
    let mut bits = Bits::new(w, h);
    fill_centers(&mut bits, fill);
    for p in polys {
        if p.points.len() == 1 {
            bits.set(p.points[0].x.floor() as i64, p.points[0].y.floor() as i64);
        }
        for (a, b) in ring_segments(p) {
            mark_segment(&mut bits, a, b);
        }
    }
    bits
}

/// Ein Teil in einer Drehung: Raster und die Lage seines Rechtecks im Teilsystem.
struct Pose {
    angle: f64,
    min: Point,
    bits: Bits,
}

fn transformed(polys: &[Polyline], t: &Affine) -> Vec<Polyline> {
    polys.iter().map(|p| Polyline::new(p.points.iter().map(|q| t.apply(*q)).collect(), p.closed)).collect()
}

fn pose(polys: &[Polyline], center: Point, angle: f64, cell: f64, use_holes: bool) -> Pose {
    let rotated = transformed(polys, &Affine::rotate_about(angle, center));
    let b = bounds_of(&rotated);
    let to_cells = Affine::translate(-b.min.x, -b.min.y).then(&Affine::scale(1.0 / cell, 1.0 / cell));
    let local = transformed(&rotated, &to_cells);
    let fill = if use_holes { local.clone() } else { outermost(&local) };
    let (w, h) = ((b.width() / cell).floor() as usize + 1, (b.height() / cell).floor() as usize + 1);
    Pose { angle, min: b.min, bits: rasterize(&local, &fill, w, h) }
}

/// Passt das Teil an Zelle (`gx`, `gy`), ohne gesperrte Zellen zu berühren?
fn fits(blocked: &Bits, part: &Bits, gx: usize, gy: usize) -> bool {
    let (wo, s) = (gx / 64, gx % 64);
    for i in 0..part.h {
        let g = blocked.row(gy + i);
        for (j, &p) in part.row(i).iter().enumerate() {
            if p == 0 {
                continue;
            }
            if g[wo + j] & (p << s) != 0 {
                return false;
            }
            if s > 0 && p >> (64 - s) != 0 && g.get(wo + j + 1).is_none_or(|&w| w & (p >> (64 - s)) != 0) {
                return false;
            }
        }
    }
    true
}

/// Sperrt die Zellen des Teils, um `k` Zellen in alle Richtungen erweitert (Mindestabstand). Je Zeile bzw. Spalte
/// genügt der Abstand zur zuletzt belegten Zelle.
fn block(blocked: &mut Bits, part: &Bits, gx: usize, gy: usize, k: usize) {
    let (w, h) = (part.w + 2 * k, part.h + 2 * k);
    let mut wide = vec![false; w * part.h];
    for y in 0..part.h {
        let mut last: Option<usize> = None;
        for x in 0..w {
            if x < part.w && part.get(x, y) {
                last = Some(x);
            }
            wide[y * w + x] = last.is_some_and(|l| x - l <= 2 * k);
        }
    }
    for x in 0..w {
        let mut last: Option<usize> = None;
        for y in 0..h {
            if y < part.h && wide[y * w + x] {
                last = Some(y);
            }
            if last.is_some_and(|l| y - l <= 2 * k) {
                blocked.set((gx + x) as i64 - k as i64, (gy + y) as i64 - k as i64);
            }
        }
    }
}

/// Legt die Teile (je Teil seine Linienzüge in Dokumentkoordinaten) auf die Platte (geschlossene Linienzüge,
/// Gerade-Ungerade-Regel). Ergebnis je Teil: die Transformation, die es an seinen Platz bringt, oder `None`, wenn es
/// nicht mehr passt.
pub fn nest(parts: &[Vec<Polyline>], sheet: &[Polyline], opts: &NestOptions) -> Vec<Option<Affine>> {
    let mut result = vec![None; parts.len()];
    let sheet_bounds = bounds_of(sheet);
    if sheet_bounds.is_empty() || parts.is_empty() {
        return result;
    }
    // Gepackt wird immer oben links; andere Ecken werden gespiegelt und am Ende zurückgespiegelt.
    let mirror = Affine::scale_about(
        if opts.right { -1.0 } else { 1.0 },
        if opts.bottom { -1.0 } else { 1.0 },
        sheet_bounds.center(),
    );
    let sheet = transformed(sheet, &mirror);
    let usable = if opts.margin > 0.0 { offset(&sheet, -opts.margin, Corner::Sharp) } else { sheet };
    let ub = bounds_of(&usable);
    if ub.is_empty() {
        return result;
    }
    let cell = (ub.width().max(ub.height()) / MAX_CELLS).max(0.05);
    let (gw, gh) = ((ub.width() / cell).ceil() as usize, (ub.height() / cell).ceil() as usize);
    let to_cells = Affine::translate(-ub.min.x, -ub.min.y).then(&Affine::scale(1.0 / cell, 1.0 / cell));
    let local = transformed(&usable, &to_cells);
    // frei sind nur Zellen ganz innerhalb der nutzbaren Fläche
    let inside = rasterize(&[], &local, gw, gh);
    let mut edge = Bits::new(gw, gh);
    for p in &local {
        for (a, b) in ring_segments(p) {
            mark_segment(&mut edge, a, b);
        }
    }
    let mut blocked = Bits::new(gw, gh);
    for (i, w) in blocked.data.iter_mut().enumerate() {
        *w = !inside.data[i] | edge.data[i];
    }
    let k = if opts.spacing > 0.0 { (opts.spacing / cell - 1e-9).ceil() as usize } else { 0 };

    let steps = opts.rotations.max(1);
    let mut order: Vec<(usize, Vec<Polyline>, Point, f64)> = parts
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let polys = transformed(p, &mirror);
            let b = bounds_of(&polys);
            (!b.is_empty()).then(|| {
                let area = pose(&polys, b.center(), 0.0, cell, false).bits.count() as f64;
                (i, polys, b.center(), area)
            })
        })
        .collect();
    order.sort_by(|a, b| b.3.total_cmp(&a.3));

    for (index, polys, center, _) in order {
        let mut best: Option<(usize, usize, usize, Pose)> = None;
        for s in 0..steps {
            let angle = s as f64 * std::f64::consts::TAU / steps as f64;
            let p = pose(&polys, center, angle, cell, opts.use_holes);
            let (w, h) = (p.bits.w, p.bits.h);
            if w > gw || h > gh {
                continue;
            }
            let limit = best.as_ref().map_or(usize::MAX, |b| b.0);
            'rows: for gy in 0..=gh - h {
                if gy + h >= limit {
                    break;
                }
                for gx in 0..=gw - w {
                    if fits(&blocked, &p.bits, gx, gy) {
                        best = Some((gy + h, gx, gy, p));
                        break 'rows;
                    }
                }
            }
        }
        if let Some((_, gx, gy, p)) = best {
            block(&mut blocked, &p.bits, gx, gy, k);
            let target = Point::new(ub.min.x + gx as f64 * cell, ub.min.y + gy as f64 * cell);
            let placed =
                Affine::rotate_about(p.angle, center).then(&Affine::translate(target.x - p.min.x, target.y - p.min.y));
            result[index] = Some(mirror.then(&placed).then(&mirror));
        }
    }
    result
}

/// Plattenrechteck als geschlossener Linienzug.
pub fn sheet_rect(r: Rect) -> Vec<Polyline> {
    vec![Polyline::new(r.corners().to_vec(), true)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Polyline {
        Polyline::new(
            vec![Point::new(x, y), Point::new(x + w, y), Point::new(x + w, y + h), Point::new(x, y + h)],
            true,
        )
    }

    fn placed(part: &[Polyline], t: &Affine) -> Rect {
        bounds_of(&transformed(part, t))
    }

    fn gap(a: &Rect, b: &Rect) -> f64 {
        let dx = (b.min.x - a.max.x).max(a.min.x - b.max.x).max(0.0);
        let dy = (b.min.y - a.max.y).max(a.min.y - b.max.y).max(0.0);
        dx.hypot(dy)
    }

    #[test]
    fn packs_squares_with_spacing_inside_the_sheet() {
        let sheet = sheet_rect(Rect::from_xywh(0.0, 0.0, 100.0, 100.0));
        let parts: Vec<Vec<Polyline>> = (0..9).map(|i| vec![rect(200.0 + i as f64 * 50.0, 0.0, 40.0, 40.0)]).collect();
        let opts = NestOptions { spacing: 2.0, margin: 1.0, ..Default::default() };
        let res = nest(&parts, &sheet, &opts);
        let boxes: Vec<Rect> = parts.iter().zip(&res).filter_map(|(p, t)| t.map(|t| placed(p, &t))).collect();
        assert_eq!(boxes.len(), 4, "2 × 2 passen auf 100 × 100");
        let inner = Rect::from_xywh(1.0, 1.0, 98.0, 98.0).expanded(1e-6);
        for (i, a) in boxes.iter().enumerate() {
            assert!(inner.contains_rect(a), "{a:?}");
            assert!((a.width() - 40.0).abs() < 1e-6 && (a.height() - 40.0).abs() < 1e-6);
            for b in &boxes[i + 1..] {
                assert!(gap(a, b) >= 2.0 - 1e-6, "{a:?} {b:?}");
            }
        }
        // oben links gepackt
        assert!(boxes.iter().any(|b| b.min.x < 2.0 && b.min.y < 2.0));
    }

    #[test]
    fn small_part_goes_into_a_hole() {
        // Rahmen 60 × 60 mit Loch 40 × 40, Platte gerade groß genug für den Rahmen
        let frame = vec![rect(0.0, 0.0, 60.0, 60.0), rect(10.0, 10.0, 40.0, 40.0)];
        let small = vec![rect(100.0, 100.0, 20.0, 20.0)];
        let sheet = sheet_rect(Rect::from_xywh(0.0, 0.0, 62.0, 62.0));
        let opts = NestOptions { spacing: 2.0, margin: 0.0, ..Default::default() };
        let res = nest(&[frame.clone(), small.clone()], &sheet, &opts);
        let f = placed(&frame, &res[0].unwrap());
        let s = placed(&small, &res[1].expect("kleines Teil im Loch"));
        let hole = Rect::new(f.min + Point::new(10.0, 10.0), f.max - Point::new(10.0, 10.0));
        assert!(hole.expanded(1e-6).contains_rect(&s), "{s:?} in {hole:?}");
        // ohne Löcher passt es nicht mehr
        let res = nest(&[frame, small], &sheet, &NestOptions { use_holes: false, ..opts });
        assert!(res[1].is_none());
    }

    #[test]
    fn rotates_when_needed_and_packs_into_the_chosen_corner() {
        let bar = vec![rect(0.0, 0.0, 90.0, 10.0)];
        let sheet = sheet_rect(Rect::from_xywh(0.0, 0.0, 20.0, 100.0));
        let fixed = NestOptions { rotations: 1, margin: 0.0, ..Default::default() };
        assert!(nest(std::slice::from_ref(&bar), &sheet, &fixed)[0].is_none());
        let t = nest(std::slice::from_ref(&bar), &sheet, &NestOptions { rotations: 4, ..fixed })[0].unwrap();
        let b = placed(&bar, &t);
        assert!((b.width() - 10.0).abs() < 1e-6 && (b.height() - 90.0).abs() < 1e-6);
        assert!(t.determinant() > 0.0, "nicht gespiegelt");
        // unten rechts packen
        let sheet = sheet_rect(Rect::from_xywh(0.0, 0.0, 200.0, 100.0));
        let t =
            nest(std::slice::from_ref(&bar), &sheet, &NestOptions { right: true, bottom: true, ..fixed })[0].unwrap();
        let b = placed(&bar, &t);
        assert!(b.max.x > 199.0 && b.max.y > 99.0, "{b:?}");
        assert!(t.determinant() > 0.0);
    }
}
