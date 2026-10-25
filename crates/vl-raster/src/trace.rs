//! Bild nachzeichnen: Schwellwert → Schwarzweiß-Maske → geschlossene Konturen entlang der Pixelkanten,
//! vereinfacht (Douglas-Peucker) und optional zu Bézierkurven geglättet. Ergebnis in Pixelkoordinaten
//! (x nach rechts, y nach unten, Pixel (0,0) belegt 0…1).

use serde::{Deserialize, Serialize};
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
}

impl Default for TraceOptions {
    fn default() -> Self {
        Self { threshold: 128, invert: false, min_area: 4.0, tolerance: 0.8, smoothing: 0.7, corner_angle: 65.0 }
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

fn area(pts: &[Point]) -> f64 {
    let n = pts.len();
    (0..n).map(|i| pts[i].cross(pts[(i + 1) % n])).sum::<f64>() / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::Affine;

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
