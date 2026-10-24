//! Import von PDF- und Illustrator-Dateien (AI ab Version 9 ist PDF-kompatibel): Vektorpfade der ersten Seite
//! einschließlich eingebetteter Formular-XObjekte. Farben werden wie beim SVG-Import den Ebenen zugeordnet.

use crate::{Imported, ImportedItem, layer_for_color};
use lopdf::content::Content;
use lopdf::{Dictionary, Object};
use vl_core::{Affine, PathBuilder, Point, VectorPath};

/// PDF-Punkt (1/72 Zoll) in mm.
const PT_TO_MM: f64 = 25.4 / 72.0;
/// Schutz vor „Dekompressionsbomben“ in Inhaltsströmen.
const MAX_CONTENT: usize = 256 * 1024 * 1024;
const MAX_FORM_DEPTH: usize = 12;

pub fn import_pdf(data: &[u8]) -> Result<Imported, String> {
    let doc = match lopdf::Document::load_mem(data) {
        Ok(d) => d,
        Err(e) => {
            return Err(if data.starts_with(b"%!PS") {
                "Diese Datei ist im PostScript-Format (EPS bzw. altes Illustrator-Format) gespeichert. \
                 Bitte als PDF, PDF-kompatible AI-Datei oder SVG speichern."
                    .into()
            } else {
                format!("PDF konnte nicht gelesen werden: {e}")
            });
        }
    };
    let pages = doc.get_pages();
    let Some((_, &page_id)) = pages.iter().next() else {
        return Err("Die PDF-Datei enthält keine Seiten.".into());
    };
    let page = doc.get_dictionary(page_id).map_err(|e| format!("Seite konnte nicht gelesen werden: {e}"))?;
    let media = inherited(&doc, page, b"MediaBox")
        .and_then(|o| o.as_array().ok())
        .map(|a| a.iter().filter_map(|v| num(&doc, v)).collect::<Vec<_>>())
        .filter(|v| v.len() == 4)
        .unwrap_or_else(|| vec![0.0, 0.0, 612.0, 792.0]);
    let (left, top) = (media[0].min(media[2]), media[1].max(media[3]));
    // Seitenkoordinaten (Ursprung unten links, y nach oben) → Dokument (mm, y nach unten)
    let page_tf = Affine([PT_TO_MM, 0.0, 0.0, -PT_TO_MM, -left * PT_TO_MM, top * PT_TO_MM]);

    let content = doc
        .get_page_content_with_limit(page_id, MAX_CONTENT)
        .map_err(|e| format!("Seiteninhalt konnte nicht gelesen werden: {e}"))?;
    let resources = inherited(&doc, page, b"Resources").and_then(|o| o.as_dict().ok());

    let mut ctx = Interp { doc: &doc, out: Imported::default(), stats: Stats::default() };
    ctx.run(&content, resources, GState { ctm: page_tf, ..GState::default() }, 0);

    let mut out = ctx.out;
    let s = ctx.stats;
    if pages.len() > 1 {
        out.warnings.push(format!("Nur die erste von {} Seiten wurde importiert.", pages.len()));
    }
    if s.text > 0 {
        out.warnings
            .push("Text in der PDF wurde nicht importiert – bitte vor dem Export in Kurven/Pfade umwandeln.".into());
    }
    if s.images > 0 {
        out.warnings.push(format!("{} eingebettete(s) Bild(er) wurden übersprungen.", s.images));
    }
    if s.white_fills > 0 {
        out.warnings
            .push(format!("{} weiße Füllfläche(n) (z. B. Seitenhintergrund) wurden übersprungen.", s.white_fills));
    }
    if out.items.is_empty() {
        out.warnings.push("In der PDF wurden keine Vektorpfade gefunden.".into());
    }
    Ok(out)
}

/// Wert eines Seitenattributs, ggf. vom übergeordneten Seitenbaum geerbt.
fn inherited<'a>(doc: &'a lopdf::Document, page: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    let mut node = page;
    for _ in 0..32 {
        if let Ok(v) = node.get(key) {
            return doc.dereference(v).ok().map(|(_, o)| o);
        }
        node = node.get(b"Parent").ok().and_then(|p| doc.dereference(p).ok()).and_then(|(_, o)| o.as_dict().ok())?;
    }
    None
}

fn num(doc: &lopdf::Document, o: &Object) -> Option<f64> {
    let o = doc.dereference(o).ok()?.1;
    o.as_float().ok().map(f64::from).filter(|v| v.is_finite())
}

#[derive(Default)]
struct Stats {
    text: usize,
    images: usize,
    white_fills: usize,
}

#[derive(Clone, Copy)]
struct GState {
    ctm: Affine,
    stroke: [u8; 3],
    fill: [u8; 3],
}

impl Default for GState {
    fn default() -> Self {
        Self { ctm: Affine::IDENTITY, stroke: [0, 0, 0], fill: [0, 0, 0] }
    }
}

