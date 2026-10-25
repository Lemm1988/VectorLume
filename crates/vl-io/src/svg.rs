//! SVG-Import über `usvg` (Transformationen, Einheiten, Formen und Text werden aufgelöst) und SVG-Export.

use crate::{Imported, ImportedItem, layer_for_color};
use std::sync::Arc;
use usvg::tiny_skia_path::PathSegment;
use vl_core::{Affine, Document, ImageAdjust, ImageObject, PathBuilder, Point, RasterImage, Shape, VectorPath};

/// usvg rechnet in CSS-Pixeln (96 dpi).
const PX_TO_MM: f64 = 25.4 / 96.0;
/// Größter Zahlenbetrag in Geometrie-Angaben. usvg braucht bei riesigen Koordinaten extrem lange (bei 1e28 Sekunden,
/// bei 1e38 Minuten); solche Werte stehen nur in beschädigten Dateien.
const MAX_NUMBER: f64 = 1e15;
/// Attribute mit Koordinaten, Größen und Transformationen.
const GEOMETRY: [&str; 18] = [
    "d",
    "points",
    "x",
    "y",
    "x1",
    "y1",
    "x2",
    "y2",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "width",
    "height",
    "viewBox",
    "transform",
    "style",
];

pub fn import_svg(data: &[u8], fonts: Option<Arc<usvg::fontdb::Database>>) -> Result<Imported, String> {
    let mut opt = usvg::Options::default();
    if let Some(db) = fonts {
        opt.fontdb = db;
    }
    if let Some(v) = absurd_number(data) {
        return Err(format!("SVG enthält eine unsinnig große Zahl ({v:e}) – die Datei ist vermutlich beschädigt."));
    }
    let tree = usvg::Tree::from_data(data, &opt).map_err(|e| format!("SVG konnte nicht gelesen werden: {e}"))?;
    let mut out = Imported::default();
    let mut skipped_images = 0;
    collect_group(tree.root(), &mut out, &mut skipped_images);
    if skipped_images > 0 {
        out.warnings.push(format!("{skipped_images} eingebettete(s) Bild(er) im SVG wurden übersprungen."));
    }
    if out.items.is_empty() {
        out.warnings.push("Im SVG wurden keine Vektorpfade gefunden.".into());
    }
    Ok(out)
}

/// Erste Zahl in einem Geometrie-Attribut, deren Betrag [`MAX_NUMBER`] übersteigt (komprimierte oder nicht lesbare
/// Dateien werden nicht geprüft, das übernimmt usvg).
fn absurd_number(data: &[u8]) -> Option<f64> {
    let text = std::str::from_utf8(data).ok()?;
    let opt = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(text, opt).ok()?;
    doc.descendants()
        .flat_map(|n| n.attributes())
        .filter(|a| GEOMETRY.contains(&a.name()))
        .find_map(|a| numbers(a.value()).find(|v| v.abs() > MAX_NUMBER))
}

/// Alle Zahlen eines Attributwerts (auch ohne Trennzeichen wie in `M10-5.5.5`).
fn numbers(s: &str) -> impl Iterator<Item = f64> + '_ {
    let b = s.as_bytes();
    let mut i = 0;
    std::iter::from_fn(move || {
        while i < b.len() {
            let start = i;
            if b[i] == b'-' || b[i] == b'+' {
                i += 1;
            }
            let digits = |i: &mut usize| {
                let from = *i;
                while *i < b.len() && b[*i].is_ascii_digit() {
                    *i += 1;
                }
                *i > from
            };
            let mut any = digits(&mut i);
            if i < b.len() && b[i] == b'.' {
                i += 1;
                any |= digits(&mut i);
            }
            if !any {
                i = start + 1;
                continue;
            }
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                let mut j = i + 1;
                if j < b.len() && (b[j] == b'-' || b[j] == b'+') {
                    j += 1;
                }
                if digits(&mut j) {
                    i = j;
                }
            }
            return s[start..i].parse().ok();
        }
        None
    })
}

