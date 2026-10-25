//! „In Bitmap umwandeln“: Formen, Texte und Bilder zu einem Graustufenbild zusammenrechnen.

use crate::{RasterError, bilinear, box_downsample};
use vl_core::{Affine, ImageObject, Object, Point, Polyline, RasterImage, Rect, Shape};

/// Höchstens so viele Pixel im Ergebnis.
pub const MAX_PIXELS: usize = 60_000_000;

/// Unterzeilen je Pixelzeile für geglättete Kanten.
const SUBROWS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderOptions {
    pub dpi: f64,
    /// Geschlossene Formen nur als Kontur statt gefüllt.
    pub outline: bool,
    /// Strichbreite für Konturen und offene Pfade (mm).
    pub line_width: f64,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self { dpi: 254.0, outline: false, line_width: 0.2 }
    }
}

impl RenderOptions {
    pub fn pixel_mm(&self) -> f64 {
        25.4 / self.dpi.clamp(10.0, 2400.0)
    }
}

/// Fläche des Ergebnisses: Umrandung aller Objekte, um eine halbe Strichbreite erweitert.
pub fn render_bounds(objects: &[&Object], opts: &RenderOptions) -> Rect {
    let b = objects.iter().fold(Rect::EMPTY, |r, o| r.union(&o.bounds()));
    if b.is_empty() { b } else { b.expanded(opts.line_width / 2.0) }
}

/// Größe des Ergebnisses in Pixeln.
pub fn render_size(bounds: &Rect, opts: &RenderOptions) -> (usize, usize) {
    let px = opts.pixel_mm();
    ((bounds.width() / px).ceil().max(1.0) as usize, (bounds.height() / px).ceil().max(1.0) as usize)
}

/// Rechnet die Objekte in ein Bild (schwarz = Form); Bilder werden abdunkelnd überlagert. Liefert das Bild und
/// seine Lage im Dokument.
pub fn render_objects(objects: &[&Object], opts: &RenderOptions) -> Result<(RasterImage, Rect), RasterError> {
    let bounds = render_bounds(objects, opts);
    if bounds.is_empty() || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return Err(RasterError::Empty);
    }
    let (cols, rows) = render_size(&bounds, opts);
    if cols.saturating_mul(rows) > MAX_PIXELS {
        return Err(RasterError::TooLarge { cols, rows });
    }
    let px = opts.pixel_mm();
    let origin = bounds.min;
    // Bedeckung durch Formen 0…1 und Helligkeit der Bilder 0…1
    let mut ink = vec![0.0f32; cols * rows];
    let mut light = vec![1.0f32; cols * rows];
    let mut fills: Vec<Vec<Polyline>> = Vec::new();
    let mut strokes: Vec<Polyline> = Vec::new();
    for o in objects {
        match &o.shape {
            Shape::Path(_) => {
                let (closed, open): (Vec<Polyline>, Vec<Polyline>) =
                    o.polylines(px / 4.0).into_iter().partition(|p| p.closed && p.points.len() >= 3);
                strokes.extend(open);
                if opts.outline {
                    strokes.extend(closed);
                } else if !closed.is_empty() {
                    fills.push(closed);
                }
            }
            Shape::Image(img) => composite_image(img, &o.transform, origin, px, cols, rows, &mut light),
        }
    }
    let to_px = |p: Point| Point::new((p.x - origin.x) / px, (p.y - origin.y) / px);
    let fills: Vec<Vec<Vec<Point>>> = fills
        .iter()
        .map(|polys| polys.iter().map(|p| p.points.iter().map(|q| to_px(*q)).collect()).collect())
        .collect();
    fill_coverage(&fills, cols, rows, &mut ink);
    let half = (opts.line_width / px / 2.0).max(0.5);
    for s in &strokes {
        let pts: Vec<Point> = s.points.iter().map(|q| to_px(*q)).collect();
        let n = if s.closed { pts.len() } else { pts.len().saturating_sub(1) };
        for i in 0..n {
            stroke_segment(pts[i], pts[(i + 1) % pts.len()], half, cols, rows, &mut ink);
        }
    }
    let pixels: Vec<u8> =
        ink.iter().zip(&light).map(|(k, l)| ((l.min(1.0 - k.min(1.0))) * 255.0).round() as u8).collect();
    let rect = Rect::from_xywh(origin.x, origin.y, cols as f64 * px, rows as f64 * px);
    Ok((RasterImage::new(cols as u32, rows as u32, pixels), rect))
}

