//! Export als PDF (eine Seite, maßstabsgetreu 1:1): Linien-Ebenen als Konturen, Füll- und Bild-Ebenen als
//! gefüllte Flächen in der Ebenenfarbe, Bilder mit Bildanpassung und Zuschnitt. Ausgeblendete Ebenen fehlen.

use lopdf::{Dictionary, Object, Stream, dictionary};
use std::fmt::Write as _;
use vl_core::{Affine, Document, ImageObject, LayerMode, Point, Rect, Segment, Shape, VectorPath};

/// mm in PDF-Punkte (1/72 Zoll).
const MM_TO_PT: f64 = 72.0 / 25.4;
/// Strichstärke der Konturen in mm.
const LINE_WIDTH: f64 = 0.1;

/// Erzeugt die PDF-Datei; `page` ist der abgebildete Ausschnitt in Dokument-mm (z. B. Entwurf plus Rand).
pub fn export_pdf(doc: &Document, page: Rect, title: &str) -> Result<Vec<u8>, String> {
    if page.is_empty() || page.width() <= 0.0 || page.height() <= 0.0 {
        return Err("Die Seite ist leer.".into());
    }
    let mut pdf = lopdf::Document::with_version("1.5");
    let pages_id = pdf.new_object_id();
    let mut images = Dictionary::new();
    let k = MM_TO_PT;
    // Dokument (Y nach unten, mm) → PDF (Y nach oben, pt)
    let mut c =
        format!("q {k:.6} 0 0 {:.6} {:.4} {:.4} cm {LINE_WIDTH} w 1 J 1 j\n", -k, -page.min.x * k, page.max.y * k);
    for o in &doc.objects {
        let Some(layer) = doc.layers.get(o.layer) else { continue };
        if !layer.visible {
            continue;
        }
        let [r, g, b] = layer.color.map(|v| v as f64 / 255.0);
        match &o.shape {
            Shape::Image(img) => {
                let name = format!("Im{}", images.len());
                images.set(name.as_bytes(), pdf.add_object(image_stream(img, &o.transform)?));
                image_ops(&mut c, img, &o.transform, &name);
            }
            Shape::Path(p) => {
                let p = p.transformed(&o.transform);
                // Unterebenen zählen mit: z. B. erst füllen, dann Kontur schneiden
                let modes = || std::iter::once(&layer.settings).chain(&layer.sub).map(|s| s.mode);
                let filled = modes().any(|m| m != LayerMode::Line);
                let outlined = modes().any(|m| m == LayerMode::Line);
                let closed = VectorPath { subpaths: p.subpaths.iter().filter(|s| s.closed).cloned().collect() };
                let open = VectorPath { subpaths: p.subpaths.iter().filter(|s| !s.closed).cloned().collect() };
                if filled && !closed.subpaths.is_empty() {
                    let _ = writeln!(c, "{r:.3} {g:.3} {b:.3} rg");
                    path_ops(&mut c, &closed);
                    c.push_str("f*\n");
                }
                let stroke = if filled && !outlined { open } else { p };
                if !stroke.subpaths.is_empty() {
                    let _ = writeln!(c, "{r:.3} {g:.3} {b:.3} RG");
                    path_ops(&mut c, &stroke);
                    c.push_str("S\n");
                }
            }
        }
    }
    c.push_str("Q\n");

    let mut content = Stream::new(Dictionary::new(), c.into_bytes());
    let _ = content.compress();
    let content_id = pdf.add_object(content);
    let resources_id = pdf.add_object(dictionary! { "XObject" => images });
    let media =
        vec![0.into(), 0.into(), Object::Real((page.width() * k) as f32), Object::Real((page.height() * k) as f32)];
    let page_id = pdf.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Resources" => resources_id,
        "MediaBox" => media,
    });
    pdf.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 }),
    );
    let catalog_id = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    let info_id = pdf.add_object(dictionary! {
        "Title" => Object::string_literal(title),
        "Producer" => Object::string_literal("VectorLume"),
    });
    pdf.trailer.set("Root", catalog_id);
    pdf.trailer.set("Info", info_id);
    let mut out = Vec::new();
    pdf.save_to(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

fn path_ops(c: &mut String, p: &VectorPath) {
    for sp in &p.subpaths {
        let mut cur = sp.start;
        let _ = writeln!(c, "{:.4} {:.4} m", cur.x, cur.y);
        for seg in &sp.segments {
            match *seg {
                Segment::Line(q) => {
                    let _ = writeln!(c, "{:.4} {:.4} l", q.x, q.y);
                }
                Segment::Quad(ctrl, q) => {
                    let c1 = cur + (ctrl - cur) * (2.0 / 3.0);
                    let c2 = q + (ctrl - q) * (2.0 / 3.0);
                    cubic(c, c1, c2, q);
                }
                Segment::Cubic(c1, c2, q) => cubic(c, c1, c2, q),
            }
            cur = seg.end();
        }
        if sp.closed {
            c.push_str("h\n");
        }
    }
}

fn cubic(c: &mut String, c1: Point, c2: Point, p: Point) {
    let _ = writeln!(c, "{:.4} {:.4} {:.4} {:.4} {:.4} {:.4} c", c1.x, c1.y, c2.x, c2.y, p.x, p.y);
}

/// Bild im lokalen Rechteck (0,0)–(Breite, Höhe) des Objekts; der Zuschnitt liegt in denselben Koordinaten.
fn image_ops(c: &mut String, img: &ImageObject, t: &Affine, name: &str) {
    let [a, b, cc, d, e, f] = t.0;
    let _ = writeln!(c, "q {a:.6} {b:.6} {cc:.6} {d:.6} {e:.4} {f:.4} cm");
    if !img.clip.is_empty() {
        for ring in img.clip.iter().filter(|r| r.len() >= 3) {
            for (i, p) in ring.iter().enumerate() {
                let _ = writeln!(c, "{:.4} {:.4} {}", p.x, p.y, if i == 0 { "m" } else { "l" });
            }
            c.push_str("h\n");
        }
        c.push_str("W* n\n");
    }
    // Einheitsquadrat → lokales Rechteck; die erste Bildzeile liegt oben (lokal y = 0)
    let _ =
        writeln!(c, "{w:.4} 0 0 {:.4} 0 {h:.4} cm /{name} Do Q", -img.height_mm, w = img.width_mm, h = img.height_mm);
}

fn image_stream(img: &ImageObject, t: &Affine) -> Result<Stream, String> {
    let src = &img.image;
    if src.width == 0 || src.height == 0 {
        return Err("Leeres Bild.".into());
    }
    let pixels = if img.adjust.is_identity() {
        src.pixels.clone()
    } else {
        img.adjust.adjusted_image(src, img.width_mm * t.mean_scale() / src.width as f64).pixels
    };
    let mut s = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => src.width as i64,
            "Height" => src.height as i64,
            "ColorSpace" => "DeviceGray",
            "BitsPerComponent" => 8,
        },
        pixels,
    );
    let _ = s.compress();
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ImportedItem;
    use std::sync::Arc;
    use vl_core::{ImageAdjust, RasterImage};

    #[test]
    fn pdf_round_trip_keeps_size_and_position() {
        let mut doc = Document::new();
        doc.add_path("Rechteck", 0, VectorPath::rect(20.0, 30.0, 40.0, 25.0, 0.0));
        let img = ImageObject {
            image: Arc::new(RasterImage::new(2, 2, vec![0, 255, 255, 0])),
            width_mm: 10.0,
            height_mm: 10.0,
            adjust: ImageAdjust { sharpen: 50.0, ..Default::default() },
            clip: Vec::new(),
        };
        doc.add_image("Bild", 0, img, Point::new(70.0, 30.0));
        let page = Rect::from_xywh(10.0, 20.0, 100.0, 50.0);
        let data = export_pdf(&doc, page, "Test").unwrap();
        assert!(data.starts_with(b"%PDF-1.5"));
        let imp = crate::pdf::import_pdf(&data).unwrap();
        let paths: Vec<_> = imp
            .items
            .iter()
            .filter_map(|i| match i {
                ImportedItem::Path { path, .. } => Some(path.bounds(&Affine::IDENTITY)),
                _ => None,
            })
            .collect();
        assert_eq!(paths.len(), 1, "Bilder werden beim Import übersprungen");
        let b = paths[0];
        // Lage relativ zur Seite: (20 − 10, 30 − 20), Größe 40 × 25 mm
        assert!((b.min.x - 10.0).abs() < 0.01 && (b.min.y - 10.0).abs() < 0.01, "{b:?}");
        assert!((b.width() - 40.0).abs() < 0.01 && (b.height() - 25.0).abs() < 0.01, "{b:?}");
    }
}