fn collect_group(group: &usvg::Group, out: &mut Imported, skipped_images: &mut usize) {
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => collect_group(g, out, skipped_images),
            usvg::Node::Path(p) => {
                if !p.is_visible() {
                    continue;
                }
                let path = convert_path(p.data(), p.abs_transform());
                if path.is_empty() {
                    continue;
                }
                let color = p
                    .stroke()
                    .and_then(|s| paint_color(s.paint()))
                    .or_else(|| p.fill().and_then(|f| paint_color(f.paint())));
                let name = if p.id().is_empty() { "SVG-Pfad".to_string() } else { p.id().to_string() };
                out.items.push(ImportedItem::Path { name, layer: color.and_then(layer_for_color), path });
            }
            usvg::Node::Text(t) => collect_group(t.flattened(), out, skipped_images),
            usvg::Node::Image(img) => match convert_image(img) {
                Some(item) if img.is_visible() => out.items.push(item),
                Some(_) => {}
                None => *skipped_images += 1,
            },
        }
    }
}

/// Eingebettetes Rasterbild (PNG, JPEG, GIF, WebP) mit seiner Lage; verschachtelte SVG-Bilder werden übersprungen.
fn convert_image(img: &usvg::Image) -> Option<ImportedItem> {
    let data = match img.kind() {
        usvg::ImageKind::PNG(d) | usvg::ImageKind::JPEG(d) | usvg::ImageKind::GIF(d) | usvg::ImageKind::WEBP(d) => d,
        usvg::ImageKind::SVG(_) => return None,
    };
    let raster = RasterImage::from_encoded(data).ok()?;
    let size = img.size();
    let (w, h) = (size.width() as f64, size.height() as f64);
    // Bild-mm → Bildpixel → SVG-Pixel (absolute Transformation) → mm
    let t = img.abs_transform();
    let transform = Affine::scale(1.0 / PX_TO_MM, 1.0 / PX_TO_MM)
        .then(&Affine([t.sx as f64, t.ky as f64, t.kx as f64, t.sy as f64, t.tx as f64, t.ty as f64]))
        .then(&Affine::scale(PX_TO_MM, PX_TO_MM));
    let name = if img.id().is_empty() { "SVG-Bild".to_string() } else { img.id().to_string() };
    Some(ImportedItem::Image {
        name,
        image: ImageObject {
            image: Arc::new(raster),
            width_mm: w * PX_TO_MM,
            height_mm: h * PX_TO_MM,
            adjust: ImageAdjust::default(),
            clip: Vec::new(),
        },
        transform,
    })
}

fn paint_color(p: &usvg::Paint) -> Option<[u8; 3]> {
    match p {
        usvg::Paint::Color(c) => Some([c.red, c.green, c.blue]),
        _ => None,
    }
}

fn convert_path(data: &usvg::tiny_skia_path::Path, t: usvg::Transform) -> VectorPath {
    // Pixel → mm, danach die absolute Transformation des Elements.
    let tf = Affine([t.sx as f64, t.ky as f64, t.kx as f64, t.sy as f64, t.tx as f64, t.ty as f64])
        .then(&Affine::scale(PX_TO_MM, PX_TO_MM));
    let pt = |p: usvg::tiny_skia_path::Point| tf.apply(Point::new(p.x as f64, p.y as f64));
    let mut pb = PathBuilder::new();
    for seg in data.segments() {
        match seg {
            PathSegment::MoveTo(p) => pb.move_to(pt(p)),
            PathSegment::LineTo(p) => pb.line_to(pt(p)),
            PathSegment::QuadTo(c, p) => pb.quad_to(pt(c), pt(p)),
            PathSegment::CubicTo(c1, c2, p) => pb.cubic_to(pt(c1), pt(c2), pt(p)),
            PathSegment::Close => pb.close(),
        }
    }
    pb.finish()
}

