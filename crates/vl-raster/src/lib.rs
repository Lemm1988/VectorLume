//! Bildaufbereitung für die Lasergravur: Abtasten auf das Gravurraster, Bildanpassung und Dithering.
//!
//! Ergebnis ist eine „Intensität“ je Rasterzelle: 0.0 = Laser aus, 1.0 = volle Leistung.

pub mod trace;

use vl_core::{Affine, DitherMode, ImageObject, Point, RasterImage};

/// Obergrenze für die Rastergröße (Schutz vor versehentlich riesigen Jobs).
pub const MAX_CELLS: usize = 120_000_000;

/// Auf das Gravurraster abgetastetes Bild. Zelle (c, r) liegt bei
/// `origin + ((c + 0.5) * dx, (r + 0.5) * dy)` im Dokument.
#[derive(Clone, Debug)]
pub struct RasterResult {
    pub origin: Point,
    pub dx: f64,
    pub dy: f64,
    pub cols: usize,
    pub rows: usize,
    pub intensity: Vec<f32>,
}

impl RasterResult {
    pub fn get(&self, c: usize, r: usize) -> f32 {
        self.intensity[r * self.cols + c]
    }

    /// Vorschaubild: 0 = volle Leistung (schwarz), 255 = aus (weiß).
    pub fn preview_pixels(&self) -> Vec<u8> {
        self.intensity.iter().map(|&v| (255.0 * (1.0 - v.clamp(0.0, 1.0))).round() as u8).collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RasterError {
    Empty,
    TooLarge { cols: usize, rows: usize },
}

impl std::fmt::Display for RasterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RasterError::Empty => write!(f, "Das Bild hat keine Fläche."),
            RasterError::TooLarge { cols, rows } => write!(
                f,
                "Das Bild ergibt {cols} × {rows} Rasterpunkte – bitte Zeilenabstand vergrößern oder Bild verkleinern."
            ),
        }
    }
}

/// Komplette Aufbereitung eines Bildobjekts für die Gravur.
pub fn engrave_image(
    img: &ImageObject,
    transform: &Affine,
    interval: f64,
    dither: DitherMode,
    negative: bool,
) -> Result<RasterResult, RasterError> {
    let sampled = sample(img, transform, interval)?;
    let mut gray = sampled.gray;
    if negative {
        for (g, &m) in gray.iter_mut().zip(&sampled.mask) {
            if m {
                *g = 1.0 - *g;
            }
        }
    }
    let mut intensity = dither_gray(&gray, sampled.cols, sampled.rows, dither);
    for (v, &m) in intensity.iter_mut().zip(&sampled.mask) {
        if !m {
            *v = 0.0;
        }
    }
    Ok(RasterResult {
        origin: sampled.origin,
        dx: sampled.dx,
        dy: sampled.dy,
        cols: sampled.cols,
        rows: sampled.rows,
        intensity,
    })
}

struct Sampled {
    origin: Point,
    dx: f64,
    dy: f64,
    cols: usize,
    rows: usize,
    /// Grauwert 0 (schwarz) … 1 (weiß), Bildanpassung bereits angewendet.
    gray: Vec<f32>,
    mask: Vec<bool>,
}