/// Gefüllte Flächen (Gerade-Ungerade je Objekt, Objekte vereinigt) mit Unterzeilen und exakter waagerechter
/// Bedeckung. Koordinaten in Pixeln; die Kanten werden vorab den Pixelzeilen zugeordnet.
fn fill_coverage(objects: &[Vec<Vec<Point>>], cols: usize, rows: usize, ink: &mut [f32]) {
    let mut edges: Vec<(usize, Point, Point)> = Vec::new();
    for (k, rings) in objects.iter().enumerate() {
        for ring in rings {
            for (i, &a) in ring.iter().enumerate() {
                let b = ring[(i + 1) % ring.len()];
                if a.y != b.y {
                    edges.push((k, a, b));
                }
            }
        }
    }
    let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); rows];
    for (i, (_, a, b)) in edges.iter().enumerate() {
        let y0 = a.y.min(b.y).floor().max(0.0) as usize;
        let y1 = (a.y.max(b.y).ceil().max(0.0) as usize).min(rows);
        for bucket in &mut buckets[y0.min(rows)..y1] {
            bucket.push(i as u32);
        }
    }
    let mut hits: Vec<(usize, f64)> = Vec::new();
    let mut spans: Vec<(f64, f64)> = Vec::new();
    let w = 1.0 / SUBROWS as f32;
    for (r, bucket) in buckets.iter().enumerate() {
        if bucket.is_empty() {
            continue;
        }
        let row = &mut ink[r * cols..(r + 1) * cols];
        for s in 0..SUBROWS {
            let y = r as f64 + (s as f64 + 0.5) / SUBROWS as f64;
            hits.clear();
            for &i in bucket {
                let (k, a, b) = edges[i as usize];
                if (a.y <= y) != (b.y <= y) {
                    hits.push((k, a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x)));
                }
            }
            hits.sort_by(|p, q| p.0.cmp(&q.0).then(p.1.total_cmp(&q.1)));
            spans.clear();
            for pair in hits.as_chunks::<2>().0 {
                // je Objekt gerade Anzahl Schnittpunkte, also gehören Paare immer zum selben Objekt
                spans.push((pair[0].1, pair[1].1));
            }
            // Überlappungen verschiedener Objekte vereinigen
            spans.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut cur: Option<(f64, f64)> = None;
            for &(a, b) in &spans {
                match cur {
                    Some((ca, cb)) if a <= cb => cur = Some((ca, cb.max(b))),
                    Some((ca, cb)) => {
                        add_span(row, ca, cb, w);
                        cur = Some((a, b));
                    }
                    None => cur = Some((a, b)),
                }
            }
            if let Some((a, b)) = cur {
                add_span(row, a, b, w);
            }
        }
    }
}

/// Fügt einer Zeile die Bedeckung der Strecke `a…b` (Pixel) mit Gewicht `w` hinzu, Randpixel anteilig.
fn add_span(row: &mut [f32], a: f64, b: f64, w: f32) {
    let n = row.len() as f64;
    let (a, b) = (a.clamp(0.0, n), b.clamp(0.0, n));
    if b <= a {
        return;
    }
    let (first, last) = (a.floor() as usize, (b.ceil() as usize).min(row.len()));
    for (c, v) in row.iter_mut().enumerate().take(last).skip(first) {
        let cover = (b.min(c as f64 + 1.0) - a.max(c as f64)).clamp(0.0, 1.0);
        *v += cover as f32 * w;
    }
}

/// Strich mit halber Breite `half` (Pixel) und weicher Kante.
fn stroke_segment(a: Point, b: Point, half: f64, cols: usize, rows: usize, ink: &mut [f32]) {
    let reach = half + 1.0;
    let x0 = (a.x.min(b.x) - reach).floor().max(0.0) as usize;
    let y0 = (a.y.min(b.y) - reach).floor().max(0.0) as usize;
    let x1 = ((a.x.max(b.x) + reach).ceil().max(0.0) as usize).min(cols);
    let y1 = ((a.y.max(b.y) + reach).ceil().max(0.0) as usize).min(rows);
    for y in y0..y1 {
        for x in x0..x1 {
            let p = Point::new(x as f64 + 0.5, y as f64 + 0.5);
            let d = vl_core::path::distance_to_segment(p, a, b);
            let cover = (half - d + 0.5).clamp(0.0, 1.0) as f32;
            let v = &mut ink[y * cols + x];
            *v = v.max(cover);
        }
    }
}

