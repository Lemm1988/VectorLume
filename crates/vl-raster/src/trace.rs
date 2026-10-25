//! Bild nachzeichnen: Schwellwert → Schwarzweiß-Maske → geschlossene Konturen entlang der Pixelkanten,
//! vereinfacht (Douglas-Peucker) und optional zu Bézierkurven geglättet. Ergebnis in Pixelkoordinaten
//! (x nach rechts, y nach unten, Pixel (0,0) belegt 0…1).
//!
//! Mit `centerline` entstehen stattdessen Mittellinien: die Maske wird auf ein Pixel Breite ausgedünnt
//! (Zhang-Suen), das Skelett zwischen End- und Verzweigungspunkten in offene Linienzüge zerlegt, kurze Fransen
//! an Ecken und Enden werden verworfen und die übrigen Stücke an Knoten mit nur zwei Enden verbunden.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use vl_core::{ImageAdjust, Point, RasterImage, VectorPath};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TraceOptions {
    /// Grauwerte unter dem Schwellwert gelten als Motiv (0 = schwarz … 255 = weiß).
    pub threshold: u8,
    /// Helle statt dunkler Bereiche nachzeichnen.
    pub invert: bool,
    /// Konturen mit kleinerer Fläche (Pixel²) werden verworfen (Staub, Rauschen).
    pub min_area: f64,
    /// Erlaubte Abweichung beim Vereinfachen in Pixeln.
    pub tolerance: f64,
    /// 0 = eckig, 1 = maximal rund (Bézierkurven).
    pub smoothing: f64,
    /// Knicke über diesem Winkel (Grad) bleiben beim Glätten Ecken.
    pub corner_angle: f64,
    /// Mittellinien statt Umrissen (für Strichzeichnungen, Handschrift, schnelles Ritzen).
    pub centerline: bool,
}

impl Default for TraceOptions {
    fn default() -> Self {
        Self {
            threshold: 128,
            invert: false,
            min_area: 4.0,
            tolerance: 0.8,
            smoothing: 0.7,
            corner_angle: 65.0,
            centerline: false,
        }
    }
}

/// Richtungen im Uhrzeigersinn (Bildschirmkoordinaten): rechts, unten, links, oben.
const DIRS: [(i64, i64); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

/// Zeichnet das Bild nach. `adjust` wird vor dem Schwellwert angewendet (wie in der Ansicht).
pub fn trace(img: &RasterImage, adjust: &ImageAdjust, opts: &TraceOptions) -> Vec<VectorPath> {
    let (w, h) = (img.width as usize, img.height as usize);
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let identity = adjust.is_identity();
    let lut: Vec<bool> = (0..=255u8)
        .map(|v| {
            let g = if identity { v } else { (adjust.apply(v as f32 / 255.0) * 255.0).round() as u8 };
            (g < opts.threshold) != opts.invert
        })
        .collect();
    let inside = |x: i64, y: i64| -> bool {
        x >= 0
            && y >= 0
            && (x as usize) < w
            && (y as usize) < h
            && lut[img.pixels[y as usize * w + x as usize] as usize]
    };
    if opts.centerline {
        let mask = img.pixels.iter().map(|&v| lut[v as usize]).collect();
        return centerline(mask, w, h, opts);
    }

    // Ausgehende Kanten je Ecke als Bitmaske (Bit = Richtung). Motiv liegt rechts der Laufrichtung.
    let vw = w + 1;
    let mut out = vec![0u8; vw * (h + 1)];
    let vid = |x: i64, y: i64| y as usize * vw + x as usize;
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            if !inside(x, y) {
                continue;
            }
            if !inside(x, y - 1) {
                out[vid(x, y)] |= 1 << 0; // oben: nach rechts
            }
            if !inside(x + 1, y) {
                out[vid(x + 1, y)] |= 1 << 1; // rechts: nach unten
            }
            if !inside(x, y + 1) {
                out[vid(x + 1, y + 1)] |= 1 << 2; // unten: nach links
            }
            if !inside(x - 1, y) {
                out[vid(x, y + 1)] |= 1 << 3; // links: nach oben
            }
        }
    }

    let mut result = Vec::new();
    for start in 0..out.len() {
        while out[start] != 0 {
            let (sx, sy) = ((start % vw) as i64, (start / vw) as i64);
            let mut d = out[start].trailing_zeros() as usize;
            let (mut x, mut y) = (sx, sy);
            // Mittelpunkte der Pixelkanten ergeben schräge statt treppenförmiger Linien.
            let mut pts: Vec<Point> = Vec::new();
            loop {
                out[vid(x, y)] &= !(1 << d);
                let (dx, dy) = DIRS[d];
                pts.push(Point::new(x as f64 + dx as f64 * 0.5, y as f64 + dy as f64 * 0.5));
                x += dx;
                y += dy;
                if (x, y) == (sx, sy) {
                    break;
                }
                // Rechts abbiegen (zum Motiv hin) hat Vorrang: diagonal berührende Pixel bleiben getrennt.
                let m = out[vid(x, y)];
                let Some(next) = [(d + 1) % 4, d, (d + 3) % 4].into_iter().find(|&n| m & (1 << n) != 0) else {
                    break;
                };
                d = next;
            }
            if pts.len() < 4 || area(&pts).abs() < opts.min_area.max(0.0) {
                continue;
            }
            let simplified = vl_core::ops::simplify_points(&pts, true, opts.tolerance.max(0.0));
            if simplified.len() >= 3 {
                result.push(vl_core::ops::smooth_points(&simplified, true, opts.smoothing, opts.corner_angle));
            }
        }
    }
    result
}

