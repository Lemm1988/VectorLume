//! DXF-Import (Linien, Polylinien mit Bögen, Kreise, Bögen, Ellipsen, Splines, Blockreferenzen) und DXF-Export.

use crate::{Imported, ImportedItem};
use dxf::entities::{Entity, EntityType};
use std::f64::consts::TAU;
use std::fmt::Write as _;
use vl_core::{Affine, Document, PathBuilder, Point, Shape, VectorPath};

const MAX_INSERT_DEPTH: usize = 8;

pub fn import_dxf(data: &[u8]) -> Result<Imported, String> {
    let mut reader = std::io::Cursor::new(data);
    let drawing = dxf::Drawing::load(&mut reader).map_err(|e| format!("DXF konnte nicht gelesen werden: {e}"))?;
    let unit = unit_to_mm(drawing.header.default_drawing_units);
    let mut out = Imported::default();
    let mut ctx = Ctx { drawing: &drawing, unsupported: 0 };
    // DXF: Y nach oben. Dokument: Y nach unten -> spiegeln.
    let base = Affine::scale(unit, -unit);
    for e in drawing.entities() {
        if e.common.is_in_paper_space {
            continue;
        }
        ctx.entity(e, &base, 0, &mut out);
    }
    if ctx.unsupported > 0 {
        out.warnings.push(format!(
            "{} DXF-Element(e) ohne Geometrie-Unterstützung (z. B. Text, Bemaßung) wurden übersprungen.",
            ctx.unsupported
        ));
    }
    if out.items.is_empty() {
        out.warnings.push("Im DXF wurden keine unterstützten Elemente gefunden.".into());
    }
    Ok(out)
}

fn unit_to_mm(u: dxf::enums::Units) -> f64 {
    use dxf::enums::Units::*;
    match u {
        Inches => 25.4,
        Feet => 304.8,
        Centimeters => 10.0,
        Meters => 1000.0,
        Microns => 0.001,
        Mils => 0.0254,
        Decimeters => 100.0,
        // Ohne Angabe ist Millimeter im Laserbereich die übliche Annahme.
        _ => 1.0,
    }
}

struct Ctx<'a> {
    drawing: &'a dxf::Drawing,
    unsupported: usize,
}