/// Exportiert alle Objekte als SVG in Millimetern. Bilder werden samt Bildanpassung und Zuschnitt als PNG
/// eingebettet, damit sich die Datei z. B. als Druckvorlage für „Drucken & Schneiden“ eignet.
pub fn export_svg(doc: &Document, width_mm: f64, height_mm: f64) -> String {
    let mut s = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"{w}mm\" height=\"{h}mm\" viewBox=\"0 0 {w} {h}\">\n",
        w = width_mm,
        h = height_mm
    );
    for o in &doc.objects {
        if let Shape::Image(img) = &o.shape {
            s.push_str(&image_element(o.id, &o.name, img, &o.transform));
            continue;
        }
        let [r, g, b] = doc.layers[o.layer].color;
        let mut d = String::new();
        match &o.shape {
            Shape::Path(p) => {
                for sp in &p.transformed(&o.transform).subpaths {
                    d.push_str(&format!("M{:.4} {:.4}", sp.start.x, sp.start.y));
                    for seg in &sp.segments {
                        match *seg {
                            vl_core::Segment::Line(p) => d.push_str(&format!(" L{:.4} {:.4}", p.x, p.y)),
                            vl_core::Segment::Quad(c, p) => {
                                d.push_str(&format!(" Q{:.4} {:.4} {:.4} {:.4}", c.x, c.y, p.x, p.y))
                            }
                            vl_core::Segment::Cubic(c1, c2, p) => d.push_str(&format!(
                                " C{:.4} {:.4} {:.4} {:.4} {:.4} {:.4}",
                                c1.x, c1.y, c2.x, c2.y, p.x, p.y
                            )),
                        }
                    }
                    if sp.closed {
                        d.push_str(" Z");
                    }
                }
            }
            Shape::Image(_) => continue,
        }
        s.push_str(&format!(
            "  <path d=\"{d}\" fill=\"none\" stroke=\"rgb({r},{g},{b})\" stroke-width=\"0.1\"><title>{}</title></path>\n",
            xml_escape(&o.name)
        ));
    }
    s.push_str("</svg>\n");
    s
}

