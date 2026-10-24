//! Perspektivische Entzerrung eines Kamerabildes auf die Arbeitsfläche: Homographie aus vier Passpunkten und
//! Abbildung des Bildes in ein Raster in Bettkoordinaten.

use vl_core::{Point, Rect};

/// Projektive Abbildung `(x, y) → ((a·x + b·y + c) / (g·x + h·y + 1), (d·x + e·y + f) / (g·x + h·y + 1))`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Homography(pub [[f64; 3]; 3]);

impl Homography {
    /// Aus vier Punktpaaren `from[i] → to[i]`. `None`, wenn drei Punkte auf einer Linie liegen o. Ä.
    pub fn from_points(from: &[Point; 4], to: &[Point; 4]) -> Option<Self> {
        let mut m = [[0.0f64; 9]; 8];
        for i in 0..4 {
            let (u, v) = (from[i].x, from[i].y);
            let (x, y) = (to[i].x, to[i].y);
            m[2 * i] = [u, v, 1.0, 0.0, 0.0, 0.0, -u * x, -v * x, x];
            m[2 * i + 1] = [0.0, 0.0, 0.0, u, v, 1.0, -u * y, -v * y, y];
        }
        let h = solve8(m)?;
        let hm = Homography([[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]]);
        // Gültig nur, wenn alle Passpunkte auf derselben Seite der Fluchtlinie liegen (keine Spiegelung durch ∞)
        let w: Vec<f64> = from.iter().map(|p| hm.0[2][0] * p.x + hm.0[2][1] * p.y + 1.0).collect();
        (w.iter().all(|w| *w > 1e-9) || w.iter().all(|w| *w < -1e-9)).then_some(hm)
    }

    pub fn apply(&self, p: Point) -> Option<Point> {
        let m = &self.0;
        let w = m[2][0] * p.x + m[2][1] * p.y + m[2][2];
        if w.abs() < 1e-12 {
            return None;
        }
        Some(Point::new((m[0][0] * p.x + m[0][1] * p.y + m[0][2]) / w, (m[1][0] * p.x + m[1][1] * p.y + m[1][2]) / w))
    }

    pub fn inverse(&self) -> Option<Self> {
        let m = &self.0;
        let c = |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
        let inv = [
            [c(1, 1, 2, 2), -c(0, 1, 2, 2), c(0, 1, 1, 2)],
            [-c(1, 0, 2, 2), c(0, 0, 2, 2), -c(0, 0, 1, 2)],
            [c(1, 0, 2, 1), -c(0, 0, 2, 1), c(0, 0, 1, 1)],
        ];
        let det = m[0][0] * inv[0][0] + m[0][1] * inv[1][0] + m[0][2] * inv[2][0];
        if det.abs() < 1e-15 || !det.is_finite() {
            return None;
        }
        Some(Homography(inv.map(|row| row.map(|v| v / det))))
    }
}

