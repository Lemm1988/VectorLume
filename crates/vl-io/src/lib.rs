//! Import und Export von Dateien für VectorLume.

pub mod barcode;
pub mod csv;
pub mod dxf;
pub mod eps;
pub mod excellon;
pub mod gerber;
pub mod hershey;
pub mod hpgl;
pub mod image;
pub mod lightburn;
pub mod model3d;
pub mod pdf;
pub mod pdf_export;
pub mod svg;
pub mod text;

use std::path::Path;
use std::sync::Arc;
use vl_core::{Affine, Document, ImageObject, LAYER_COUNT, LayerMode, ObjectId, PALETTE, Point, VectorPath};

pub use barcode::{BarcodeInfo, barcode_info};
pub use csv::parse_csv;
pub use lightburn::{LightBurnImport, import_lightburn};
pub use text::{TextAlign, TextSpec, font_families, render_text, system_fonts, text_to_path};

pub const VECTOR_EXTENSIONS: &[&str] = &["svg", "svgz", "dxf", "pdf", "ai", "eps", "plt", "hpgl", "hgl"];
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "bmp", "gif", "webp", "tif", "tiff"];
/// 3D-Modelle, die als Tiefenbild eingefügt werden (eigener Dialog statt direktem Import).
pub const MODEL_EXTENSIONS: &[&str] = &["stl", "obj"];

/// Leiterplatten-Fertigungsdaten (Gerber und Excellon).
pub fn is_pcb_extension(ext: &str) -> bool {
    gerber::GERBER_EXTENSIONS.contains(&ext) || excellon::DRILL_EXTENSIONS.contains(&ext)
}

pub fn pcb_extensions() -> Vec<&'static str> {
    gerber::GERBER_EXTENSIONS.iter().chain(excellon::DRILL_EXTENSIONS).copied().collect()
}

#[derive(Clone, Debug)]
pub enum ImportedItem {
    /// `layer = None` bedeutet: aktuelle Ebene verwenden.
    Path { name: String, layer: Option<usize>, path: VectorPath },
    Image {
        name: String,
        image: ImageObject,
        /// Lage des Bildes (Bildkoordinaten in mm → Dokument).
        transform: Affine,
    },
}

#[derive(Clone, Debug, Default)]
pub struct Imported {
    pub items: Vec<ImportedItem>,
    pub warnings: Vec<String>,
}

pub fn import_file(path: &Path, fonts: Option<Arc<usvg::fontdb::Database>>) -> Result<Imported, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let data = std::fs::read(path).map_err(|e| format!("Datei konnte nicht gelesen werden: {e}"))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("Import").to_string();
    match ext.as_str() {
        "svg" | "svgz" => svg::import_svg(&data, fonts),
        "dxf" => dxf::import_dxf(&data),
        "pdf" | "ai" => pdf::import_pdf(&data),
        "eps" | "epsf" => eps::import_eps(&data),
        "plt" | "hpgl" | "hgl" => hpgl::import_hpgl(&data),
        e if IMAGE_EXTENSIONS.contains(&e) => image::import_image(&data, &name),
        e if gerber::GERBER_EXTENSIONS.contains(&e) => {
            gerber::import_gerber(&data, &name, gerber::PROFILE_EXTENSIONS.contains(&e))
        }
        e if excellon::DRILL_EXTENSIONS.contains(&e) => excellon::import_excellon(&data, &name),
        _ => Err(format!("Dateiformat „.{ext}“ wird nicht unterstützt.")),
    }
}

/// Ordnet eine Farbe der ähnlichsten Palettenfarbe zu (Vergleich über Farbton, Sättigung, Helligkeit).
/// Schwarz/Weiß/Grau → `None` (aktuelle Ebene).
pub fn layer_for_color(rgb: [u8; 3]) -> Option<usize> {
    let (h, s, v) = hsv(rgb);
    if s < 0.15 || v < 0.15 {
        return None;
    }
    (0..LAYER_COUNT - 1)
        .map(|i| {
            let (ph, ps, pv) = hsv(PALETTE[i]);
            let dh = (h - ph).abs().min(360.0 - (h - ph).abs()) / 180.0;
            (i, 12.0 * dh * dh + 0.5 * (s - ps).powi(2) + (v - pv).powi(2))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

fn hsv(rgb: [u8; 3]) -> (f64, f64, f64) {
    let [r, g, b] = rgb.map(|c| c as f64 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d < 1e-9 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, if max > 0.0 { d / max } else { 0.0 }, max)
}

/// Ebene für importierte Bilder: vorhandene Bildebene oder erste leere Ebene.
pub fn image_layer(doc: &Document) -> usize {
    let used = doc.used_layers();
    used.iter()
        .copied()
        .find(|&l| doc.layers[l].settings.mode == LayerMode::Image)
        .or_else(|| (0..LAYER_COUNT).find(|l| !used.contains(l)))
        .unwrap_or(0)
}

/// Fügt importierte Elemente ein; mehrere Elemente werden gruppiert.
pub fn add_to_document(doc: &mut Document, imported: Imported, default_layer: usize) -> Vec<ObjectId> {
    let mut ids = Vec::new();
    for item in imported.items {
        match item {
            ImportedItem::Path { name, layer, path } => {
                ids.push(doc.add_path(name, layer.unwrap_or(default_layer), path))
            }
            ImportedItem::Image { name, image, transform } => {
                let layer = image_layer(doc);
                let id = doc.add_image(name, layer, image, Point::ZERO);
                if let Some(o) = doc.get_mut(id) {
                    o.transform = transform;
                }
                ids.push(id);
            }
        }
    }
    doc.group(&ids);
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_mapping() {
        assert_eq!(layer_for_color([0, 0, 0]), None);
        assert_eq!(layer_for_color([128, 128, 128]), None);
        assert_eq!(layer_for_color([255, 0, 0]), Some(1));
        assert_eq!(layer_for_color([0, 255, 0]), Some(2));
        assert_eq!(layer_for_color(PALETTE[4]), Some(4));
    }

    #[test]
    fn images_go_to_free_layer() {
        let mut doc = Document::new();
        doc.add_path("a", 0, VectorPath::rect(0.0, 0.0, 1.0, 1.0, 0.0));
        assert_eq!(image_layer(&doc), 1);
    }
}
