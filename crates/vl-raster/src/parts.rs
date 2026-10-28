//! Teile eines Bildes erkennen (z. B. einzelne Aufkleber oder Magnete auf einem Bogen), damit nachgezeichnete
//! Konturen je Teil gruppiert werden können – auch wenn die Schneidlinie eines Teils Lücken hat und Nachbarteile
//! sich fast berühren.
//!
//! 1. Lücken schließen: das Motiv um `gap` Pixel wachsen lassen und eingeschlossene Flächen füllen. Danach ist
//!    jedes Teil mit geschlossenem Umriss eine volle Fläche; berühren sich zwei Teile beinahe, hängen ihre Flächen
//!    an einer Engstelle zusammen.
//! 2. Tiefe: Abstand jedes Flächenpixels zum Rand der Fläche. Jedes Teil hat innen eine hohe Tiefe, eine
//!    Engstelle zwischen zwei Teilen eine geringe.
//! 3. Fluten von innen nach außen: Bereiche wachsen mit sinkender Tiefe. Treffen zwei aufeinander, ist die
//!    aktuelle Tiefe die Engstelle zwischen ihnen. Sie werden nur vereinigt, wenn die Engstelle mindestens
//!    `NECK` mal so tief ist wie der flachere der beiden – so bleiben sich berührende Teile getrennt, während
//!    Einschnürungen innerhalb eines Teils (Taille, Hals) und die vielen kleinen Spitzen der Tiefe zusammenfinden.
//! 4. Ein Teil braucht einen Kern von `core` Pixeln Tiefe. Flache Stücke ohne Kern (z. B. eine Schneidlinie mit
//!    Lücken, deren Inneres deshalb nicht gefüllt wurde) gehören zum nächsten Teil in Reichweite `core`.
//! 5. Je Kontur entscheidet die Mehrheit ihrer Punkte. Umschließt eine Kontur aber ein anderes Teil zu mehr als
//!    der Hälfte (`WRAP`), gehört sie zu diesem – so bleibt die Schneidlinie bei ihrem Aufkleber, auch wenn sie
//!    das Nachbarteil berührt.

use vl_core::{Affine, Point, Polyline, VectorPath};

/// Engstelle / Tiefe des flacheren Bereichs, ab der zwei Bereiche ein Teil sind.
const NECK: f64 = 0.4;
/// Winkel, den eine Kontur um die Mitte eines Teils herumreichen muss, um zu ihm zu gehören.
const WRAP: f64 = std::f64::consts::PI;

/// Teil-Nummer je Pfad (Pixelkoordinaten), 0 = keines. `ink` ist die Maske des Motivs.
pub fn path_parts(paths: &[VectorPath], ink: &[bool], w: usize, h: usize) -> Vec<u32> {
    let labels = part_labels(ink, w, h);
    let count = labels.iter().copied().max().unwrap_or(0) as usize;
    let mut sum = vec![(0.0, 0.0, 0usize); count + 1];
    for (i, &l) in labels.iter().enumerate().filter(|(_, l)| **l != 0) {
        let s = &mut sum[l as usize];
        s.0 += (i % w) as f64 + 0.5;
        s.1 += (i / w) as f64 + 0.5;
        s.2 += 1;
    }
    let centers: Vec<Point> =
        sum.iter().map(|&(x, y, n)| Point::new(x / n.max(1) as f64, y / n.max(1) as f64)).collect();
    paths
        .iter()
        .map(|p| {
            let polys = p.flatten(&Affine::IDENTITY, 0.5);
            let mut votes = vec![0usize; count + 1];
            for pt in polys.iter().flat_map(|l| &l.points) {
                votes[label_at(&labels, w, h, pt.x, pt.y) as usize] += 1;
            }
            let vote = (1..=count).filter(|&l| votes[l] > 0).max_by_key(|&l| votes[l]).unwrap_or(0);
            // Nur Teile, deren Mitte im Rahmen der Kontur liegt, kann sie zu mehr als der Hälfte umschließen
            let bounds = p.bounds(&Affine::IDENTITY);
            let wrapped = (1..=count)
                .filter(|&l| l != vote && bounds.contains(centers[l]))
                .map(|l| (l, coverage(&polys, centers[l])))
                .max_by(|a, b| a.1.total_cmp(&b.1));
            match wrapped {
                Some((l, c)) if c >= WRAP && (vote == 0 || coverage(&polys, centers[vote]) < WRAP) => l as u32,
                _ => vote as u32,
            }
        })
        .collect()
}