/// Löst ein 8×8-Gleichungssystem (erweiterte Matrix) mit Spaltenpivotsuche.
fn solve8(mut m: [[f64; 9]; 8]) -> Option<[f64; 8]> {
    for col in 0..8 {
        let pivot = (col..8).max_by(|&a, &b| m[a][col].abs().total_cmp(&m[b][col].abs()))?;
        if m[pivot][col].abs() < 1e-12 {
            return None;
        }
        m.swap(col, pivot);
        let p = m[col];
        for (row, r) in m.iter_mut().enumerate() {
            let f = r[col] / p[col];
            if row != col && f != 0.0 {
                for (v, pv) in r.iter_mut().zip(p.iter()).skip(col) {
                    *v -= f * pv;
                }
            }
        }
    }
    let mut x = [0.0; 8];
    for i in 0..8 {
        x[i] = m[i][8] / m[i][i];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// Ergebnis der Entzerrung: RGBA-Raster, das genau `area` abdeckt (Zeile 0 = `area.min.y`).
pub struct WarpedImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Bildet ein RGB-Bild (`width`×`height`) auf den Bereich `area` ab. `to_image` rechnet Bettkoordinaten in
/// Bildkoordinaten (0…1) um; außerhalb des Bildes bleibt das Ergebnis durchsichtig. `max_px` begrenzt die
/// längere Seite des Ergebnisses.
pub fn warp_to_area(
    rgb: &[u8],
    width: usize,
    height: usize,
    to_image: &Homography,
    area: Rect,
    max_px: usize,
) -> WarpedImage {
    assert_eq!(rgb.len(), width * height * 3, "RGB-Daten passen nicht zur Größe");
    let scale = max_px as f64 / area.width().max(area.height()).max(1e-9);
    let (ow, oh) =
        (((area.width() * scale).round() as usize).max(1), ((area.height() * scale).round() as usize).max(1));
    let mut out = vec![0u8; ow * oh * 4];
    let (sx, sy) = (area.width() / ow as f64, area.height() / oh as f64);
    for j in 0..oh {
        for i in 0..ow {
            let bed = Point::new(area.min.x + (i as f64 + 0.5) * sx, area.min.y + (j as f64 + 0.5) * sy);
            let Some(uv) = to_image.apply(bed) else { continue };
            let (fx, fy) = (uv.x * width as f64 - 0.5, uv.y * height as f64 - 0.5);
            if fx < -0.5 || fy < -0.5 || fx > width as f64 - 0.5 || fy > height as f64 - 0.5 {
                continue;
            }
            let (x0, y0) = (fx.floor().max(0.0) as usize, fy.floor().max(0.0) as usize);
            let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
            let (tx, ty) = ((fx - x0 as f64).clamp(0.0, 1.0), (fy - y0 as f64).clamp(0.0, 1.0));
            let px = |x: usize, y: usize, c: usize| rgb[(y * width + x) * 3 + c] as f64;
            let o = (j * ow + i) * 4;
            for c in 0..3 {
                let top = px(x0, y0, c) * (1.0 - tx) + px(x1, y0, c) * tx;
                let bottom = px(x0, y1, c) * (1.0 - tx) + px(x1, y1, c) * tx;
                out[o + c] = (top * (1.0 - ty) + bottom * ty).round() as u8;
            }
            out[o + 3] = 255;
        }
    }
    WarpedImage { width: ow, height: oh, rgba: out }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Point, b: Point) -> bool {
        a.distance(b) < 1e-6
    }

    #[test]
    fn maps_points_and_inverts() {
        // schräg aufgenommenes Bett: Bildpunkte (0…1) → Bettkoordinaten (mm)
        let img = [Point::new(0.1, 0.2), Point::new(0.9, 0.15), Point::new(0.8, 0.9), Point::new(0.2, 0.85)];
        let bed = [Point::new(80.0, 80.0), Point::new(320.0, 80.0), Point::new(320.0, 320.0), Point::new(80.0, 320.0)];
        let h = Homography::from_points(&img, &bed).unwrap();
        for i in 0..4 {
            assert!(close(h.apply(img[i]).unwrap(), bed[i]));
        }
        let inv = h.inverse().unwrap();
        for i in 0..4 {
            assert!(close(inv.apply(bed[i]).unwrap(), img[i]));
        }
        // Gerade bleiben gerade: Mitte der Bettkante liegt auf der Bildkante
        let mid = inv.apply(Point::new(200.0, 80.0)).unwrap();
        let t = (mid.x - img[0].x) / (img[1].x - img[0].x);
        assert!((img[0].y + t * (img[1].y - img[0].y) - mid.y).abs() < 1e-9);
        // drei Punkte auf einer Linie
        let line = [Point::new(0.0, 0.0), Point::new(0.5, 0.5), Point::new(1.0, 1.0), Point::new(0.0, 1.0)];
        assert!(Homography::from_points(&line, &bed).is_none());
    }

    #[test]
    fn warps_onto_area() {
        // 2×2-Bild: links oben rot, sonst schwarz; Bild deckt das Bett 0…100 genau ab
        let mut rgb = vec![0u8; 12];
        rgb[0] = 255;
        let img = [Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(1.0, 1.0), Point::new(0.0, 1.0)];
        let bed = [Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0)];
        let to_image = Homography::from_points(&bed, &img).unwrap();
        let w = warp_to_area(&rgb, 2, 2, &to_image, Rect::from_xywh(0.0, 0.0, 200.0, 100.0), 20);
        assert_eq!((w.width, w.height), (20, 10));
        let at = |i: usize, j: usize| &w.rgba[(j * w.width + i) * 4..(j * w.width + i) * 4 + 4];
        assert_eq!(at(0, 0), &[255, 0, 0, 255]);
        assert_eq!(at(9, 9)[0], 0);
        // rechts außerhalb des Bildes: durchsichtig
        assert_eq!(at(15, 5)[3], 0);
    }
}