fn to_u8(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Farbe aus 1 (Grau), 3 (RGB) oder 4 (CMYK) Werten.
fn color(v: &[f64]) -> Option<[u8; 3]> {
    match *v {
        [g] => Some([to_u8(g); 3]),
        [r, g, b] => Some([to_u8(r), to_u8(g), to_u8(b)]),
        [c, m, y, k] => {
            Some([to_u8((1.0 - c) * (1.0 - k)), to_u8((1.0 - m) * (1.0 - k)), to_u8((1.0 - y) * (1.0 - k))])
        }
        _ => None,
    }
}

struct Interp<'a> {
    doc: &'a lopdf::Document,
    out: Imported,
    stats: Stats,
}

impl Interp<'_> {
    fn run(&mut self, content: &[u8], resources: Option<&Dictionary>, start: GState, depth: usize) {
        let Ok(ops) = Content::decode(content) else { return };
        let mut gs = start;
        let mut stack: Vec<GState> = Vec::new();
        let mut pb = PathBuilder::new();
        let mut current: Option<Point> = None;
        let mut sub_start: Option<Point> = None;
        for op in &ops.operations {
            let nums: Vec<f64> = op.operands.iter().filter_map(|o| num(self.doc, o)).collect();
            let p = |i: usize| gs.ctm.apply(Point::new(nums[i], nums[i + 1]));
            match (op.operator.as_str(), nums.len()) {
                ("q", _) => stack.push(gs),
                ("Q", _) => gs = stack.pop().unwrap_or(gs),
                ("cm", 6) => {
                    let m = Affine([nums[0], nums[1], nums[2], nums[3], nums[4], nums[5]]);
                    gs.ctm = m.then(&gs.ctm);
                }
                ("RG" | "G" | "K", _) => gs.stroke = color(&nums).unwrap_or(gs.stroke),
                ("rg" | "g" | "k", _) => gs.fill = color(&nums).unwrap_or(gs.fill),
                ("SC" | "SCN", _) => gs.stroke = color(&nums).unwrap_or(gs.stroke),
                ("sc" | "scn", _) => gs.fill = color(&nums).unwrap_or(gs.fill),
                ("m", 2) => {
                    let q = p(0);
                    pb.move_to(q);
                    current = Some(q);
                    sub_start = Some(q);
                }
                ("l", 2) => {
                    let q = p(0);
                    pb.line_to(q);
                    current = Some(q);
                }
                ("c", 6) => {
                    let q = p(4);
                    pb.cubic_to(p(0), p(2), q);
                    current = Some(q);
                }
                ("v", 4) => {
                    let q = p(2);
                    pb.cubic_to(current.unwrap_or(q), p(0), q);
                    current = Some(q);
                }
                ("y", 4) => {
                    let q = p(2);
                    pb.cubic_to(p(0), q, q);
                    current = Some(q);
                }
                ("h", _) => {
                    pb.close();
                    current = sub_start;
                }
                ("re", 4) => {
                    let (x, y, w, h) = (nums[0], nums[1], nums[2], nums[3]);
                    let c = |a: f64, b: f64| gs.ctm.apply(Point::new(a, b));
                    pb.move_to(c(x, y));
                    pb.line_to(c(x + w, y));
                    pb.line_to(c(x + w, y + h));
                    pb.line_to(c(x, y + h));
                    pb.close();
                    current = Some(c(x, y));
                    sub_start = current;
                }
                (op @ ("S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*"), _) => {
                    if matches!(op, "s" | "b" | "b*") {
                        pb.close();
                    }
                    let stroked = matches!(op, "S" | "s" | "B" | "B*" | "b" | "b*");
                    let path = std::mem::take(&mut pb).finish();
                    self.emit(path, stroked, &gs);
                    current = None;
                }
                ("n", _) => {
                    // Beschneidungspfad o. Ä. ohne Zeichnen verwerfen
                    pb = PathBuilder::new();
                    current = None;
                }
                ("Tj" | "TJ" | "'" | "\"", _) => self.stats.text += 1,
                ("BI", _) => self.stats.images += 1,
                ("Do", _) => {
                    if let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) {
                        self.xobject(name, resources, gs, depth);
                    }
                }
                _ => {}
            }
        }
    }

    fn emit(&mut self, path: VectorPath, stroked: bool, gs: &GState) {
        if path.is_empty() {
            return;
        }
        let rgb = if stroked { gs.stroke } else { gs.fill };
        if !stroked && rgb.iter().all(|&c| c >= 250) {
            self.stats.white_fills += 1;
            return;
        }
        self.out.items.push(ImportedItem::Path { name: "PDF-Pfad".into(), layer: layer_for_color(rgb), path });
    }

    fn xobject(&mut self, name: &[u8], resources: Option<&Dictionary>, gs: GState, depth: usize) {
        let doc = self.doc;
        let Some(stream) = resources
            .and_then(|r| r.get(b"XObject").ok())
            .and_then(|x| doc.dereference(x).ok())
            .and_then(|(_, x)| x.as_dict().ok())
            .and_then(|x| x.get(name).ok())
            .and_then(|o| doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_stream().ok())
        else {
            return;
        };
        match stream.dict.get(b"Subtype").ok().and_then(|s| s.as_name().ok()) {
            Some(b"Image") => self.stats.images += 1,
            Some(b"Form") if depth < MAX_FORM_DEPTH => {
                let matrix = stream
                    .dict
                    .get(b"Matrix")
                    .ok()
                    .and_then(|m| doc.dereference(m).ok())
                    .and_then(|(_, m)| m.as_array().ok())
                    .map(|a| a.iter().filter_map(|v| num(doc, v)).collect::<Vec<_>>())
                    .filter(|v| v.len() == 6)
                    .map(|v| Affine([v[0], v[1], v[2], v[3], v[4], v[5]]))
                    .unwrap_or(Affine::IDENTITY);
                let own = stream
                    .dict
                    .get(b"Resources")
                    .ok()
                    .and_then(|r| doc.dereference(r).ok())
                    .and_then(|(_, r)| r.as_dict().ok());
                let Ok(content) = stream.decompressed_content_with_limit(MAX_CONTENT) else { return };
                let inner = GState { ctm: matrix.then(&gs.ctm), ..gs };
                self.run(&content, own.or(resources), inner, depth + 1);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;
    use lopdf::{Stream, content::Operation};

    /// Baut eine einseitige PDF (A4 quer nicht nötig: 200 × 100 pt) mit dem angegebenen Inhalt.
    fn pdf(ops: Vec<Operation>, form: Option<Vec<Operation>>) -> Vec<u8> {
        let mut doc = lopdf::Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let mut resources = Dictionary::new();
        if let Some(form_ops) = form {
            let body = Content { operations: form_ops }.encode().unwrap();
            let form = Stream::new(
                dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
                "Matrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 100.into(), 0.into()] },
                body,
            );
            let form_id = doc.add_object(form);
            resources.set("XObject", dictionary! { "Fm1" => form_id });
        }
        let content = Content { operations: ops }.encode().unwrap();
        let content_id = doc.add_object(Stream::new(dictionary! {}, content));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content_id, "Resources" => resources,
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
                "MediaBox" => vec![0.into(), 0.into(), 200.into(), 100.into()],
            }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        let mut buf = Vec::new();
        doc.save_to(&mut buf).unwrap();
        buf
    }

    fn op(o: &str, args: &[f32]) -> Operation {
        Operation::new(o, args.iter().map(|&v| Object::Real(v)).collect())
    }

    #[test]
    fn imports_paths_with_page_transform_and_colors() {
        let data = pdf(
            vec![
                // weißer Hintergrund (wird übersprungen)
                op("rg", &[1.0, 1.0, 1.0]),
                op("re", &[0.0, 0.0, 200.0, 100.0]),
                op("f", &[]),
                // rote Linie von (0,0) nach (72,0) pt → (0, 35,28) … (25,4, 35,28) mm
                op("RG", &[1.0, 0.0, 0.0]),
                op("m", &[0.0, 0.0]),
                op("l", &[72.0, 0.0]),
                op("S", &[]),
            ],
            None,
        );
        let out = import_pdf(&data).unwrap();
        assert_eq!(out.items.len(), 1);
        let ImportedItem::Path { layer, path, .. } = &out.items[0] else { panic!() };
        assert_eq!(*layer, layer_for_color([255, 0, 0]));
        let b = path.bounds(&Affine::IDENTITY);
        assert!((b.min.x).abs() < 1e-6 && (b.max.x - 25.4).abs() < 1e-6);
        assert!((b.min.y - 100.0 * PT_TO_MM).abs() < 1e-6);
        assert!(out.warnings.iter().any(|w| w.contains("weiße")));
    }

    #[test]
    fn follows_form_xobjects_and_ctm() {
        let data = pdf(
            vec![
                op("q", &[]),
                op("cm", &[2.0, 0.0, 0.0, 2.0, 0.0, 0.0]),
                Operation::new("Do", vec![Object::Name(b"Fm1".to_vec())]),
                op("Q", &[]),
            ],
            Some(vec![op("re", &[0.0, 0.0, 10.0, 10.0]), op("S", &[])]),
        );
        let out = import_pdf(&data).unwrap();
        assert_eq!(out.items.len(), 1);
        let ImportedItem::Path { path, .. } = &out.items[0] else { panic!() };
        let b = path.bounds(&Affine::IDENTITY);
        // Form-Matrix verschiebt um 100 pt, cm skaliert × 2 → x 200…220 pt
        assert!((b.min.x - 200.0 * PT_TO_MM).abs() < 1e-4, "{b:?}");
        assert!((b.width() - 20.0 * PT_TO_MM).abs() < 1e-4);
    }

    #[test]
    fn postscript_gets_helpful_error() {
        let err = import_pdf(b"%!PS-Adobe-3.0 EPSF-3.0\n").unwrap_err();
        assert!(err.contains("PostScript"));
    }
}