/// Teil-Nummer je Pixel (0 = Hintergrund oder keinem Teil zugeordnet).
fn part_labels(ink: &[bool], w: usize, h: usize) -> Vec<u32> {
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let diag = ((w * w + h * h) as f64).sqrt();
    let gap = (diag / 900.0).max(2.0);
    // Abstände in Drittelpixeln (Chamfer 3/4)
    let core = 3.0 * (diag / 150.0).clamp(4.0, 40.0);
    let area = {
        let (near, _) = distance(&seeds(ink), w, h);
        fill_holes(&near.iter().map(|&d| d as f64 <= 3.0 * gap).collect::<Vec<_>>(), w, h)
    };
    let (mut depth, _) = distance(&seeds(&area.iter().map(|&a| !a).collect::<Vec<_>>()), w, h);
    // Außerhalb des Bildes ist Hintergrund (wie beim Nachzeichnen)
    for (i, d) in depth.iter_mut().enumerate() {
        let (x, y) = (i % w, i / w);
        *d = (*d).min(3 * (x.min(y).min(w - 1 - x).min(h - 1 - y) as u32 + 1));
    }

    // Pixel nach Tiefe einsortieren (tiefste zuerst)
    let max = (0..w * h).filter(|&i| area[i]).map(|i| depth[i]).max().unwrap_or(0) as usize;
    let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); max + 1];
    for i in (0..w * h).filter(|&i| area[i]) {
        buckets[depth[i] as usize].push(i as u32);
    }
    // Bereiche als Union-Find mit der größten Tiefe je Bereich
    let mut region = vec![u32::MAX; w * h];
    let mut parent: Vec<u32> = Vec::new();
    let mut peak: Vec<u32> = Vec::new();
    fn find(parent: &mut [u32], mut r: u32) -> u32 {
        while parent[r as usize] != r {
            parent[r as usize] = parent[parent[r as usize] as usize];
            r = parent[r as usize];
        }
        r
    }
    let mut roots: Vec<u32> = Vec::with_capacity(8);
    for d in (0..=max).rev() {
        for &i in &buckets[d] {
            let i = i as usize;
            roots.clear();
            for j in neighbors8(i, w, h) {
                if region[j] != u32::MAX {
                    let r = find(&mut parent, region[j]);
                    if !roots.contains(&r) {
                        roots.push(r);
                    }
                }
            }
            if roots.is_empty() {
                region[i] = parent.len() as u32;
                parent.push(parent.len() as u32);
                peak.push(d as u32);
                continue;
            }
            roots.sort_by_key(|&r| std::cmp::Reverse(peak[r as usize]));
            region[i] = roots[0];
            for k in 1..roots.len() {
                let (a, b) = (find(&mut parent, roots[0]), find(&mut parent, roots[k]));
                if a != b && d as f64 >= NECK * peak[a as usize].min(peak[b as usize]) as f64 {
                    let (hi, lo) = if peak[a as usize] >= peak[b as usize] { (a, b) } else { (b, a) };
                    parent[lo as usize] = hi;
                }
            }
        }
    }
    // Bereiche mit Kern fortlaufend nummerieren
    let mut number = vec![0u32; parent.len()];
    let mut count = 0;
    let mut labels: Vec<u32> = (0..w * h)
        .map(|i| {
            if region[i] == u32::MAX {
                return 0;
            }
            let r = find(&mut parent, region[i]) as usize;
            if (peak[r] as f64) < core {
                return 0;
            }
            if number[r] == 0 {
                count += 1;
                number[r] = count;
            }
            number[r]
        })
        .collect();
    drop((depth, buckets, region));
    // Flache Stücke zum nächsten Teil in Reichweite
    let (dist, nearest) = distance(&labels, w, h);
    for i in 0..w * h {
        if area[i] && labels[i] == 0 && dist[i] as f64 <= core {
            labels[i] = nearest[i];
        }
    }
    labels
}

/// Teil-Nummer an einer Stelle (Pixelkoordinaten), notfalls aus der Nachbarschaft; 0 = keines.
fn label_at(labels: &[u32], w: usize, h: usize, x: f64, y: f64) -> u32 {
    if labels.is_empty() {
        return 0;
    }
    let (cx, cy) = (x.floor() as i64, y.floor() as i64);
    for r in 0..=2i64 {
        for dy in -r..=r {
            for dx in -r..=r {
                let (px, py) = (cx + dx, cy + dy);
                if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                    let l = labels[py as usize * w + px as usize];
                    if l != 0 {
                        return l;
                    }
                }
            }
        }
    }
    0
}

