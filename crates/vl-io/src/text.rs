//! Text in Vektorpfade umwandeln (installierte Schriften über `fontdb`, Glyphen über `ttf-parser`).

use std::sync::{Arc, OnceLock};
use usvg::fontdb::{self, Database, Family, Query, Style, Weight};
use vl_core::{Affine, PathBuilder, Point, VectorPath};

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextSpec {
    pub text: String,
    pub family: String,
    pub bold: bool,
    pub italic: bool,
    /// Höhe der Großbuchstaben in mm.
    pub height_mm: f64,
    /// Zusätzlicher Zeichenabstand in mm.
    pub letter_spacing: f64,
    /// Zeilenabstand als Faktor der Schrifthöhe.
    pub line_spacing: f64,
    pub align: TextAlign,
}

impl Default for TextSpec {
    fn default() -> Self {
        Self {
            text: "VectorLume".into(),
            family: "Arial".into(),
            bold: false,
            italic: false,
            height_mm: 10.0,
            letter_spacing: 0.0,
            line_spacing: 1.0,
            align: TextAlign::Left,
        }
    }
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
    let query = Query {
        families: &[Family::Name(&spec.family), Family::SansSerif],
        weight: if spec.bold { Weight::BOLD } else { Weight::NORMAL },
        style: if spec.italic { Style::Italic } else { Style::Normal },
        ..Default::default()
    };
    let id = db.query(&query).ok_or_else(|| format!("Schrift „{}“ nicht gefunden.", spec.family))?;
    let result = db.with_face_data(id, |data, index| -> Result<VectorPath, String> {
        let face = ttf_parser::Face::parse(data, index).map_err(|e| format!("Schrift unlesbar: {e}"))?;
        layout(&face, spec)
    });
    result.ok_or_else(|| "Schriftdaten konnten nicht geladen werden.".to_string())?
}

fn layout(face: &ttf_parser::Face, spec: &TextSpec) -> Result<VectorPath, String> {
    let cap = face.capital_height().filter(|&h| h > 0).map(|h| h as f64).unwrap_or(face.ascender() as f64 * 0.7);
    let scale = spec.height_mm.max(0.1) / cap;
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
}
