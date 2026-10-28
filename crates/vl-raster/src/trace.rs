//! Bild nachzeichnen: geschlossene Konturen auf dem Schwellwert, aus den Grauwerten mit Unterpixel-Genauigkeit
//! bestimmt (Marching Squares), dann mit Ecken-Erkennung zu wenigen glatten Bézierkurven angepasst
//! (`vl_core::fit`). Ergebnis in Pixelkoordinaten (x nach rechts, y nach unten, Pixel (0,0) belegt 0…1).
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
            min_area: 2.0,
            tolerance: 0.8,
            smoothing: 0.7,
            corner_angle: 65.0,
            centerline: false,
        }
    }
}

/// Zeichnet das Bild nach. `adjust` wird vor dem Schwellwert angewendet (wie in der Ansicht).
pub fn trace(img: &RasterImage, adjust: &ImageAdjust, opts: &TraceOptions) -> Vec<VectorPath> {
    trace_with(img, &level_table(adjust, opts), opts)
}

/// Wie `trace`, dazu je Pfad die Nummer des Teils (Aufkleber, Magnet …), zu dem er gehört – 0 = keines
/// (`crate::parts`).
pub fn trace_parts(img: &RasterImage, adjust: &ImageAdjust, opts: &TraceOptions) -> (Vec<VectorPath>, Vec<u32>) {
    let lut = level_table(adjust, opts);
    let paths = trace_with(img, &lut, opts);
    let (w, h) = (img.width as usize, img.height as usize);
    let ink: Vec<bool> = img.pixels.iter().map(|&v| lut[v as usize] > 0.0).collect();
    let parts = crate::parts::path_parts(&paths, &ink, w, h);
    (paths, parts)
}

/// Abstand zum Schwellwert je Grauwert, > 0 = Motiv. Die Schwelle liegt zwischen zwei ganzen Grauwerten, damit
/// kein Pixel genau auf ihr liegt.
fn level_table(adjust: &ImageAdjust, opts: &TraceOptions) -> Vec<f32> {
    let identity = adjust.is_identity();
    let level = opts.threshold as f32 - 0.5;
    let sign = if opts.invert { -1.0 } else { 1.0 };
    (0..=255u8)
        .map(|v| {
            let g = if identity { v as f32 } else { adjust.apply(v as f32 / 255.0) * 255.0 };
            (level - g) * sign
        })
        .collect()
}

fn trace_with(img: &RasterImage, lut: &[f32], opts: &TraceOptions) -> Vec<VectorPath> {
    let (w, h) = (img.width as usize, img.height as usize);
    if w == 0 || h == 0 {
        return Vec::new();
    }
    if opts.centerline {
        let mask = img.pixels.iter().map(|&v| lut[v as usize] > 0.0).collect();
        return centerline(mask, w, h, opts);
    }
    let fit = fit_options(opts);
    iso_contours(&img.pixels, w, h, lut)
        .into_iter()
        .filter(|pts| pts.len() >= 3 && area(pts).abs() >= opts.min_area.max(0.0))
        .map(|pts| {
            // Dünne Formen (feine Striche, kleine Punkte) bekommen eine engere Toleranz und kürzere Messweite,
            // sonst würden sie verbogen oder ihre Enden rund geschliffen. Dicke ≈ 2 · Fläche / Umfang.
            let thickness = 2.0 * area(&pts).abs() / polyline_length(&pts, true).max(1e-9);
            let fit = vl_core::fit::FitOptions {
                tolerance: fit.tolerance.min((0.3 * thickness).max(0.1)),
                reach: fit.reach.min(thickness.max(1.0)),
                ..fit
            };
            vl_core::fit::fit_points(&pts, true, &fit)
        })
        .filter(|p| !p.is_empty())
        .collect()
}

fn fit_options(opts: &TraceOptions) -> vl_core::fit::FitOptions {
    let tolerance = opts.tolerance.max(0.0);
    vl_core::fit::FitOptions {
        tolerance,
        smoothing: opts.smoothing,
        corner_deg: opts.corner_angle,
        reach: 1.5 + tolerance,
    }
}

