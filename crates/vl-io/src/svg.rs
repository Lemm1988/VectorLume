//! SVG-Import über `usvg` (Transformationen, Einheiten, Formen und Text werden aufgelöst) und SVG-Export.

use crate::{Imported, ImportedItem, layer_for_color};
use std::sync::Arc;
use usvg::tiny_skia_path::PathSegment;
use vl_core::{Affine, Document, PathBuilder, Point, Shape, VectorPath};

/// usvg rechnet in CSS-Pixeln (96 dpi).
const PX_TO_MM: f64 = 25.4 / 96.0;

pub fn import_svg(data: &[u8], fonts: Option<Arc<usvg::fontdb::Database>>) -> Result<Imported, String> {
    let mut opt = usvg::Options::default();
    if let Some(db) = fonts {
        opt.fontdb = db;
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
            usvg::Node::Image(_) => *skipped_images += 1,
        }
    }
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

/// Exportiert alle Vektorobjekte als SVG in Millimetern (Bilder werden als Umriss ausgegeben).
pub fn export_svg(doc: &Document, width_mm: f64, height_mm: f64) -> String {
    let mut s = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}mm\" height=\"{h}mm\" viewBox=\"0 0 {w} {h}\">\n",
        w = width_mm,
        h = height_mm
    );
    for o in &doc.objects {
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
            Shape::Image(_) => {
                for poly in o.polylines(0.01) {
                    for (i, p) in poly.points.iter().enumerate() {
                        d.push_str(&format!("{}{:.4} {:.4} ", if i == 0 { "M" } else { "L" }, p.x, p.y));
                    }
                    d.push('Z');
                }
            }
        }
        s.push_str(&format!(
            "  <path d=\"{d}\" fill=\"none\" stroke=\"rgb({r},{g},{b})\" stroke-width=\"0.1\"><title>{}</title></path>\n",
            xml_escape(&o.name)
        ));
    }
    s.push_str("</svg>\n");
    s
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
    fn export_roundtrip() {
        let mut doc = Document::new();
        doc.add_path("Rechteck", 0, VectorPath::rect(5.0, 5.0, 10.0, 10.0, 2.0));
        let svg = export_svg(&doc, 400.0, 400.0);
        let imp = import_svg(svg.as_bytes(), None).unwrap();
        let ImportedItem::Path { path, .. } = &imp.items[0] else { panic!() };
        let b = path.bounds(&Affine::IDENTITY);
        assert!((b.min.x - 5.0).abs() < 1e-3 && (b.max.y - 15.0).abs() < 1e-3);
    }
}