/// Winkel (Bogenmaß, 0 … 2π), den die Polylinien von `c` aus gesehen überdecken.
fn coverage(polys: &[Polyline], c: Point) -> f64 {
    use std::f64::consts::{PI, TAU};
    let mut spans: Vec<(f64, f64)> = Vec::new();
    for l in polys {
        let n = l.points.len();
        let segments = if l.closed { n } else { n.saturating_sub(1) };
        for i in 0..segments {
            let (a, b) = (l.points[i], l.points[(i + 1) % n]);
            let (aa, ab) = ((a.y - c.y).atan2(a.x - c.x), (b.y - c.y).atan2(b.x - c.x));
            let d = (ab - aa + PI).rem_euclid(TAU) - PI;
            let start = (if d >= 0.0 { aa } else { ab }).rem_euclid(TAU);
            let end = start + d.abs();
            if end > TAU {
                spans.push((start, TAU));
                spans.push((0.0, end - TAU));
            } else {
                spans.push((start, end));
            }
        }
    }
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut total = 0.0;
    let mut current: Option<(f64, f64)> = None;
    for (s, e) in spans {
        current = match current {
            Some((cs, ce)) if s <= ce => Some((cs, ce.max(e))),
            Some((cs, ce)) => {
                total += ce - cs;
                Some((s, e))
            }
            None => Some((s, e)),
        };
    }
    total + current.map_or(0.0, |(cs, ce)| ce - cs)
}

fn neighbors8(i: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    let (x, y) = ((i % w) as i64, (i / w) as i64);
    (-1i64..=1)
        .flat_map(move |dy| (-1i64..=1).map(move |dx| (x + dx, y + dy)))
        .filter(move |&(nx, ny)| nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h && (nx, ny) != (x, y))
        .map(move |(nx, ny)| ny as usize * w + nx as usize)
}

fn seeds(mask: &[bool]) -> Vec<u32> {
    mask.iter().map(|&m| m as u32).collect()
}

/// Chamfer-Abstand (3 gerade, 4 schräg ≈ 3 · euklidisch) jedes Pixels zum nächsten Pixel mit `seed` ≠ 0, dazu
/// dessen Wert.
fn distance(seed: &[u32], w: usize, h: usize) -> (Vec<u32>, Vec<u32>) {
    let inf = u32::MAX / 2;
    let mut d: Vec<u32> = seed.iter().map(|&s| if s != 0 { 0 } else { inf }).collect();
    let mut v = seed.to_vec();
    let relax = |d: &mut [u32], v: &mut [u32], i: usize, j: usize, step: u32| {
        if d[j] + step < d[i] {
            d[i] = d[j] + step;
            v[i] = v[j];
        }
    };
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if x > 0 {
                relax(&mut d, &mut v, i, i - 1, 3);
            }
            if y > 0 {
                relax(&mut d, &mut v, i, i - w, 3);
                if x > 0 {
                    relax(&mut d, &mut v, i, i - w - 1, 4);
                }
                if x + 1 < w {
                    relax(&mut d, &mut v, i, i - w + 1, 4);
                }
            }
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            if x + 1 < w {
                relax(&mut d, &mut v, i, i + 1, 3);
            }
            if y + 1 < h {
                relax(&mut d, &mut v, i, i + w, 3);
                if x + 1 < w {
                    relax(&mut d, &mut v, i, i + w + 1, 4);
                }
                if x > 0 {
                    relax(&mut d, &mut v, i, i + w - 1, 4);
                }
            }
        }
    }
    (d, v)
}

