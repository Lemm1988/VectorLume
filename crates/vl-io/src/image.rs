//! Rasterbild-Import (PNG, JPG, BMP, GIF, WEBP, TIFF).

use crate::{Imported, ImportedItem};
use std::sync::Arc;
use vl_core::{ImageAdjust, ImageObject, Point, RasterImage};

/// Standardauflösung, wenn die Datei keine verwertbare Angabe enthält: 254 dpi = 0,1 mm pro Pixel.
const DEFAULT_MM_PER_PX: f64 = 0.1;
/// Längere Bildseite wird beim Import höchstens so groß.
const MAX_IMPORT_SIZE_MM: f64 = 150.0;
/// Größere Bilder werden beim Import verkleinert (Speicher und Rechenzeit).
const MAX_PIXELS: u64 = 25_000_000;

pub fn import_image(data: &[u8], name: &str) -> Result<Imported, String> {
    let mut img = image::load_from_memory(data).map_err(|e| format!("Bild konnte nicht gelesen werden: {e}"))?;
    let mut out = Imported::default();
    let pixels = img.width() as u64 * img.height() as u64;
    if pixels > MAX_PIXELS {
        let f = (MAX_PIXELS as f64 / pixels as f64).sqrt();
        let (w, h) = ((img.width() as f64 * f) as u32, (img.height() as f64 * f) as u32);
        img = img.resize(w.max(1), h.max(1), image::imageops::FilterType::Triangle);
        out.warnings.push(format!("Das Bild wurde auf {w} × {h} Pixel verkleinert."));
    }
    let raster = RasterImage::from_dynamic(&img);
    let (mut w_mm, mut h_mm) = (raster.width as f64 * DEFAULT_MM_PER_PX, raster.height as f64 * DEFAULT_MM_PER_PX);
    let longest = w_mm.max(h_mm);
    if longest > MAX_IMPORT_SIZE_MM {
        let f = MAX_IMPORT_SIZE_MM / longest;
        w_mm *= f;
        h_mm *= f;
    }
    out.items.push(ImportedItem::Image {
        name: name.to_string(),
        image: ImageObject { image: Arc::new(raster), width_mm: w_mm, height_mm: h_mm, adjust: ImageAdjust::default() },
        at: Point::ZERO,
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_png_with_transparency() {
        let mut img = image::RgbaImage::new(20, 10);
        for (x, _, p) in img.enumerate_pixels_mut() {
            *p = if x < 10 { image::Rgba([0, 0, 0, 255]) } else { image::Rgba([0, 0, 0, 0]) };
        }
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        let imp = import_image(buf.get_ref(), "test.png").unwrap();
        let ImportedItem::Image { image, .. } = &imp.items[0] else { panic!() };
        assert_eq!((image.width_mm, image.height_mm), (2.0, 1.0));
        assert_eq!(image.image.get(0, 0), 0);
        // Transparent -> weiß (nicht gravieren)
        assert_eq!(image.image.get(15, 0), 255);
    }
}