impl Ctx<'_> {
    fn entity(&mut self, e: &Entity, t: &Affine, depth: usize, out: &mut Imported) {
        let p = |x: f64, y: f64| t.apply(Point::new(x, y));
        let mut pb = PathBuilder::new();
        let name;
        match &e.specific {
            EntityType::Line(l) => {
                name = "Linie";
                pb.move_to(p(l.p1.x, l.p1.y));
                pb.line_to(p(l.p2.x, l.p2.y));
            }
            EntityType::Circle(c) => {
                name = "Kreis";
                pb.move_to(p(c.center.x + c.radius, c.center.y));
                arc_through(&mut pb, t, (c.center.x, c.center.y), c.radius, 0.0, TAU);
                pb.close();
            }
            EntityType::Arc(a) => {
                name = "Bogen";
                let start = a.start_angle.to_radians();
                let mut sweep = a.end_angle.to_radians() - start;
                if sweep <= 0.0 {
                    sweep += TAU;
                }
                pb.move_to(p(a.center.x + a.radius * start.cos(), a.center.y + a.radius * start.sin()));
                arc_through(&mut pb, t, (a.center.x, a.center.y), a.radius, start, sweep);
            }
            EntityType::Ellipse(el) => {
                let major = Point::new(el.major_axis.x, el.major_axis.y);
                let rx = major.length();
                let ry = rx * el.minor_axis_ratio;
                let rot = major.y.atan2(major.x);
                let (start, mut end) = (el.start_parameter, el.end_parameter);
                if end <= start {
                    end += TAU;
                }
                // Ellipse in lokalen Koordinaten erzeugen und dann transformieren.
                let local = Affine::rotate(rot).then(&Affine::translate(el.center.x, el.center.y)).then(t);
                let mut eb = PathBuilder::new();
                eb.arc(Point::ZERO, rx, ry, 0.0, start, end - start);
                if (end - start - TAU).abs() < 1e-9 {
                    eb.close();
                }
                push(out, &e.common.layer, "Ellipse", eb.finish().transformed(&local));
                return;
            }
            EntityType::LwPolyline(lw) => {
                name = "Polylinie";
                let verts: Vec<(f64, f64, f64)> = lw.vertices.iter().map(|v| (v.x, v.y, v.bulge)).collect();
                bulge_polyline(&mut pb, t, &verts, lw.is_closed());
            }
            EntityType::Polyline(pl) => {
                name = "Polylinie";
                let verts: Vec<(f64, f64, f64)> =
                    pl.vertices().map(|v| (v.location.x, v.location.y, v.bulge)).collect();
                bulge_polyline(&mut pb, t, &verts, pl.is_closed());
            }
            EntityType::Spline(s) => {
                name = "Spline";
                let pts = spline_points(s);
                if let Some((&first, rest)) = pts.split_first() {
                    pb.move_to(p(first.x, first.y));
                    for q in rest {
                        pb.line_to(p(q.x, q.y));
                    }
                    if s.is_closed() {
                        pb.close();
                    }
                }
            }
            EntityType::Insert(ins) => {
                if depth >= MAX_INSERT_DEPTH {
                    return;
                }
                let Some(block) = self.drawing.blocks().find(|b| b.name == ins.name) else { return };
                let block_t = Affine::translate(-block.base_point.x, -block.base_point.y)
                    .then(&Affine::scale(ins.x_scale_factor, ins.y_scale_factor))
                    .then(&Affine::rotate(ins.rotation.to_radians()));
                let cols = ins.column_count.max(1);
                let rows = ins.row_count.max(1);
                for r in 0..rows {
                    for c in 0..cols {
                        let offset = Point::new(
                            ins.location.x + c as f64 * ins.column_spacing,
                            ins.location.y + r as f64 * ins.row_spacing,
                        );
                        let full = block_t.then(&Affine::translate(offset.x, offset.y)).then(t);
                        for be in &block.entities {
                            self.entity(be, &full, depth + 1, out);
                        }
                    }
                }
                return;
            }
            _ => {
                self.unsupported += 1;
                return;
            }
        }
        push(out, &e.common.layer, name, pb.finish());
    }
}

fn push(out: &mut Imported, dxf_layer: &str, kind: &str, path: VectorPath) {
    if path.is_empty() {
        return;
    }
    let name =
        if dxf_layer.is_empty() || dxf_layer == "0" { kind.to_string() } else { format!("{kind} ({dxf_layer})") };
    out.items.push(ImportedItem::Path { name, layer: None, path });
}

/// Kreisbogen in DXF-Koordinaten an den Pfad anhängen (transformiert, daher als Polylinie aus Kubiken).
fn arc_through(pb: &mut PathBuilder, t: &Affine, center: (f64, f64), r: f64, start: f64, sweep: f64) {
    let mut local = PathBuilder::new();
    local.move_to(Point::new(center.0 + r * start.cos(), center.1 + r * start.sin()));
    local.arc(Point::new(center.0, center.1), r, r, 0.0, start, sweep);
    for sp in local.finish().transformed(t).subpaths {
        for seg in sp.segments {
            match seg {
                vl_core::Segment::Line(p) => pb.line_to(p),
                vl_core::Segment::Quad(c, p) => pb.quad_to(c, p),
                vl_core::Segment::Cubic(a, b, p) => pb.cubic_to(a, b, p),
            }
        }
    }
}