/// Füllt alles, was nicht über Hintergrund (4er-Nachbarschaft) mit dem Bildrand verbunden ist.
fn fill_holes(fg: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut outside = vec![false; w * h];
    let border = (0..w).flat_map(|x| [x, (h - 1) * w + x]).chain((0..h).flat_map(|y| [y * w, y * w + w - 1]));
    let mut stack: Vec<usize> = border.filter(|&i| !fg[i]).collect();
    for &i in &stack {
        outside[i] = true;
    }
    while let Some(i) = stack.pop() {
        let (x, y) = (i % w, i / w);
        let next =
            [(x + 1 < w).then(|| i + 1), (x > 0).then(|| i - 1), (y + 1 < h).then(|| i + w), (y > 0).then(|| i - w)];
        for j in next.into_iter().flatten() {
            if !fg[j] && !outside[j] {
                outside[j] = true;
                stack.push(j);
            }
        }
    }
    outside.into_iter().map(|o| !o).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::{TraceOptions, trace};
    use vl_core::RasterImage;

    fn mask(w: usize, h: usize, ink: impl Fn(f64, f64) -> bool) -> Vec<bool> {
        (0..w * h).map(|i| ink((i % w) as f64 + 0.5, (i / w) as f64 + 0.5)).collect()
    }

    /// Zwei Aufkleber 4 px nebeneinander: dünner Außenrand mit Lücken, darin ein Motiv, das den Rand nicht
    /// berührt. Jeder wird ein eigenes Teil mit seinem Inhalt, obwohl sich die Ränder fast berühren.
    #[test]
    fn stickers_with_gaps_stay_separate_parts() {
        let (w, h) = (260usize, 140usize);
        let ink = mask(w, h, |x, y| {
            let sticker = |cx: f64| {
                let d = (x - cx).hypot(y - 70.0);
                let angle = (y - 70.0).atan2(x - cx).to_degrees();
                // Rand 2 px breit mit zwei Lücken (oben und unten), dazu ein Punkt und ein Strich innen
                let ring = (58.0..60.0).contains(&d) && !(82.0..84.0).contains(&angle.abs());
                ring || (x - cx).hypot(y - 60.0) < 12.0 || ((x - cx).abs() < 25.0 && (y - 95.0).abs() < 2.0)
            };
            sticker(68.0) || sticker(192.0)
        });
        let labels = part_labels(&ink, w, h);
        let at = |x: f64, y: f64| label_at(&labels, w, h, x, y);
        let left = at(68.0 - 59.0, 70.0);
        let right = at(192.0 + 59.0, 70.0);
        assert!(left != 0 && right != 0 && left != right);
        // Inhalt gehört zum eigenen Aufkleber
        assert_eq!(at(68.0, 60.0), left);
        assert_eq!(at(68.0, 95.0), left);
        assert_eq!(at(192.0, 60.0), right);
        assert_eq!(at(192.0, 95.0), right);
        // auch die Randstücke an der engsten Stelle
        assert_eq!(at(68.0 + 59.0, 70.0), left);
        assert_eq!(at(192.0 - 59.0, 70.0), right);
    }

    /// Ganz schwarzes Bild: ein Teil, keine Überläufe.
    #[test]
    fn full_image_is_one_part() {
        let labels = part_labels(&[true; 40 * 30], 40, 30);
        assert!(labels.iter().all(|&l| l == 1));
        assert!(part_labels(&[], 0, 0).is_empty());
    }

    /// Schneidlinien mit Lücken, deren Inneres deshalb offen bleibt: Die eine berührt das Nachbarteil, die
    /// andere ist in zwei Bögen zerfallen. Beide gehören zu dem Teil, das sie umschließen; ein entfernter
    /// Fleck gehört zu keinem.
    #[test]
    fn broken_cut_lines_belong_to_the_part_they_surround() {
        let (w, h) = (420usize, 160usize);
        let ink = mask(w, h, |x, y| {
            let ring = |cx: f64, gaps: &[(f64, f64)]| {
                let d = (x - cx).hypot(y - 80.0);
                let angle = (y - 80.0).atan2(x - cx).to_degrees();
                (46.0..48.0).contains(&d) && !gaps.iter().any(|g| (g.0..g.1).contains(&angle))
            };
            let body = |cx: f64| (x - cx).hypot(y - 80.0) < 40.0;
            // A: Ring mit einer Lücke oben, rechts 2 px neben B; C: Ring in zwei Bögen
            ring(80.0, &[(-95.0, -85.0)])
                || body(80.0)
                || body(170.0)
                || ring(330.0, &[(-95.0, -85.0), (85.0, 95.0)])
                || body(330.0)
                || ((x - 250.0).abs() < 3.0 && (y - 10.0).abs() < 3.0)
        });
        let pixels = ink.iter().map(|&i| if i { 0 } else { 255 }).collect();
        let img = RasterImage::new(w as u32, h as u32, pixels);
        let paths = trace(&img, &Default::default(), &TraceOptions::default());
        let parts = path_parts(&paths, &ink, w, h);
        // Teile der Konturen, deren Rahmen die Bedingung erfüllt
        let part_of = |keep: &dyn Fn(vl_core::Rect) -> bool| -> Vec<u32> {
            let found: Vec<u32> =
                paths.iter().zip(&parts).filter(|(p, _)| keep(p.bounds(&Affine::IDENTITY))).map(|(_, &n)| n).collect();
            assert!(!found.is_empty());
            found
        };
        let a = part_of(&|r| r.min.x > 39.0 && r.max.x < 121.0 && r.width() > 70.0)[0];
        let b = part_of(&|r| r.min.x > 129.0 && r.max.x < 211.0 && r.width() > 70.0)[0];
        assert!(a != 0 && b != 0 && a != b);
        assert_eq!(part_of(&|r| r.min.x > 31.0 && r.max.x < 129.0 && r.width() > 90.0), vec![a], "Ring von A");
        let c = part_of(&|r| r.min.x > 289.0 && r.max.x < 371.0 && r.width() > 70.0)[0];
        assert!(c != 0 && c != a && c != b);
        let arcs = part_of(&|r| r.min.x > 281.0 && r.max.x < 379.0 && (r.min.x < 289.0 || r.max.x > 371.0));
        assert_eq!(arcs, vec![c, c], "Bögen von C");
        let speck = part_of(&|r| r.min.x > 240.0 && r.max.x < 260.0);
        assert!(speck.len() == 1 && ![a, b, c].contains(&speck[0]), "Fleck");
    }
}