/// Nachbarn im Uhrzeigersinn ab oben: N, NO, O, SO, S, SW, W, NW.
const RING: [(i64, i64); 8] = [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)];

fn centerline(mut mask: Vec<bool>, w: usize, h: usize, opts: &TraceOptions) -> Vec<VectorPath> {
    remove_small_components(&mut mask, w, h, opts.min_area);
    let area = mask.iter().filter(|&&b| b).count() as f64;
    thin(&mut mask, w, h);
    let (mut strokes, rings) = skeleton_strokes(&mask, w, h);
    // Strichbreite ≈ Fläche / Skelettlänge; Fransen sind etwa halb so lang
    let skeleton: f64 = strokes
        .iter()
        .map(|s| polyline_length(&s.pts, false))
        .chain(rings.iter().map(|r| polyline_length(r, true)))
        .sum();
    let width = if skeleton > 0.0 { area / skeleton } else { 0.0 };
    prune_spurs(&mut strokes, 0.75 * width + 1.0);
    collapse_bridges(&mut strokes, width + 1.0);
    let (open, mut closed) = join_at_nodes(strokes, (1.5 * width).max(3.0));
    closed.extend(rings);
    let tol = opts.tolerance.max(0.0);
    let mut result = Vec::new();
    for (pts, is_closed) in open.iter().map(|p| (p, false)).chain(closed.iter().map(|p| (p, true))) {
        if pts.len() < 2 || polyline_length(pts, is_closed) < 1.5 {
            continue;
        }
        let simplified = vl_core::ops::simplify_points(pts, is_closed, tol);
        if simplified.len() >= if is_closed { 3 } else { 2 } {
            result.push(vl_core::ops::smooth_points(&simplified, is_closed, opts.smoothing, opts.corner_angle));
        }
    }
    result
}

/// Linienzug des Skeletts zwischen zwei Knoten (`a`, `b`: freies Ende = Pixelindex, Verzweigung = Index der
/// Pixelgruppe + Pixelzahl).
#[derive(Default)]
struct Stroke {
    pts: Vec<Point>,
    a: usize,
    b: usize,
}

fn polyline_length(pts: &[Point], closed: bool) -> f64 {
    let open: f64 = pts.windows(2).map(|p| p[0].distance(p[1])).sum();
    match (closed, pts.first(), pts.last()) {
        (true, Some(a), Some(b)) => open + a.distance(*b),
        _ => open,
    }
}