fn sample(img: &ImageObject, transform: &Affine, interval: f64) -> Result<Sampled, RasterError> {
    let interval = interval.max(0.01);
    let bounds = transform.transform_rect(&img.local_rect());
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 || img.image.width == 0 || img.image.height == 0 {
        return Err(RasterError::Empty);
    }
    let cols = (bounds.width() / interval).ceil().max(1.0) as usize;
    let rows = (bounds.height() / interval).ceil().max(1.0) as usize;
    if cols.saturating_mul(rows) > MAX_CELLS {
        return Err(RasterError::TooLarge { cols, rows });
    }
    let inv = transform.inverse().ok_or(RasterError::Empty)?;
    let src: &RasterImage = &img.image;
    let (w, h) = (src.width as f64, src.height as f64);
    let to_px = Affine::scale(w / img.width_mm, h / img.height_mm);
    let doc_to_px = inv.then(&to_px);

    // Wie viele Quellpixel deckt eine Rasterzelle ab? Bei starker Verkleinerung vorher mitteln.
    let fx = doc_to_px.apply_vector(Point::new(interval, 0.0)).length();
    let fy = doc_to_px.apply_vector(Point::new(0.0, interval)).length();
    let factor = fx.min(fy).floor().max(1.0) as u32;
    let reduced;
    let (source, scale) = if factor >= 2 {
        reduced = box_downsample(src, factor);
        (&reduced, 1.0 / factor as f64)
    } else {
        (src, 1.0)
    };

    let origin = bounds.min;
    let mut gray = vec![1.0f32; cols * rows];
    let mut mask = vec![false; cols * rows];
    for r in 0..rows {
        for c in 0..cols {
            let p = Point::new(origin.x + (c as f64 + 0.5) * interval, origin.y + (r as f64 + 0.5) * interval);
            let q = doc_to_px.apply(p);
            if q.x < 0.0 || q.y < 0.0 || q.x >= w || q.y >= h {
                continue;
            }
            let i = r * cols + c;
            mask[i] = true;
            let v = bilinear(source, q.x * scale, q.y * scale);
            gray[i] = img.adjust.apply(v);
        }
    }
    Ok(Sampled { origin, dx: interval, dy: interval, cols, rows, gray, mask })
}

fn box_downsample(src: &RasterImage, k: u32) -> RasterImage {
    let w = src.width.div_ceil(k);
    let h = src.height.div_ceil(k);
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let (mut sum, mut n) = (0u32, 0u32);
            for yy in (y * k)..((y + 1) * k).min(src.height) {
                for xx in (x * k)..((x + 1) * k).min(src.width) {
                    sum += src.get(xx, yy) as u32;
                    n += 1;
                }
            }
            out.push((sum / n.max(1)) as u8);
        }
    }
    RasterImage::new(w, h, out)
}

/// Bilineare Interpolation; Koordinaten in Pixeln, Pixelmitte bei +0.5.
fn bilinear(img: &RasterImage, x: f64, y: f64) -> f32 {
    let fx = (x - 0.5).max(0.0);
    let fy = (y - 0.5).max(0.0);
    let x0 = (fx.floor() as u32).min(img.width - 1);
    let y0 = (fy.floor() as u32).min(img.height - 1);
    let x1 = (x0 + 1).min(img.width - 1);
    let y1 = (y0 + 1).min(img.height - 1);
    let tx = (fx - x0 as f64).clamp(0.0, 1.0) as f32;
    let ty = (fy - y0 as f64).clamp(0.0, 1.0) as f32;
    let p = |xx, yy| img.get(xx, yy) as f32 / 255.0;
    let top = p(x0, y0) * (1.0 - tx) + p(x1, y0) * tx;
    let bottom = p(x0, y1) * (1.0 - tx) + p(x1, y1) * tx;
    top * (1.0 - ty) + bottom * ty
}

/// Fehlerverteilungs-Kern: (dx, dy, Gewicht), Summe der Gewichte geteilt durch `divisor`.
struct Kernel {
    divisor: f32,
    taps: &'static [(i32, i32, f32)],
}

const FLOYD_STEINBERG: Kernel = Kernel { divisor: 16.0, taps: &[(1, 0, 7.0), (-1, 1, 3.0), (0, 1, 5.0), (1, 1, 1.0)] };
const JARVIS: Kernel = Kernel {
    divisor: 48.0,
    taps: &[
        (1, 0, 7.0),
        (2, 0, 5.0),
        (-2, 1, 3.0),
        (-1, 1, 5.0),
        (0, 1, 7.0),
        (1, 1, 5.0),
        (2, 1, 3.0),
        (-2, 2, 1.0),
        (-1, 2, 3.0),
        (0, 2, 5.0),
        (1, 2, 3.0),
        (2, 2, 1.0),
    ],
};
const STUCKI: Kernel = Kernel {
    divisor: 42.0,
    taps: &[
        (1, 0, 8.0),
        (2, 0, 4.0),
        (-2, 1, 2.0),
        (-1, 1, 4.0),
        (0, 1, 8.0),
        (1, 1, 4.0),
        (2, 1, 2.0),
        (-2, 2, 1.0),
        (-1, 2, 2.0),
        (0, 2, 4.0),
        (1, 2, 2.0),
        (2, 2, 1.0),
    ],
};
const ATKINSON: Kernel =
    Kernel { divisor: 8.0, taps: &[(1, 0, 1.0), (2, 0, 1.0), (-1, 1, 1.0), (0, 1, 1.0), (1, 1, 1.0), (0, 2, 1.0)] };