/// Polylinie mit „Bulge“-Werten (Bogensegmente) umwandeln.
fn bulge_polyline(pb: &mut PathBuilder, t: &Affine, verts: &[(f64, f64, f64)], closed: bool) {
    if verts.is_empty() {
        return;
    }
    pb.move_to(t.apply(Point::new(verts[0].0, verts[0].1)));
    let n = verts.len();
    let count = if closed { n } else { n - 1 };
    for i in 0..count {
        let (x0, y0, bulge) = verts[i];
        let (x1, y1, _) = verts[(i + 1) % n];
        if bulge.abs() < 1e-9 {
            pb.line_to(t.apply(Point::new(x1, y1)));
            continue;
        }
        // Bulge = tan(Öffnungswinkel / 4)
        let a = Point::new(x0, y0);
        let b = Point::new(x1, y1);
        let theta = 4.0 * bulge.atan();
        let chord = a.distance(b);
        if chord < 1e-12 {
            continue;
        }
        let r = chord / (2.0 * (theta / 2.0).sin()).abs();
        let mid = a.lerp(b, 0.5);
        let dir = (b - a) * (1.0 / chord);
        let normal = Point::new(-dir.y, dir.x);
        let h = r * (theta / 2.0).cos().abs();
        // Mittelpunkt links der Sehne bei positivem Bulge (gegen den Uhrzeigersinn) und Bogen < 180°.
        let side = if (bulge > 0.0) == (theta.abs() < std::f64::consts::PI) { 1.0 } else { -1.0 };
        let center = mid + normal * (h * side);
        let start = (a.y - center.y).atan2(a.x - center.x);
        arc_through(pb, t, (center.x, center.y), r, start, theta);
    }
    if closed {
        pb.close();
    }
}

/// Wertet einen (ggf. rationalen) B-Spline aus; ohne Kontrollpunkte werden die Stützpunkte verbunden.
fn spline_points(s: &dxf::entities::Spline) -> Vec<Point> {
    let cps: Vec<Point> = s.control_points.iter().map(|p| Point::new(p.x, p.y)).collect();
    let degree = s.degree_of_curve.max(1) as usize;
    let knots = &s.knot_values;
    if cps.len() <= degree || knots.len() != cps.len() + degree + 1 {
        return s.fit_points.iter().map(|p| Point::new(p.x, p.y)).collect();
    }
    let weights: Vec<f64> =
        if s.weight_values.len() == cps.len() { s.weight_values.clone() } else { vec![1.0; cps.len()] };
    let (t0, t1) = (knots[degree], knots[cps.len()]);
    let steps = (cps.len() * 16).clamp(32, 4000);
    (0..=steps)
        .map(|i| {
            let u = t0 + (t1 - t0) * i as f64 / steps as f64;
            de_boor(u, degree, knots, &cps, &weights)
        })
        .collect()
}

fn de_boor(u: f64, p: usize, knots: &[f64], cps: &[Point], w: &[f64]) -> Point {
    let n = cps.len();
    let mut k = p;
    while k < n - 1 && u >= knots[k + 1] {
        k += 1;
    }
    let mut d: Vec<(Point, f64)> = (0..=p)
        .map(|j| {
            let i = j + k - p;
            (cps[i] * w[i], w[i])
        })
        .collect();
    for r in 1..=p {
        for j in (r..=p).rev() {
            let i = j + k - p;
            let denom = knots[i + p + 1 - r] - knots[i];
            let alpha = if denom.abs() < 1e-12 { 0.0 } else { (u - knots[i]) / denom };
            let (a, aw) = d[j - 1];
            let (b, bw) = d[j];
            d[j] = (a * (1.0 - alpha) + b * alpha, aw * (1.0 - alpha) + bw * alpha);
        }
    }
    let (pt, wt) = d[p];
    if wt.abs() < 1e-12 { pt } else { pt * (1.0 / wt) }
}

/// Ergebnis des DXF-Exports.
pub struct DxfExport {
    pub text: String,
    /// Anzahl der Bilder, die nicht exportiert werden konnten.
    pub skipped_images: usize,
}