/// `<image>` mit eingebettetem PNG; Bildanpassungen sind eingerechnet, ein Zuschnitt wird zum `clipPath`.
fn image_element(id: u64, name: &str, img: &ImageObject, t: &Affine) -> String {
    use base64::Engine as _;
    let src = &img.image;
    let adjusted;
    let out_img: &RasterImage = if img.adjust.is_identity() {
        src
    } else {
        adjusted = img.adjust.adjusted_image(src, img.width_mm * t.mean_scale() / src.width.max(1) as f64);
        &adjusted
    };
    let Ok(png) = out_img.to_png() else { return String::new() };
    let data = base64::engine::general_purpose::STANDARD.encode(png);
    let mut out = String::new();
    let mut clip_attr = String::new();
    if !img.clip.is_empty() {
        let mut d = String::new();
        for ring in &img.clip {
            for (i, p) in ring.iter().enumerate() {
                d.push_str(&format!("{}{:.4} {:.4} ", if i == 0 { "M" } else { "L" }, p.x, p.y));
            }
            d.push('Z');
        }
        out.push_str(&format!("  <clipPath id=\"zuschnitt{id}\"><path d=\"{d}\" clip-rule=\"evenodd\"/></clipPath>\n"));
        clip_attr = format!(" clip-path=\"url(#zuschnitt{id})\"");
    }
    let [a, b, c, dd, e, f] = t.0;
    out.push_str(&format!(
        "  <image x=\"0\" y=\"0\" width=\"{w:.4}\" height=\"{h:.4}\" preserveAspectRatio=\"none\" \
         transform=\"matrix({a:.6} {b:.6} {c:.6} {dd:.6} {e:.4} {f:.4})\"{clip_attr} \
         xlink:href=\"data:image/png;base64,{data}\"><title>{}</title></image>\n",
        xml_escape(name),
        w = img.width_mm,
        h = img.height_mm,
    ));
    out
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_mm_units_and_colors() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="50mm" viewBox="0 0 100 50">
            <rect x="10" y="10" width="20" height="10" fill="none" stroke="#ff0000"/>
            <g transform="translate(50,0)"><circle cx="10" cy="25" r="5" fill="#000"/></g>
        </svg>"##;
        let imp = import_svg(svg, None).unwrap();
        assert_eq!(imp.items.len(), 2);
        let ImportedItem::Path { path, layer, .. } = &imp.items[0] else { panic!() };
        let b = path.bounds(&Affine::IDENTITY);
        assert!((b.min.x - 10.0).abs() < 1e-3 && (b.width() - 20.0).abs() < 1e-3, "{b:?}");
        assert_eq!(*layer, Some(1)); // Rot
        let ImportedItem::Path { path, layer, .. } = &imp.items[1] else { panic!() };
        assert!((path.bounds(&Affine::IDENTITY).center().x - 60.0).abs() < 1e-3);
        assert_eq!(*layer, None); // Schwarz -> aktuelle Ebene
    }

    #[test]
    fn rejects_absurd_numbers() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm">
            <path d="M0 0 Q 5 1e30 10 0" stroke="black"/></svg>"#;
        assert!(import_svg(svg, None).unwrap_err().contains("beschädigt"));
        let found: Vec<f64> = numbers("M10-5.5.5e2 L1E+3,2e").collect();
        assert_eq!(found, [10.0, -5.5, 50.0, 1000.0, 2.0]);
    }

    #[test]
    fn export_roundtrip() {
        let mut doc = Document::new();
        doc.add_path("Rechteck", 0, VectorPath::rect(5.0, 5.0, 10.0, 10.0, 2.0));
        let svg = export_svg(&doc, 400.0, 400.0);
        let imp = import_svg(svg.as_bytes(), None).unwrap();
        let ImportedItem::Path { path, .. } = &imp.items[0] else { panic!() };
        let b = path.bounds(&Affine::IDENTITY);
        assert!((b.min.x - 5.0).abs() < 1e-3 && (b.max.y - 15.0).abs() < 1e-3);
    }

    #[test]
    fn images_are_embedded_and_read_back() {
        let mut doc = Document::new();
        let image = ImageObject {
            image: Arc::new(RasterImage::new(4, 2, vec![0, 50, 100, 150, 200, 250, 255, 255])),
            width_mm: 40.0,
            height_mm: 20.0,
            adjust: ImageAdjust { brightness: 10.0, ..Default::default() },
            clip: Vec::new(),
        };
        let id = doc.add_image("Foto", 1, image, Point::new(10.0, 5.0));
        doc.transform_objects(&[id], &Affine::rotate_about(0.5, Point::new(30.0, 15.0)));
        let svg = export_svg(&doc, 400.0, 400.0);
        assert!(svg.contains("data:image/png;base64,"));
        let imp = import_svg(svg.as_bytes(), None).unwrap();
        assert!(imp.warnings.is_empty(), "{:?}", imp.warnings);
        let ImportedItem::Image { image, transform, .. } = &imp.items[0] else { panic!() };
        assert_eq!((image.image.width, image.image.height), (4, 2));
        // Helligkeit ist eingerechnet
        assert_eq!(image.image.pixels[0], 26);
        let orig = doc.get(id).unwrap();
        for p in [Point::ZERO, Point::new(40.0, 0.0), Point::new(40.0, 20.0)] {
            let local = Point::new(p.x / 40.0 * image.width_mm, p.y / 20.0 * image.height_mm);
            let (a, b) = (transform.apply(local), orig.transform.apply(p));
            assert!(a.distance(b) < 1e-3, "{a:?} {b:?}");
        }
    }

    #[test]
    fn clipped_image_exports_clip_path() {
        let mut doc = Document::new();
        let sq = vec![Point::new(1.0, 1.0), Point::new(9.0, 1.0), Point::new(9.0, 9.0), Point::new(1.0, 9.0)];
        let image = ImageObject {
            image: Arc::new(RasterImage::new(1, 1, vec![0])),
            width_mm: 10.0,
            height_mm: 10.0,
            adjust: ImageAdjust::default(),
            clip: vec![sq],
        };
        doc.add_image("Foto", 1, image, Point::ZERO);
        let svg = export_svg(&doc, 100.0, 100.0);
        assert!(svg.contains("<clipPath") && svg.contains("clip-path=\"url(#zuschnitt"));
        // usvg liest die Datei ohne Fehler
        assert!(import_svg(svg.as_bytes(), None).is_ok());
    }
}