const BURKES: Kernel = Kernel {
    divisor: 32.0,
    taps: &[(1, 0, 8.0), (2, 0, 4.0), (-2, 1, 2.0), (-1, 1, 4.0), (0, 1, 8.0), (1, 1, 4.0), (2, 1, 2.0)],
};
const SIERRA: Kernel = Kernel {
    divisor: 32.0,
    taps: &[
        (1, 0, 5.0),
        (2, 0, 3.0),
        (-2, 1, 2.0),
        (-1, 1, 4.0),
        (0, 1, 5.0),
        (1, 1, 4.0),
        (2, 1, 2.0),
        (-1, 2, 2.0),
        (0, 2, 3.0),
        (1, 2, 2.0),
    ],
};

/// Wandelt Grauwerte (1 = weiß) in Laser-Intensitäten (1 = volle Leistung) um.
pub fn dither_gray(gray: &[f32], cols: usize, rows: usize, mode: DitherMode) -> Vec<f32> {
    assert_eq!(gray.len(), cols * rows);
    match mode {
        DitherMode::Grayscale => gray.iter().map(|&g| (1.0 - g).clamp(0.0, 1.0)).collect(),
        DitherMode::Threshold => gray.iter().map(|&g| if g < 0.5 { 1.0 } else { 0.0 }).collect(),
        DitherMode::Ordered => {
            let m = bayer8();
            let mut out = vec![0.0; gray.len()];
            for r in 0..rows {
                for c in 0..cols {
                    let t = (m[r % 8][c % 8] as f32 + 0.5) / 64.0;
                    out[r * cols + c] = if gray[r * cols + c] < t { 1.0 } else { 0.0 };
                }
            }
            out
        }
        DitherMode::FloydSteinberg => diffuse(gray, cols, rows, &FLOYD_STEINBERG),
        DitherMode::Jarvis => diffuse(gray, cols, rows, &JARVIS),
        DitherMode::Stucki => diffuse(gray, cols, rows, &STUCKI),
        DitherMode::Atkinson => diffuse(gray, cols, rows, &ATKINSON),
        DitherMode::Burkes => diffuse(gray, cols, rows, &BURKES),
        DitherMode::Sierra => diffuse(gray, cols, rows, &SIERRA),
    }
}

/// Fehlerverteilung mit Schlangenlinien-Abtastung (vermeidet Richtungsartefakte).
fn diffuse(gray: &[f32], cols: usize, rows: usize, k: &Kernel) -> Vec<f32> {
    let mut buf = gray.to_vec();
    let mut out = vec![0.0f32; gray.len()];
    for r in 0..rows {
        let reverse = r % 2 == 1;
        for i in 0..cols {
            let c = if reverse { cols - 1 - i } else { i };
            let idx = r * cols + c;
            let old = buf[idx];
            let new = if old < 0.5 { 0.0 } else { 1.0 };
            out[idx] = 1.0 - new;
            let err = old - new;
            if err == 0.0 {
                continue;
            }
            for &(dx, dy, w) in k.taps {
                let dx = if reverse { -dx } else { dx };
                let (nc, nr) = (c as i64 + dx as i64, r as i64 + dy as i64);
                if nc < 0 || nc >= cols as i64 || nr >= rows as i64 {
                    continue;
                }
                buf[nr as usize * cols + nc as usize] += err * w / k.divisor;
            }
        }
    }
    out
}