/// Exportiert alle Vektorobjekte als DXF R12 (mm, Y nach oben, Ursprung unten links der Arbeitsfläche).
/// Kurven werden mit 0,01 mm Toleranz in Polylinien umgewandelt; je VectorLume-Ebene entsteht ein DXF-Layer.
pub fn export_dxf(doc: &Document, bed_height: f64) -> DxfExport {
    let mut t = String::new();
    let pair = |t: &mut String, code: i32, value: &str| {
        let _ = writeln!(
            t,
            "{code}
{value}"
        );
    };
    pair(&mut t, 0, "SECTION");
    pair(&mut t, 2, "HEADER");
    pair(&mut t, 9, "$ACADVER");
    pair(&mut t, 1, "AC1009");
    pair(&mut t, 9, "$INSUNITS");
    pair(&mut t, 70, "4");
    pair(&mut t, 0, "ENDSEC");

    let used = doc.used_layers();
    pair(&mut t, 0, "SECTION");
    pair(&mut t, 2, "TABLES");
    pair(&mut t, 0, "TABLE");
    pair(&mut t, 2, "LAYER");
    pair(&mut t, 70, &used.len().to_string());
    for &l in &used {
        pair(&mut t, 0, "LAYER");
        pair(&mut t, 2, &layer_name(&doc.layers[l].name, l));
        pair(&mut t, 70, "0");
        pair(&mut t, 62, &aci(doc.layers[l].color).to_string());
        pair(&mut t, 6, "CONTINUOUS");
    }
    pair(&mut t, 0, "ENDTAB");
    pair(&mut t, 0, "ENDSEC");

    pair(&mut t, 0, "SECTION");
    pair(&mut t, 2, "ENTITIES");
    let mut skipped_images = 0;
    for o in &doc.objects {
        if matches!(o.shape, Shape::Image(_)) {
            skipped_images += 1;
            continue;
        }
        let layer = layer_name(&doc.layers[o.layer].name, o.layer);
        for poly in o.polylines(0.01) {
            if poly.points.len() < 2 {
                continue;
            }
            pair(&mut t, 0, "POLYLINE");
            pair(&mut t, 8, &layer);
            pair(&mut t, 66, "1");
            pair(&mut t, 70, if poly.closed { "1" } else { "0" });
            for p in &poly.points {
                pair(&mut t, 0, "VERTEX");
                pair(&mut t, 8, &layer);
                pair(&mut t, 10, &format!("{:.4}", p.x));
                pair(&mut t, 20, &format!("{:.4}", bed_height - p.y));
            }
            pair(&mut t, 0, "SEQEND");
            pair(&mut t, 8, &layer);
        }
    }
    pair(&mut t, 0, "ENDSEC");
    pair(&mut t, 0, "EOF");
    DxfExport { text: t, skipped_images }
}

/// DXF-Layernamen dürfen nur bestimmte Zeichen enthalten.
fn layer_name(name: &str, index: usize) -> String {
    let clean: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() || "-_$".contains(c) { c } else { '_' }).collect();
    if clean.is_empty() { format!("L{:02}", index + 1) } else { clean }
}

