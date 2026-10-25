//! Text in Vektorpfade umwandeln (installierte Schriften über `fontdb`, Glyphen über `ttf-parser`).

use std::sync::{Arc, OnceLock};
use usvg::fontdb::{self, Database, Family, Query, Style, Weight};
use vl_core::ops::{polyline_length, position_at};
pub use vl_core::text::{TextAlign, TextSpec};
use vl_core::text::{TextData, TextOnPath};
use vl_core::{Affine, PathBuilder, Point, Polyline, VectorPath};

static SYSTEM_FONTS: OnceLock<Arc<Database>> = OnceLock::new();

/// Systemschriften (einmalig geladen, danach geteilt).
pub fn system_fonts() -> Arc<Database> {
    SYSTEM_FONTS
        .get_or_init(|| {
            let mut db = Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

/// Alphabetisch sortierte Schriftfamilien.
pub fn font_families(db: &Database) -> Vec<String> {
    let mut names: Vec<String> = db.faces().filter_map(|f| f.families.first().map(|(n, _)| n.clone())).collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

struct Outline<'a> {
    pb: &'a mut PathBuilder,
    t: Affine,
}

impl ttf_parser::OutlineBuilder for Outline<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.pb.move_to(self.t.apply(Point::new(x as f64, y as f64)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.pb.line_to(self.t.apply(Point::new(x as f64, y as f64)));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.pb.quad_to(self.t.apply(Point::new(x1 as f64, y1 as f64)), self.t.apply(Point::new(x as f64, y as f64)));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.pb.cubic_to(
            self.t.apply(Point::new(x1 as f64, y1 as f64)),
            self.t.apply(Point::new(x2 as f64, y2 as f64)),
            self.t.apply(Point::new(x as f64, y as f64)),
        );
    }
    fn close(&mut self) {
        self.pb.close();
    }
}

/// Erzeugt den Text als Pfad. Die linke obere Ecke der ersten Zeile liegt bei (0, 0).
pub fn text_to_path(db: &Database, spec: &TextSpec) -> Result<VectorPath, String> {
    with_face(db, spec, |face| layout(face, spec))
}

/// Erzeugt einen Text aus Textdaten mit dem bereits eingesetzten Inhalt `text` (Platzhalter ersetzt):
/// normal mit der linken oberen Ecke bei (0, 0) oder entlang des Pfades in Dokumentkoordinaten.
pub fn render_text(db: &Database, data: &TextData, text: &str) -> Result<VectorPath, String> {
    if let Some(b) = &data.barcode {
        return crate::barcode::render_barcode(db, data, b, text);
    }
    let spec = TextSpec { text: text.to_string(), ..data.spec.clone() };
    match &data.on_path {
        Some(on) => with_face(db, &spec, |face| layout_on_path(face, &spec, on)),
        None => with_face(db, &spec, |face| layout(face, &spec)),
    }
}

fn with_face(
    db: &Database,
    spec: &TextSpec,
    f: impl FnOnce(&ttf_parser::Face) -> Result<VectorPath, String>,
) -> Result<VectorPath, String> {
    let query = Query {
        families: &[Family::Name(&spec.family), Family::SansSerif],
        weight: if spec.bold { Weight::BOLD } else { Weight::NORMAL },
        style: if spec.italic { Style::Italic } else { Style::Normal },
        ..Default::default()
    };
    let id = db.query(&query).ok_or_else(|| format!("Schrift „{}“ nicht gefunden.", spec.family))?;
    let result = db.with_face_data(id, |data, index| -> Result<VectorPath, String> {
        let face = ttf_parser::Face::parse(data, index).map_err(|e| format!("Schrift unlesbar: {e}"))?;
        f(&face)
    });
    let path = result.ok_or_else(|| "Schriftdaten konnten nicht geladen werden.".to_string())??;
    Ok(if spec.weld { welded(&path) } else { path })
}

/// Verschweißt überlappende Glyphen (Nicht-Null-Regel der Schriften); das Ergebnis besteht aus Polygonen.
fn welded(path: &VectorPath) -> VectorPath {
    let polys = path.flatten(&Affine::IDENTITY, 0.01);
    let mut out = VectorPath::default();
    for p in vl_core::ops::union_nonzero(&polys) {
        out.append(VectorPath::polygon(&p.points, true));
    }
    out
}

fn layout(face: &ttf_parser::Face, spec: &TextSpec) -> Result<VectorPath, String> {
    let scale = cap_scale(face, spec);
    let cap = spec.height_mm.max(0.1) / scale;
    let line_height =
        (face.ascender() as f64 - face.descender() as f64 + face.line_gap() as f64) * scale * spec.line_spacing;
    let ascent = cap * scale;

    let lines: Vec<&str> = spec.text.lines().collect();
    let widths: Vec<f64> = lines.iter().map(|l| line_width(face, l, scale, spec.letter_spacing)).collect();
    let max_width = widths.iter().cloned().fold(0.0, f64::max);

    let mut pb = PathBuilder::new();
    for (i, line) in lines.iter().enumerate() {
        let baseline = ascent + i as f64 * line_height;
        let mut x = match spec.align {
            TextAlign::Left => 0.0,
            TextAlign::Center => (max_width - widths[i]) / 2.0,
            TextAlign::Right => max_width - widths[i],
        };
        let mut prev: Option<ttf_parser::GlyphId> = None;
        for ch in line.chars() {
            let Some(gid) = face.glyph_index(ch).or_else(|| face.glyph_index('?')) else { continue };
            if let Some(p) = prev {
                x += kerning(face, p, gid) * scale;
            }
            // Schrift: Y nach oben -> Dokument: Y nach unten
            let t = Affine::scale(scale, -scale).then(&Affine::translate(x, baseline));
            face.outline_glyph(gid, &mut Outline { pb: &mut pb, t });
            x += face.glyph_hor_advance(gid).unwrap_or(0) as f64 * scale + spec.letter_spacing;
            prev = Some(gid);
        }
    }
    let path = pb.finish();
    if path.is_empty() && !spec.text.trim().is_empty() {
        return Err("Die Schrift enthält keine Umrisse für diesen Text.".into());
    }
    Ok(path)
}

fn cap_scale(face: &ttf_parser::Face, spec: &TextSpec) -> f64 {
    let cap = face.capital_height().filter(|&h| h > 0).map(|h| h as f64).unwrap_or(face.ascender() as f64 * 0.7);
    spec.height_mm.max(0.1) / cap
}

/// Text entlang einer Linie: jede Glyphe sitzt mit ihrer Mitte auf dem Pfad und folgt dessen Richtung.
/// Geschlossene Pfade beginnen oben (bzw. unten bei umgekehrter Richtung); „Mitte“ zentriert dort.
fn layout_on_path(face: &ttf_parser::Face, spec: &TextSpec, on: &TextOnPath) -> Result<VectorPath, String> {
    let scale = cap_scale(face, spec);
    let text = spec.text.lines().collect::<Vec<_>>().join(" ");
    let mut pts = on.points.clone();
    if on.reverse {
        pts.reverse();
    }
    if pts.len() < 2 {
        return Err("Der Pfad für den Text ist zu kurz.".into());
    }
    if on.closed {
        // Oben (bzw. unten) beginnen, damit der Text aufrecht über (unter) der Form steht
        let key = |p: &Point| if on.reverse { -p.y } else { p.y };
        let top = (0..pts.len()).min_by(|&a, &b| key(&pts[a]).total_cmp(&key(&pts[b]))).unwrap_or(0);
        pts.rotate_left(top);
    }
    let poly = Polyline::new(pts, on.closed);
    let total = polyline_length(&poly);
    let width = line_width(face, &text, scale, spec.letter_spacing);
    let start = on.offset
        + match (spec.align, on.closed) {
            (TextAlign::Left, _) => 0.0,
            (TextAlign::Center, true) => -width / 2.0,
            (TextAlign::Right, true) => -width,
            (TextAlign::Center, false) => (total - width) / 2.0,
            (TextAlign::Right, false) => total - width,
        };
    let mut pb = PathBuilder::new();
    let mut x = 0.0;
    let mut prev: Option<ttf_parser::GlyphId> = None;
    for ch in text.chars() {
        let Some(gid) = face.glyph_index(ch).or_else(|| face.glyph_index('?')) else { continue };
        if let Some(p) = prev {
            x += kerning(face, p, gid) * scale;
        }
        let adv = face.glyph_hor_advance(gid).unwrap_or(0) as f64 * scale;
        if let Some((p, angle)) = position_at(&poly, start + x + adv / 2.0) {
            let t = Affine::scale(scale, -scale)
                .then(&Affine::translate(-adv / 2.0, -on.distance))
                .then(&Affine::rotate(angle))
                .then(&Affine::translate(p.x, p.y));
            face.outline_glyph(gid, &mut Outline { pb: &mut pb, t });
        }
        x += adv + spec.letter_spacing;
        prev = Some(gid);
    }
    let path = pb.finish();
    if path.is_empty() && !text.trim().is_empty() {
        return Err("Die Schrift enthält keine Umrisse für diesen Text.".into());
    }
    Ok(path)
}

fn line_width(face: &ttf_parser::Face, line: &str, scale: f64, spacing: f64) -> f64 {
    let mut w = 0.0;
    let mut prev = None;
    for ch in line.chars() {
        let Some(gid) = face.glyph_index(ch).or_else(|| face.glyph_index('?')) else { continue };
        if let Some(p) = prev {
            w += kerning(face, p, gid) * scale;
        }
        w += face.glyph_hor_advance(gid).unwrap_or(0) as f64 * scale + spacing;
        prev = Some(gid);
    }
    (w - spacing).max(0.0)
}

fn kerning(face: &ttf_parser::Face, left: ttf_parser::GlyphId, right: ttf_parser::GlyphId) -> f64 {
    face.tables()
        .kern
        .and_then(|k| {
            k.subtables.into_iter().filter(|s| s.horizontal && !s.variable).find_map(|s| s.glyphs_kerning(left, right))
        })
        .unwrap_or(0) as f64
}

/// Für Tests und Notfälle: beliebige installierte Schrift.
pub fn any_family(db: &Database) -> Option<String> {
    db.faces().find_map(|f| match f.source {
        fontdb::Source::File(_) | fontdb::Source::Binary(_) | fontdb::Source::SharedFile(..) => {
            f.families.first().map(|(n, _)| n.clone())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_text_with_system_font() {
        let db = system_fonts();
        let preferred = ["Arial", "Segoe UI", "DejaVu Sans", "Liberation Sans", "Helvetica"];
        let families = font_families(&db);
        let pick = preferred.iter().find(|p| families.iter().any(|f| f == *p)).map(|s| s.to_string());
        let Some(family) = pick.or_else(|| any_family(&db)) else {
            eprintln!("Keine Systemschriften gefunden – Test übersprungen.");
            return;
        };
        let spec = TextSpec { text: "HI".into(), family, height_mm: 10.0, ..Default::default() };
        let path = text_to_path(&db, &spec).unwrap();
        let b = path.bounds(&Affine::IDENTITY);
        assert!(b.width() > 3.0, "{b:?}");
        // Großbuchstaben ohne Unterlänge: Höhe entspricht ungefähr der gewünschten Höhe.
        assert!((b.height() - 10.0).abs() < 1.5, "{b:?}");
        assert!(b.min.y > -1.0, "{b:?}");
    }

    #[test]
    fn welding_merges_overlapping_letters() {
        let db = system_fonts();
        let Some(family) = any_family(&db) else { return };
        let spec = TextSpec { text: "OO".into(), family, height_mm: 10.0, letter_spacing: -3.0, ..Default::default() };
        let plain = text_to_path(&db, &spec).unwrap();
        let welded = text_to_path(&db, &TextSpec { weld: true, ..spec }).unwrap();
        let closed = |p: &VectorPath| p.subpaths.iter().filter(|s| s.closed).count();
        assert_eq!(closed(&plain), 4, "zwei O mit je einem Loch");
        // eine gemeinsame Außenkontur, die Löcher bleiben
        assert!(closed(&welded) == 3, "{}", closed(&welded));
        let b = |p: &VectorPath| p.bounds(&Affine::IDENTITY);
        assert!((b(&plain).width() - b(&welded).width()).abs() < 0.05);
    }

    #[test]
    fn text_follows_line_and_circle() {
        let db = system_fonts();
        let Some(family) = any_family(&db) else { return };
        // Waagrechte Linie: Text liegt darüber (y < 0), zentriert um x = 50
        let mut data = TextData {
            spec: TextSpec { family, height_mm: 5.0, align: TextAlign::Center, ..Default::default() },
            on_path: Some(TextOnPath {
                points: vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0)],
                ..Default::default()
            }),
            ..Default::default()
        };
        let p = render_text(&db, &data, "HHHH").unwrap();
        let b = p.bounds(&Affine::IDENTITY);
        assert!(b.max.y <= 0.01 && b.min.y > -6.0, "{b:?}");
        assert!((b.center().x - 50.0).abs() < 1.0, "{b:?}");
        // Kreis (Uhrzeigersinn): Text oben außen, mittig über dem höchsten Punkt
        let circle = VectorPath::ellipse(Point::new(0.0, 0.0), 30.0, 30.0).flatten(&Affine::IDENTITY, 0.01);
        data.on_path = Some(TextOnPath { points: circle[0].points.clone(), closed: true, ..Default::default() });
        let b = render_text(&db, &data, "HHHH").unwrap().bounds(&Affine::IDENTITY);
        assert!(b.max.y < -29.0 && b.center().x.abs() < 1.0, "{b:?}");
        // umgekehrt: unten innen
        data.on_path.as_mut().unwrap().reverse = true;
        let b = render_text(&db, &data, "HHHH").unwrap().bounds(&Affine::IDENTITY);
        assert!(b.min.y > 20.0 && b.max.y < 30.5 && b.center().x.abs() < 1.0, "{b:?}");
    }
}