fn bayer8() -> [[u8; 8]; 8] {
    let mut m = [[0u8; 8]; 8];
    for (y, row) in m.iter_mut().enumerate() {
        for (x, v) in row.iter_mut().enumerate() {
            // Bit-Verschränkung von x^y und y ergibt die Bayer-Matrix.
            let (mut val, xc) = (0u8, x ^ y);
            for bit in 0..3 {
                val |= (((xc >> bit) & 1) as u8) << (2 * (2 - bit) + 1);
                val |= (((y >> bit) & 1) as u8) << (2 * (2 - bit));
            }
            *v = val;
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use vl_core::ImageAdjust;

    fn gradient(w: u32, h: u32) -> RasterImage {
        let px = (0..h).flat_map(|_| (0..w).map(move |x| (x * 255 / (w - 1)) as u8)).collect();
        RasterImage::new(w, h, px)
    }

    #[test]
    fn bayer_is_permutation() {
        let mut seen = [false; 64];
        for row in bayer8() {
            for v in row {
                assert!(!seen[v as usize]);
                seen[v as usize] = true;
            }
        }
    }

    #[test]
    fn dithering_preserves_mean() {
        let (cols, rows) = (64, 64);
        let gray = vec![0.3f32; cols * rows];
        for mode in DitherMode::ALL {
            let out = dither_gray(&gray, cols, rows, mode);
            let mean: f32 = out.iter().sum::<f32>() / out.len() as f32;
            match mode {
                DitherMode::Threshold => assert_eq!(mean, 1.0),
                // Atkinson verteilt nur 6/8 des Fehlers und darf daher abweichen.
                DitherMode::Atkinson => assert!((mean - 0.7).abs() < 0.15, "{mode:?}: {mean}"),
                _ => assert!((mean - 0.7).abs() < 0.03, "{mode:?}: {mean}"),
            }
        }
    }

    #[test]
    fn engrave_axis_aligned_image() {
        let img = ImageObject {
            image: Arc::new(gradient(100, 10)),
            width_mm: 10.0,
            height_mm: 1.0,
            adjust: ImageAdjust::default(),
        };
        let r = engrave_image(&img, &Affine::translate(5.0, 5.0), 0.1, DitherMode::Grayscale, false).unwrap();
        assert_eq!((r.cols, r.rows), (100, 10));
        assert!((r.origin.x - 5.0).abs() < 1e-9);
        // Links schwarz = volle Leistung, rechts weiß = aus.
        assert!(r.get(0, 0) > 0.95);
        assert!(r.get(99, 0) < 0.05);
        let neg = engrave_image(&img, &Affine::translate(5.0, 5.0), 0.1, DitherMode::Grayscale, true).unwrap();
        assert!(neg.get(0, 0) < 0.05);
    }

    #[test]
    fn downsampling_averages() {
        // Schachbrett 1px, stark verkleinert -> mittleres Grau.
        let (w, h) = (200u32, 200u32);
        let px = (0..h).flat_map(|y| (0..w).map(move |x| if (x + y) % 2 == 0 { 0 } else { 255 })).collect();
        let img = ImageObject {
            image: Arc::new(RasterImage::new(w, h, px)),
            width_mm: 20.0,
            height_mm: 20.0,
            adjust: ImageAdjust::default(),
        };
        let r = engrave_image(&img, &Affine::IDENTITY, 1.0, DitherMode::Grayscale, false).unwrap();
        let mean: f32 = r.intensity.iter().sum::<f32>() / r.intensity.len() as f32;
        assert!((mean - 0.5).abs() < 0.05, "{mean}");
    }

    #[test]
    fn rotated_image_has_masked_corners() {
        let img = ImageObject {
            image: Arc::new(RasterImage::new(10, 10, vec![0; 100])),
            width_mm: 10.0,
            height_mm: 10.0,
            adjust: ImageAdjust::default(),
        };
        let t = Affine::rotate_about(std::f64::consts::FRAC_PI_4, Point::new(5.0, 5.0));
        let r = engrave_image(&img, &t, 0.25, DitherMode::Threshold, false).unwrap();
        assert_eq!(r.get(0, 0), 0.0);
        assert_eq!(r.get(r.cols / 2, r.rows / 2), 1.0);
    }
}