/// Überlagert ein Bild abdunkelnd (Minimum) auf das Ergebnisraster mit Ursprung `origin` und Pixelgröße `px`.
fn composite_image(
    img: &ImageObject,
    transform: &Affine,
    origin: Point,
    px: f64,
    cols: usize,
    rows: usize,
    light: &mut [f32],
) {
    let src = &img.image;
    if src.width == 0 || src.height == 0 || img.width_mm <= 0.0 || img.height_mm <= 0.0 {
        return;
    }
    let Some(inv) = transform.inverse() else { return };
    let sharpened;
    let src: &RasterImage = if img.adjust.sharpens() {
        let px_mm = img.width_mm * transform.mean_scale() / src.width as f64;
        let mut pixels = src.pixels.clone();
        img.adjust.sharpen_pixels(&mut pixels, src.width as usize, src.height as usize, px_mm);
        sharpened = RasterImage::new(src.width, src.height, pixels);
        &sharpened
    } else {
        &img.image
    };
    let (w, h) = (src.width as f64, src.height as f64);
    let doc_to_px = inv.then(&Affine::scale(w / img.width_mm, h / img.height_mm));
    // stark verkleinerte Bilder vorher mitteln
    let fx = doc_to_px.apply_vector(Point::new(px, 0.0)).length();
    let fy = doc_to_px.apply_vector(Point::new(0.0, px)).length();
    let factor = fx.min(fy).floor().max(1.0) as u32;
    let reduced;
    let (source, scale) = if factor >= 2 {
        reduced = box_downsample(src, factor);
        (&reduced, 1.0 / factor as f64)
    } else {
        (src, 1.0)
    };
    let clip = img.clip_mask(src.width as usize, src.height as usize);
    let b = transform.transform_rect(&img.local_rect());
    let c0 = ((b.min.x - origin.x) / px).floor().max(0.0) as usize;
    let r0 = ((b.min.y - origin.y) / px).floor().max(0.0) as usize;
    let c1 = (((b.max.x - origin.x) / px).ceil().max(0.0) as usize).min(cols);
    let r1 = (((b.max.y - origin.y) / px).ceil().max(0.0) as usize).min(rows);
    for r in r0..r1 {
        for c in c0..c1 {
            let p = Point::new(origin.x + (c as f64 + 0.5) * px, origin.y + (r as f64 + 0.5) * px);
            let q = doc_to_px.apply(p);
            if q.x < 0.0 || q.y < 0.0 || q.x >= w || q.y >= h {
                continue;
            }
            if let Some(m) = &clip
                && !m[q.y as usize * src.width as usize + q.x as usize]
            {
                continue;
            }
            let v = img.adjust.apply(bilinear(source, q.x * scale, q.y * scale));
            let l = &mut light[r * cols + c];
            *l = l.min(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::{Document, ImageAdjust, VectorPath};

    fn at(img: &RasterImage, x: u32, y: u32) -> u8 {
        img.get(x, y)
    }

    #[test]
    fn filled_shapes_and_holes() {
        let mut doc = Document::new();
        // Quadrat 10 mm mit Loch 4 mm
        let mut p = VectorPath::rect(0.0, 0.0, 10.0, 10.0, 0.0);
        p.append(VectorPath::rect(3.0, 3.0, 4.0, 4.0, 0.0));
        let id = doc.add_path("a", 0, p);
        let opts = RenderOptions { dpi: 25.4, outline: false, line_width: 0.0 };
        let objs: Vec<&Object> = vec![doc.get(id).unwrap()];
        let (img, rect) = render_objects(&objs, &opts).unwrap();
        assert_eq!((img.width, img.height), (10, 10));
        assert_eq!(rect.min, Point::ZERO);
        assert_eq!(at(&img, 0, 0), 0);
        assert_eq!(at(&img, 2, 5), 0);
        assert_eq!(at(&img, 5, 5), 255, "Loch bleibt weiß");
        let black = img.pixels.iter().filter(|v| **v == 0).count();
        assert_eq!(black, 100 - 16);
    }

    #[test]
    fn edges_are_antialiased_and_overlaps_union() {
        let mut doc = Document::new();
        let a = doc.add_path("a", 0, VectorPath::rect(0.0, 0.0, 6.5, 4.0, 0.0));
        let b = doc.add_path("b", 0, VectorPath::rect(2.0, 0.0, 4.5, 4.0, 0.0));
        let opts = RenderOptions { dpi: 25.4, outline: false, line_width: 0.0 };
        let objs: Vec<&Object> = vec![doc.get(a).unwrap(), doc.get(b).unwrap()];
        let (img, _) = render_objects(&objs, &opts).unwrap();
        // Überlappung zählt nicht doppelt: innen schwarz, nicht „schwärzer“
        assert_eq!(at(&img, 3, 1), 0);
        // Pixel 6 ist halb bedeckt
        assert!((120..=135).contains(&at(&img, 6, 1)), "{}", at(&img, 6, 1));
    }

    #[test]
    fn outlines_open_paths_and_images() {
        let mut doc = Document::new();
        let line = doc.add_path("l", 0, VectorPath::line(Point::new(0.0, 5.0), Point::new(20.0, 5.0)));
        let image = ImageObject {
            image: std::sync::Arc::new(RasterImage::new(2, 1, vec![0, 255])),
            width_mm: 20.0,
            height_mm: 10.0,
            adjust: ImageAdjust::default(),
            clip: Vec::new(),
        };
        let img = doc.add_image("b", 1, image, Point::new(0.0, 0.0));
        let opts = RenderOptions { dpi: 25.4, outline: true, line_width: 1.0 };
        let objs: Vec<&Object> = vec![doc.get(line).unwrap(), doc.get(img).unwrap()];
        let (out, rect) = render_objects(&objs, &opts).unwrap();
        assert_eq!((out.width, out.height), (21, 11));
        assert!((rect.min.x + 0.5).abs() < 1e-9);
        // rechte Bildhälfte weiß, Linie quer hindurch schwarz
        assert!(at(&out, 18, 2) > 200);
        assert_eq!(at(&out, 18, 5), 0);
        // linke Bildhälfte dunkel
        assert!(at(&out, 3, 2) < 60);
    }
}