/// Konturen auf der Schwelle, mit Unterpixel-Genauigkeit (Marching Squares): Die Abtastpunkte sind die
/// Pixelmitten, auf jeder Kante zwischen Motiv und Hintergrund wird die Lage der Schwelle aus den beiden
/// Grauwerten linear eingerechnet. Bei harten Schwarzweiß-Bildern sind das die Mitten der Pixelkanten, bei
/// weichen Kanten (Kantenglättung, Fotos, vergrößerte Bilder) folgt die Kontur dem tatsächlichen Verlauf
/// zwischen den Pixeln. Rings um das Bild liegt ein Rand ohne Motiv, alle Konturen sind daher geschlossen;
/// das Motiv liegt rechts der Laufrichtung, Löcher laufen andersherum.
fn iso_contours(pixels: &[u8], w: usize, h: usize, lut: &[f32]) -> Vec<Vec<Point>> {
    // Gitter mit Rand: Abtastpunkt (gx, gy) = Pixel (gx − 1, gy − 1), Mitte bei (gx − 0.5, gy − 0.5)
    let (gw, gh) = (w + 2, h + 2);
    let val = |gx: usize, gy: usize| -> f32 {
        if gx == 0 || gy == 0 || gx > w || gy > h {
            f32::NEG_INFINITY
        } else {
            lut[pixels[(gy - 1) * w + gx - 1] as usize]
        }
    };
    // Kanten-Nummern: waagrecht (gx, gy)–(gx + 1, gy) gerade, senkrecht (gx, gy)–(gx, gy + 1) ungerade
    let horiz = |gx: usize, gy: usize| 2 * (gy * gw + gx) as u64;
    let vert = |gx: usize, gy: usize| 2 * (gy * gw + gx) as u64 + 1;
    let mut next: HashMap<u64, u64> = HashMap::new();
    let mut order: Vec<u64> = Vec::new();
    for gy in 0..gh - 1 {
        for gx in 0..gw - 1 {
            let v = [val(gx, gy), val(gx + 1, gy), val(gx + 1, gy + 1), val(gx, gy + 1)];
            let ins = v.map(|x| x > 0.0);
            if ins == [ins[0]; 4] {
                continue;
            }
            // Ecken im Uhrzeigersinn: oben links, oben rechts, unten rechts, unten links; Kante k verbindet
            // Ecke k mit Ecke k + 1. Ein Übergang Motiv → Hintergrund „verlässt“, der umgekehrte „betritt“.
            let edges = [horiz(gx, gy), vert(gx + 1, gy), horiz(gx, gy + 1), vert(gx, gy)];
            let cross: Vec<(u64, bool)> =
                (0..4).filter(|&k| ins[k] != ins[(k + 1) % 4]).map(|k| (edges[k], ins[k])).collect();
            if cross.len() == 2 {
                let (leave, enter) = if cross[0].1 { (cross[0].0, cross[1].0) } else { (cross[1].0, cross[0].0) };
                next.insert(leave, enter);
                order.push(leave);
            } else {
                // Sattel: Liegt die Mitte im Motiv, hängen die beiden Motiv-Ecken zusammen (zur nächsten
                // betretenen Kante im Uhrzeigersinn), sonst werden sie einzeln umrundet.
                let joined = v.iter().sum::<f32>() / 4.0 > 0.0;
                for k in (0..4).filter(|&k| cross[k].1) {
                    let enter = if joined { cross[(k + 1) % 4].0 } else { cross[(k + 3) % 4].0 };
                    next.insert(cross[k].0, enter);
                    order.push(cross[k].0);
                }
            }
        }
    }
    let point = |e: u64| -> Point {
        let k = (e / 2) as usize;
        let (gx, gy) = (k % gw, k / gw);
        let horizontal = e.is_multiple_of(2);
        let (a, b) = if horizontal { (val(gx, gy), val(gx + 1, gy)) } else { (val(gx, gy), val(gx, gy + 1)) };
        // Am Bildrand liegt die Kontur auf der Pixelkante
        let t = if a.is_finite() && b.is_finite() { (a / (a - b)) as f64 } else { 0.5 };
        if horizontal {
            Point::new(gx as f64 - 0.5 + t, gy as f64 - 0.5)
        } else {
            Point::new(gx as f64 - 0.5, gy as f64 - 0.5 + t)
        }
    };
    let mut loops = Vec::new();
    for start in order {
        let mut pts = Vec::new();
        let mut e = start;
        while let Some(n) = next.remove(&e) {
            pts.push(point(e));
            e = n;
        }
        if !pts.is_empty() {
            loops.push(pts);
        }
    }
    loops
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
    let fit = fit_options(opts);
    let mut result = Vec::new();
    for (pts, is_closed) in open.iter().map(|p| (p, false)).chain(closed.iter().map(|p| (p, true))) {
        if pts.len() < 2 || polyline_length(pts, is_closed) < 1.5 {
            continue;
        }
        let path = vl_core::fit::fit_points(pts, is_closed, &fit);
        if !path.is_empty() {
            result.push(path);
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
/// bilden zusammen einen Knoten (Mittelpunkt der Gruppe), damit Kreuzungen nicht in viele Kleinststücke zerfallen –
/// bei kompakten Gruppen. Verschmelzen in dichten Schraffuren viele Striche zu einem Klumpen, bleibt er ein Knoten,
/// jede Linie endet aber an ihrem eigenen Anschlusspixel – sonst liefen alle sternförmig zu seinem Mittelpunkt.
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
    // Gruppen benachbarter Verzweigungspixel; nur kompakte haben einen gemeinsamen Mittelpunkt
    const MAX_GROUP: usize = 4;
    let mut group = vec![usize::MAX; n];
    let mut group_center: Vec<Option<Point>> = Vec::new();
    let mut members = Vec::new();
    for start in 0..n {
        if degree[start] < 3 || group[start] != usize::MAX {
            continue;
        }
        let id = group_center.len();
        let mut stack = vec![start];
        group[start] = id;
        members.clear();
        while let Some(i) = stack.pop() {
            members.push(i);
            for j in neighbors(i) {
                if degree[j] >= 3 && group[j] == usize::MAX {
                    group[j] = id;
                    stack.push(j);
                }
            }
        }
        let span = members.iter().fold(vl_core::Rect::EMPTY, |mut r, &i| {
            r.include(center(i));
            r
        });
        let compact = members.len() <= MAX_GROUP && span.width() <= 2.0 && span.height() <= 2.0;
        let sum = members.iter().fold(Point::ZERO, |s, &i| s + center(i));
        group_center.push(compact.then(|| sum * (1.0 / members.len() as f64)));
    }
    let node_key = |i: usize| if degree[i] >= 3 { n + group[i] } else { i };
    let node_point = |i: usize| if degree[i] >= 3 { group_center[group[i]].unwrap_or(center(i)) } else { center(i) };
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
/// werden zu einem Knoten in ihrer Mitte zusammengezogen. Jeder Knoten höchstens einmal: sonst wanderte er in
/// dichten Schraffuren von Brücke zu Brücke und zöge weit entfernte Linienenden zu einem Stern zusammen.
fn collapse_bridges(strokes: &mut Vec<Stroke>, max_len: f64) {
    let mut merged: HashSet<usize> = HashSet::new();
    loop {
        let count = end_counts(strokes);
        let Some(k) = strokes.iter().position(|s| {
            s.a != s.b
                && !merged.contains(&s.a)
                && !merged.contains(&s.b)
                && count[&s.a] >= 3
                && count[&s.b] >= 3
                && polyline_length(&s.pts, false) < max_len
        }) else {
            break;
        };
        let bridge = strokes.remove(k);
        merged.extend([bridge.a, bridge.b]);
        let (p, q) = (bridge.pts[0], bridge.pts[bridge.pts.len() - 1]);
        let mid = p.lerp(q, 0.5);
        // Nur Enden an der Brücke rücken in ihre Mitte; an einem großen Knoten enden Linien auch weiter entfernt
        let near = |e: Point| e.distance(p) <= max_len || e.distance(q) <= max_len;
        for s in strokes.iter_mut() {
            if s.a == bridge.a || s.a == bridge.b {
                s.a = bridge.a;
                if near(s.pts[0]) {
                    s.pts[0] = mid;
                }
            }
            if s.b == bridge.a || s.b == bridge.b {
                s.b = bridge.a;
                let last = s.pts.len() - 1;
                if near(s.pts[last]) {
                    s.pts[last] = mid;
                }
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

    /// Feine Schraffur aus 1 px breiten Strichen (Strichzeichnung): alle Konturen bleiben im Bild, auch geglättet.
    #[test]
    fn thin_hatching_stays_inside_the_image() {
        let img = image(90, 70, |x, y| {
            (y % 5 == 0 && x > 3) || (x % 7 == 0 && y > 10) || (x as i64 - 2 * y as i64).rem_euclid(15) < 2
        });
        for opts in
            [TraceOptions::default(), TraceOptions { smoothing: 1.0, tolerance: 0.3, ..TraceOptions::default() }]
        {
            let paths = trace(&img, &ImageAdjust::default(), &opts);
            assert!(!paths.is_empty());
            for poly in paths.iter().flat_map(|p| p.flatten(&Affine::IDENTITY, 0.01)) {
                for q in &poly.points {
                    assert!(q.x >= -0.5 && q.x <= 90.5 && q.y >= -0.5 && q.y <= 70.5, "{q:?}");
                }
            }
        }
    }

    /// Mittellinien in dichter Kreuzschraffur (wie Fell- oder Schattenstriche): jede Linie verläuft auf dem Motiv,
    /// keine Sterne aus Linien, die zu einem weit entfernten Knoten gezogen werden.
    #[test]
    fn centerlines_stay_on_dense_hatching() {
        let (w, h) = (120u32, 90u32);
        let on = |x: u32, y: u32| {
            let (x, y) = (x as i64, y as i64);
            (x > 5 && x < 115 && y > 5 && y < 85)
                && ((x + y).rem_euclid(5) < 2 || (x - 2 * y).rem_euclid(6) < 2 || (3 * x + y).rem_euclid(9) < 2)
        };
        let img = image(w, h, on);
        let opts = TraceOptions { centerline: true, ..TraceOptions::default() };
        let paths = trace(&img, &ImageAdjust::default(), &opts);
        assert!(!paths.is_empty());
        let near_ink = |p: Point| {
            let (cx, cy) = (p.x.floor() as i64, p.y.floor() as i64);
            (-2..=2).any(|dy| {
                (-2..=2).any(|dx| {
                    let (x, y) = (cx + dx, cy + dy);
                    x >= 0 && y >= 0 && x < w as i64 && y < h as i64 && on(x as u32, y as u32)
                })
            })
        };
        for poly in paths.iter().flat_map(|p| p.flatten(&Affine::IDENTITY, 0.01)) {
            let n = poly.points.len();
            let segs = if poly.closed { n } else { n - 1 };
            for i in 0..segs {
                let (a, b) = (poly.points[i], poly.points[(i + 1) % n]);
                let steps = (a.distance(b).ceil() as usize).max(1);
                for k in 0..=steps {
                    let q = a.lerp(b, k as f64 / steps as f64);
                    assert!(near_ink(q), "Linie abseits des Motivs bei {q:?} ({a:?} → {b:?})");
                }
            }
        }
    }

    /// Weiche Kanten (kantengeglättet, vergrößert): die Kontur folgt dem Grauverlauf zwischen den Pixeln statt
    /// der Pixeltreppe, und „Glätten“ macht aus dem Vieleck wenige Kurven.
    #[test]
    fn soft_edges_are_traced_between_pixels() {
        // Kreis Radius 12.3 px, Randpixel nach Flächenanteil grau (4×4 Unterabtastung)
        let (c, r) = (Point::new(20.0, 20.0), 12.3);
        let mut px = Vec::new();
        for y in 0..40 {
            for x in 0..40 {
                let inside = (0..16)
                    .filter(|k| {
                        let q = Point::new(
                            x as f64 + (k % 4) as f64 / 4.0 + 0.125,
                            y as f64 + (k / 4) as f64 / 4.0 + 0.125,
                        );
                        q.distance(c) < r
                    })
                    .count();
                px.push((255.0 * (1.0 - inside as f64 / 16.0)).round() as u8);
            }
        }
        let img = RasterImage::new(40, 40, px);
        let opts = TraceOptions { tolerance: 0.1, ..TraceOptions::default() };
        let paths = trace(&img, &ImageAdjust::default(), &opts);
        assert_eq!(paths.len(), 1);
        let poly = &paths[0].flatten(&Affine::IDENTITY, 0.01)[0];
        for q in &poly.points {
            assert!((q.distance(c) - r).abs() < 0.12, "{q:?}: {}", q.distance(c));
        }
        // geglättet: wenige Kurven; ohne Glätten: nur gerade Linien
        let segs = &paths[0].subpaths[0].segments;
        assert!(segs.len() <= 12 && segs.iter().all(|s| matches!(s, vl_core::Segment::Cubic(..))), "{}", segs.len());
        let flat = trace(&img, &ImageAdjust::default(), &TraceOptions { smoothing: 0.0, ..opts });
        assert!(flat[0].subpaths[0].segments.iter().all(|s| matches!(s, vl_core::Segment::Line(..))));
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