/// Nächstliegende Farbe der AutoCAD-Grundpalette (1–7, 8, 9).
fn aci(rgb: [u8; 3]) -> u8 {
    const BASE: [(u8, [u8; 3]); 9] = [
        (1, [255, 0, 0]),
        (2, [255, 255, 0]),
        (3, [0, 255, 0]),
        (4, [0, 255, 255]),
        (5, [0, 0, 255]),
        (6, [255, 0, 255]),
        (7, [255, 255, 255]),
        (8, [128, 128, 128]),
        (9, [192, 192, 192]),
    ];
    let dist = |c: [u8; 3]| (0..3).map(|i| (c[i] as i32 - rgb[i] as i32).pow(2)).sum::<i32>();
    BASE.iter().min_by_key(|(_, c)| dist(*c)).map(|(i, _)| *i).unwrap_or(7)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::Affine;

    #[test]
    fn bulge_semicircle() {
        // Halbkreis von (0,0) nach (10,0) mit Bulge 1 -> Radius 5
        let mut pb = PathBuilder::new();
        bulge_polyline(&mut pb, &Affine::IDENTITY, &[(0.0, 0.0, 1.0), (10.0, 0.0, 0.0)], false);
        let polys = pb.finish().flatten(&Affine::IDENTITY, 0.001);
        let center = Point::new(5.0, 0.0);
        for q in &polys[0].points {
            assert!((q.distance(center) - 5.0).abs() < 0.01, "{q:?}");
        }
        // Positiver Bulge läuft gegen den Uhrzeigersinn: Bogen liegt unterhalb (y < 0).
        assert!(polys[0].points.iter().all(|q| q.y <= 1e-9));
    }

    #[test]
    fn spline_degree_one_is_polyline() {
        let cps = [Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0)];
        let knots = [0.0, 0.0, 0.5, 1.0, 1.0];
        let w = [1.0; 3];
        let mid = de_boor(0.5, 1, &knots, &cps, &w);
        assert!(mid.distance(Point::new(10.0, 0.0)) < 1e-9);
        let end = de_boor(1.0, 1, &knots, &cps, &w);
        assert!(end.distance(Point::new(10.0, 10.0)) < 1e-9);
    }

    #[test]
    fn imports_simple_dxf() {
        let mut drawing = dxf::Drawing::new();
        drawing.header.default_drawing_units = dxf::enums::Units::Millimeters;
        let line = dxf::entities::Line::new(dxf::Point::new(0.0, 0.0, 0.0), dxf::Point::new(20.0, 10.0, 0.0));
        drawing.add_entity(Entity::new(EntityType::Line(line)));
        let circle =
            dxf::entities::Circle { center: dxf::Point::new(50.0, 50.0, 0.0), radius: 5.0, ..Default::default() };
        drawing.add_entity(Entity::new(EntityType::Circle(circle)));
        let mut buf = Vec::new();
        drawing.save(&mut buf).unwrap();
        let imp = import_dxf(&buf).unwrap();
        assert_eq!(imp.items.len(), 2);
        let ImportedItem::Path { path, .. } = &imp.items[1] else { panic!() };
        let b = path.bounds(&Affine::IDENTITY);
        assert!((b.width() - 10.0).abs() < 1e-3);
        // Y wird gespiegelt
        assert!((b.center().y + 50.0).abs() < 1e-3);
    }

    #[test]
    fn export_round_trip() {
        let mut doc = Document::new();
        doc.add_path("Kreis", 0, VectorPath::ellipse(Point::new(50.0, 50.0), 10.0, 10.0));
        doc.add_path("Linie", 3, VectorPath::line(Point::new(0.0, 0.0), Point::new(20.0, 5.0)));
        let out = export_dxf(&doc, 400.0);
        assert_eq!(out.skipped_images, 0);
        let back = import_dxf(out.text.as_bytes()).unwrap();
        assert_eq!(back.items.len(), 2, "{:?}", back.warnings);
        // Import spiegelt Y (Dokument: Y nach unten); Abstände bleiben erhalten.
        let mut bounds: Vec<vl_core::Rect> = back
            .items
            .iter()
            .filter_map(|i| match i {
                crate::ImportedItem::Path { path, .. } => Some(path.bounds(&Affine::IDENTITY)),
                _ => None,
            })
            .collect();
        bounds.sort_by(|a, b| b.width().total_cmp(&a.width()));
        assert!((bounds[0].width() - 20.0).abs() < 0.01 && (bounds[1].width() - 20.0).abs() < 0.01);
        assert_eq!(aci([255, 10, 10]), 1);
        assert_eq!(layer_name("Schnitt außen", 0), "Schnitt_au_en");
    }
}
