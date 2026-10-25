//! Speichern und Laden von VectorLume-Projekten (`.vlume`, JSON mit eingebetteten PNG-Bildern).

use crate::document::Document;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const PROJECT_EXTENSION: &str = "vlume";
const FORMAT_ID: &str = "vectorlume-project";
const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct ProjectFile {
    format: String,
    version: u32,
    generator: String,
    document: Document,
}

pub fn project_to_string(doc: &Document) -> Result<String, String> {
    let file = ProjectFile {
        format: FORMAT_ID.into(),
        version: FORMAT_VERSION,
        generator: format!("VectorLume {}", env!("CARGO_PKG_VERSION")),
        document: doc.clone(),
    };
    serde_json::to_string(&file).map_err(|e| format!("Projekt konnte nicht serialisiert werden: {e}"))
}

pub fn project_from_str(text: &str) -> Result<Document, String> {
    let file: ProjectFile =
        serde_json::from_str(text).map_err(|e| format!("Projektdatei ist beschädigt oder ungültig: {e}"))?;
    if file.format != FORMAT_ID {
        return Err("Die Datei ist kein VectorLume-Projekt.".into());
    }
    if file.version > FORMAT_VERSION {
        return Err(format!(
            "Das Projekt wurde mit einer neueren Version erstellt (Format {}). Bitte VectorLume aktualisieren.",
            file.version
        ));
    }
    let mut doc = file.document;
    doc.repair();
    Ok(doc)
}

pub fn save_project(doc: &Document, path: &Path) -> Result<(), String> {
    let text = project_to_string(doc)?;
    // Erst in eine temporäre Datei schreiben, damit ein Absturz das alte Projekt nicht zerstört.
    let tmp = path.with_extension("vlume.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("Speichern fehlgeschlagen: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Speichern fehlgeschlagen: {e}"))
}

pub fn load_project(path: &Path) -> Result<Document, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("Datei konnte nicht gelesen werden: {e}"))?;
    project_from_str(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{ImageAdjust, ImageObject, RasterImage, Shape};
    use crate::geom::Point;
    use crate::path::VectorPath;
    use std::sync::Arc;

    #[test]
    fn roundtrip_with_image() {
        let mut doc = Document::new();
        doc.add_path("Kreis", 2, VectorPath::ellipse(Point::new(10.0, 10.0), 5.0, 5.0));
        let img = RasterImage::new(3, 2, vec![0, 50, 100, 150, 200, 255]);
        doc.add_image(
            "Bild",
            4,
            ImageObject {
                image: Arc::new(img.clone()),
                width_mm: 3.0,
                height_mm: 2.0,
                adjust: ImageAdjust { brightness: 10.0, ..Default::default() },
                clip: Vec::new(),
            },
            Point::new(1.0, 1.0),
        );
        doc.layers[2].settings.speed = 1234.0;
        let text = project_to_string(&doc).unwrap();
        // volle Leistung ist Standard und wird nicht gespeichert
        assert!(!text.contains("power_scale"));
        doc.objects[0].power_scale = 40.0;
        let text = project_to_string(&doc).unwrap();
        let back = project_from_str(&text).unwrap();
        assert_eq!(back, doc);
        match &back.objects[1].shape {
            Shape::Image(i) => assert_eq!(*i.image, img),
            _ => panic!("Bild erwartet"),
        }
    }

    #[test]
    fn rejects_foreign_json() {
        assert!(project_from_str(r#"{"format":"x","version":1,"generator":"","document":{}}"#).is_err());
    }
}