/// Entfernt zusammenhängende Flecken (8er-Nachbarschaft) mit weniger als `min_area` Pixeln.
fn remove_small_components(mask: &mut [bool], w: usize, h: usize, min_area: f64) {
    if min_area <= 1.0 {
        return;
    }
    let mut seen = vec![false; mask.len()];
    let mut stack = Vec::new();
    let mut comp = Vec::new();
    for start in 0..mask.len() {
        if !mask[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        comp.clear();
        while let Some(i) = stack.pop() {
            comp.push(i);
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            for (dx, dy) in RING {
                let (nx, ny) = (x + dx, y + dy);
                if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                    let j = ny as usize * w + nx as usize;
                    if mask[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        if (comp.len() as f64) < min_area {
            for &i in &comp {
                mask[i] = false;
            }
        }
    }
}

/// Belegung der acht Nachbarn in `RING`-Reihenfolge (außerhalb = leer).
fn ring(mask: &[bool], w: usize, h: usize, i: usize) -> [bool; 8] {
    let (x, y) = ((i % w) as i64, (i / w) as i64);
    RING.map(|(dx, dy)| {
        let (nx, ny) = (x + dx, y + dy);
        nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h && mask[ny as usize * w + nx as usize]
    })
}

/// Zhang-Suen-Ausdünnung, danach Eckpixel an Treppenstufen entfernen (Skelett ein Pixel breit in 8er-Nachbarschaft).
fn thin(mask: &mut [bool], w: usize, h: usize) {
    let mut remove = Vec::new();
    loop {
        let mut changed = false;
        for step in 0..2 {
            remove.clear();
            for i in 0..mask.len() {
                if !mask[i] {
                    continue;
                }
                let r = ring(mask, w, h, i);
                let b = r.iter().filter(|&&v| v).count();
                let a = (0..8).filter(|&k| !r[k] && r[(k + 1) % 8]).count();
                let (n, e, s, wv) = (r[0], r[2], r[4], r[6]);
                let ok = if step == 0 { !(n && e && s) && !(e && s && wv) } else { !(n && e && wv) && !(n && s && wv) };
                if (2..=6).contains(&b) && a == 1 && ok {
                    remove.push(i);
                }
            }
            for &i in &remove {
                mask[i] = false;
            }
            changed |= !remove.is_empty();
        }
        if !changed {
            break;
        }
    }
    // Ecke zweier gesetzter Nachbarn (N+O, O+S, S+W, W+N), deren Gegenseite leer ist: die beiden hängen auch
    // diagonal zusammen, das Eckpixel ist überflüssig
    for i in 0..mask.len() {
        if !mask[i] {
            continue;
        }
        let r = ring(mask, w, h, i);
        let corner = (0..4).any(|q| {
            let k = 2 * q;
            r[k] && r[(k + 2) % 8] && !r[(k + 4) % 8] && !r[(k + 5) % 8] && !r[(k + 6) % 8]
        });
        if corner {
            mask[i] = false;
        }
    }
}

/// Zerlegt das Skelett in Linienzüge zwischen Knoten und in geschlossene Ringe. Benachbarte Verzweigungspixel
/// bilden zusammen einen Knoten (Mittelpunkt der Gruppe), damit Kreuzungen nicht in viele Kleinststücke zerfallen.
fn skeleton_strokes(mask: &[bool], w: usize, h: usize) -> (Vec<Stroke>, Vec<Vec<Point>>) {
    let n = mask.len();
    let neighbors = |i: usize| {
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        RING.iter().filter_map(move |(dx, dy)| {
            let (nx, ny) = (x + dx, y + dy);
            (nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h)
                .then(|| ny as usize * w + nx as usize)
                .filter(|&j| mask[j])
        })
    };
    let degree: Vec<u8> = (0..n).map(|i| if mask[i] { neighbors(i).count() as u8 } else { 0 }).collect();
    let center = |i: usize| Point::new((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
    // Gruppen benachbarter Verzweigungspixel
    let mut group = vec![usize::MAX; n];
    let mut group_center: Vec<Point> = Vec::new();
    for start in 0..n {
        if degree[start] < 3 || group[start] != usize::MAX {
            continue;
        }
        let id = group_center.len();
        let mut stack = vec![start];
        group[start] = id;
        let (mut sum, mut count) = (Point::ZERO, 0.0);
        while let Some(i) = stack.pop() {
            sum += center(i);
            count += 1.0;
            for j in neighbors(i) {
                if degree[j] >= 3 && group[j] == usize::MAX {
                    group[j] = id;
                    stack.push(j);
                }
            }
        }
        group_center.push(sum * (1.0 / count));
    }
    let node_key = |i: usize| if degree[i] >= 3 { n + group[i] } else { i };
    let node_point = |i: usize| if degree[i] >= 3 { group_center[group[i]] } else { center(i) };
    let is_node = |i: usize| degree[i] != 2;
    let edge = |a: usize, b: usize| (a.min(b), a.max(b));
    let mut used: HashSet<(usize, usize)> = HashSet::new();
    let mut strokes = Vec::new();
    for i in (0..n).filter(|&i| mask[i] && is_node(i)) {
        for m in neighbors(i).collect::<Vec<_>>() {
            if used.contains(&edge(i, m)) {
                continue;
            }
            used.insert(edge(i, m));
            // Kanten innerhalb einer Verzweigungsgruppe gehören zum Knoten
            if degree[i] >= 3 && degree[m] >= 3 && group[i] == group[m] {
                continue;
            }
            let mut pts = vec![node_point(i)];
            let (mut prev, mut cur) = (i, m);
            while !is_node(cur) {
                pts.push(center(cur));
                let Some(next) = neighbors(cur).find(|&k| k != prev && !used.contains(&edge(cur, k))) else { break };
                used.insert(edge(cur, next));
                (prev, cur) = (cur, next);
            }
            if is_node(cur) {
                pts.push(node_point(cur));
            }
            strokes.push(Stroke { pts, a: node_key(i), b: if is_node(cur) { node_key(cur) } else { cur } });
        }
    }
    // Übrig sind Ringe ohne Knoten
    let mut rings = Vec::new();
    for i in (0..n).filter(|&i| mask[i] && degree[i] == 2) {
        let Some(m) = neighbors(i).find(|&m| !used.contains(&edge(i, m))) else { continue };
        used.insert(edge(i, m));
        let mut pts = vec![center(i)];
        let (mut prev, mut cur) = (i, m);
        while cur != i {
            pts.push(center(cur));
            let Some(next) = neighbors(cur).find(|&k| k != prev && !used.contains(&edge(cur, k))) else { break };
            used.insert(edge(cur, next));
            (prev, cur) = (cur, next);
        }
        if pts.len() >= 3 {
            rings.push(pts);
        }
    }
    (strokes, rings)
}

fn end_counts(strokes: &[Stroke]) -> HashMap<usize, usize> {
    let mut count: HashMap<usize, usize> = HashMap::new();
    for s in strokes {
        *count.entry(s.a).or_default() += 1;
        *count.entry(s.b).or_default() += 1;
    }
    count
}

/// Verwirft kurze Stücke zwischen einem freien Ende und einer Verzweigung.
fn prune_spurs(strokes: &mut Vec<Stroke>, max_len: f64) {
    let count = end_counts(strokes);
    strokes.retain(|s| {
        let (a, b) = (count[&s.a], count[&s.b]);
        let spur = (a == 1) != (b == 1) && a.max(b) >= 3;
        !(spur && polyline_length(&s.pts, false) < max_len)
    });
}

/// Kurze Brücken zwischen zwei Verzweigungen (an schrägen Kreuzungen spaltet die Ausdünnung den Knoten auf)
/// werden zu einem Knoten in ihrer Mitte zusammengezogen.
fn collapse_bridges(strokes: &mut Vec<Stroke>, max_len: f64) {
    loop {
        let count = end_counts(strokes);
        let Some(k) = strokes.iter().position(|s| {
            s.a != s.b && count[&s.a] >= 3 && count[&s.b] >= 3 && polyline_length(&s.pts, false) < max_len
        }) else {
            break;
        };
        let bridge = strokes.remove(k);
        let mid = bridge.pts[0].lerp(bridge.pts[bridge.pts.len() - 1], 0.5);
        for s in strokes.iter_mut() {
            if s.a == bridge.a || s.a == bridge.b {
                s.a = bridge.a;
                s.pts[0] = mid;
            }
            if s.b == bridge.a || s.b == bridge.b {
                s.b = bridge.a;
                let last = s.pts.len() - 1;
                s.pts[last] = mid;
            }
        }
    }
}

/// Ende eines Linienzugs: Index und `true` für den Anfang.
type StrokeEnd = (usize, bool);

/// Richtung, in der ein Linienzug seinen Knoten verlässt (gemittelt über die Strecke `reach`).
fn leaving_direction(s: &Stroke, at_start: bool, reach: f64) -> Point {
    let n = s.pts.len();
    let at = |k: usize| if at_start { s.pts[k] } else { s.pts[n - 1 - k] };
    let origin = at(0);
    let mut p = at(1);
    let mut walked = 0.0;
    for k in 1..n {
        walked += at(k).distance(at(k - 1));
        p = at(k);
        if walked >= reach {
            break;
        }
    }
    let d = p - origin;
    let len = d.length();
    if len < 1e-9 { Point::ZERO } else { d * (1.0 / len) }
}

/// Verbindet Linienzüge an Knoten: treffen sich dort nur zwei Enden, immer; an Verzweigungen die Enden, die
/// fast gerade durchlaufen (ein X wird zu zwei Linien, ein T zu Balken und Stiel). Liefert offene und dabei
/// geschlossene Linienzüge.
fn join_at_nodes(mut strokes: Vec<Stroke>, reach: f64) -> (Vec<Vec<Point>>, Vec<Vec<Point>>) {
    // höchstens 30° Abweichung von der Geraden
    const STRAIGHT: f64 = -0.866;
    let mut closed = Vec::new();
    loop {
        let mut ends: HashMap<usize, Vec<StrokeEnd>> = HashMap::new();
        for (i, s) in strokes.iter().enumerate() {
            ends.entry(s.a).or_default().push((i, true));
            ends.entry(s.b).or_default().push((i, false));
        }
        // Paare nach Güte: erzwungene (nur zwei Enden) zuerst, dann die geradesten
        let mut pairs: Vec<(f64, usize, StrokeEnd, StrokeEnd)> = Vec::new();
        for (key, v) in ends {
            if v.len() == 2 {
                pairs.push((-2.0, key, v[0], v[1]));
            } else if v.len() >= 3 {
                let dirs: Vec<Point> =
                    v.iter().map(|&(i, start)| leaving_direction(&strokes[i], start, reach)).collect();
                for x in 0..v.len() {
                    for y in x + 1..v.len() {
                        let c = dirs[x].dot(dirs[y]);
                        if c < STRAIGHT && v[x].0 != v[y].0 {
                            pairs.push((c, key, v[x], v[y]));
                        }
                    }
                }
            }
        }
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut alive = vec![true; strokes.len()];
        let mut touched = vec![false; strokes.len()];
        let mut merged = false;
        for (_, _, (i, i_start), (j, j_start)) in pairs {
            if touched[i] || touched[j] {
                continue;
            }
            touched[i] = true;
            touched[j] = true;
            merged = true;
            if i == j {
                // beide Enden desselben Zugs: Ring
                let mut ring = std::mem::take(&mut strokes[i].pts);
                ring.pop();
                closed.push(ring);
                alive[i] = false;
                continue;
            }
            // i so drehen, dass es am Treffpunkt endet, j so, dass es dort beginnt
            let mut a = std::mem::take(&mut strokes[i]);
            let mut b = std::mem::take(&mut strokes[j]);
            if i_start {
                a.pts.reverse();
                std::mem::swap(&mut a.a, &mut a.b);
            }
            if !j_start {
                b.pts.reverse();
                std::mem::swap(&mut b.a, &mut b.b);
            }
            a.pts.extend(b.pts.into_iter().skip(1));
            strokes[i] = Stroke { pts: a.pts, a: a.a, b: b.b };
            alive[j] = false;
        }
        strokes = strokes.into_iter().zip(alive).filter(|(s, a)| *a && !s.pts.is_empty()).map(|(s, _)| s).collect();
        if !merged {
            break;
        }
    }
    (strokes.into_iter().map(|s| s.pts).collect(), closed)
}

fn area(pts: &[Point]) -> f64 {
    let n = pts.len();
    (0..n).map(|i| pts[i].cross(pts[(i + 1) % n])).sum::<f64>() / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::{Affine, Polyline};

    fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> bool) -> RasterImage {
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                px.push(if f(x, y) { 0 } else { 255 });
            }
        }
        RasterImage::new(w, h, px)
    }

    fn sharp() -> TraceOptions {
        TraceOptions { smoothing: 0.0, tolerance: 0.1, min_area: 0.0, ..TraceOptions::default() }
    }

    #[test]
    fn square_with_hole() {
        // 10×10-Quadrat mit 4×4-Loch
        let img = image(20, 20, |x, y| {
            (5..15).contains(&x) && (5..15).contains(&y) && !((8..12).contains(&x) && (8..12).contains(&y))
        });
        let paths = trace(&img, &ImageAdjust::default(), &sharp());
        assert_eq!(paths.len(), 2);
        let mut areas: Vec<f64> =
            paths.iter().map(|p| p.flatten(&Affine::IDENTITY, 0.01)[0].signed_area().abs()).collect();
        areas.sort_by(f64::total_cmp);
        // Kantenmitten: Fläche etwas kleiner als außen, etwas größer als innen (Ecken abgeschrägt)
        assert!((areas[1] - 100.0).abs() < 2.1, "{areas:?}");
        assert!((areas[0] - 16.0).abs() < 2.1, "{areas:?}");
    }

    #[test]
    fn diagonal_pixels_stay_separate_and_specks_vanish() {
        let img =
            image(10, 10, |x, y| (x, y) == (2, 2) || (x, y) == (3, 3) || (7..10).contains(&x) && (7..10).contains(&y));
        let all = trace(&img, &ImageAdjust::default(), &sharp());
        assert_eq!(all.len(), 3);
        let opts = TraceOptions { min_area: 2.0, ..sharp() };
        assert_eq!(trace(&img, &ImageAdjust::default(), &opts).len(), 1);
    }

    #[test]
    fn centerline_follows_strokes() {
        let opts = TraceOptions { centerline: true, ..sharp() };
        // waagrechter Balken 5 px dick: eine Linie in der Mitte, fast so lang wie der Balken
        let bar = image(60, 20, |x, y| (5..55).contains(&x) && (8..13).contains(&y));
        let paths = trace(&bar, &ImageAdjust::default(), &opts);
        assert_eq!(paths.len(), 1, "{paths:?}");
        let poly = &paths[0].flatten(&Affine::IDENTITY, 0.01)[0];
        assert!(!poly.closed);
        assert!(poly.points.iter().all(|p| (p.y - 10.5).abs() < 0.6), "{poly:?}");
        // die Enden schrumpfen beim Ausdünnen um etwa die halbe Strichbreite
        assert!(poly.length() > 42.0, "{}", poly.length());
        // Kreuz: zwei Balken → höchstens zwei Linienzüge, keine Fransen
        let cross = image(60, 60, |x, y| {
            ((8..53).contains(&x) && (28..33).contains(&y)) || ((28..33).contains(&x) && (8..53).contains(&y))
        });
        let paths = trace(&cross, &ImageAdjust::default(), &opts);
        let polys: Vec<Polyline> = paths.iter().flat_map(|p| p.flatten(&Affine::IDENTITY, 0.01)).collect();
        assert!(polys.len() <= 4 && !polys.is_empty(), "{}", polys.len());
        let total: f64 = polys.iter().map(|p| p.length()).sum();
        assert!((total - 2.0 * (45.0 - 5.0)).abs() < 5.0, "{total}");
        // Ring (Kreislinie 3 px dick): ein geschlossener Linienzug mit dem mittleren Radius
        let ring = image(60, 60, |x, y| {
            let d = ((x as f64 - 29.5).powi(2) + (y as f64 - 29.5).powi(2)).sqrt();
            (18.5..21.5).contains(&d)
        });
        let paths = trace(&ring, &ImageAdjust::default(), &opts);
        assert_eq!(paths.len(), 1);
        let poly = &paths[0].flatten(&Affine::IDENTITY, 0.01)[0];
        assert!(poly.closed);
        let r = poly.points.iter().map(|p| p.distance(Point::new(30.0, 30.0))).sum::<f64>() / poly.points.len() as f64;
        assert!((r - 20.0).abs() < 1.0, "{r}");
    }

    #[test]
    fn invert_and_smoothing() {
        let img = image(30, 30, |x, y| {
            let (dx, dy) = (x as f64 - 14.5, y as f64 - 14.5);
            dx * dx + dy * dy < 100.0
        });
        let smooth = trace(&img, &ImageAdjust::default(), &TraceOptions::default());
        assert_eq!(smooth.len(), 1);
        assert!(smooth[0].subpaths[0].segments.iter().any(|s| matches!(s, vl_core::Segment::Cubic(..))));
        let a = smooth[0].flatten(&Affine::IDENTITY, 0.01)[0].signed_area().abs();
        assert!((a - std::f64::consts::PI * 100.0).abs() < 15.0, "{a}");
        // invertiert: der Hintergrund (Rahmen um den Kreis) wird zur Kontur mit Loch
        let inv = trace(&img, &ImageAdjust::default(), &TraceOptions { invert: true, ..sharp() });
        assert_eq!(inv.len(), 2);
    }
}
