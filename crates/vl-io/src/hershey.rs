//! Eingebaute Einlinienschriften (Hershey-Schriften): Buchstaben aus offenen Strichen statt Umrissen, z. B. zum
//! schnellen Gravieren oder Ritzen. Daten und Danksagung: `fonts/hershey/README.md`.

use std::sync::OnceLock;
use vl_core::{Affine, PathBuilder, Point};

/// Schriftfamilie mit Dateien für normal, fett und kursiv (fehlende Schnitte: normal, kursiv wird geneigt).
struct Family {
    name: &'static str,
    regular: &'static str,
    bold: Option<&'static str>,
    italic: Option<&'static str>,
}

const FAMILIES: [Family; 5] = [
    Family { name: "Hershey Sans", regular: "futural", bold: Some("futuram"), italic: None },
    Family { name: "Hershey Serif", regular: "timesr", bold: None, italic: Some("timesi") },
    Family { name: "Hershey Script", regular: "scripts", bold: Some("scriptc"), italic: None },
    Family { name: "Hershey Cursive", regular: "cursive", bold: None, italic: None },
    Family { name: "Hershey Gothic", regular: "gothiceng", bold: None, italic: None },
];

fn data(file: &str) -> &'static str {
    match file {
        "futural" => include_str!("../fonts/hershey/futural.jhf"),
        "futuram" => include_str!("../fonts/hershey/futuram.jhf"),
        "timesr" => include_str!("../fonts/hershey/timesr.jhf"),
        "timesi" => include_str!("../fonts/hershey/timesi.jhf"),
        "scripts" => include_str!("../fonts/hershey/scripts.jhf"),
        "scriptc" => include_str!("../fonts/hershey/scriptc.jhf"),
        "cursive" => include_str!("../fonts/hershey/cursive.jhf"),
        _ => include_str!("../fonts/hershey/gothiceng.jhf"),
    }
}

/// Namen der eingebauten Einlinienschriften.
pub fn families() -> impl Iterator<Item = &'static str> {
    FAMILIES.iter().map(|f| f.name)
}

pub fn is_single_line(family: &str) -> bool {
    FAMILIES.iter().any(|f| f.name == family)
}

/// Glyphe in Hershey-Koordinaten (Y nach unten): linker und rechter Rand, Striche.
struct Glyph {
    left: f64,
    right: f64,
    strokes: Vec<Vec<Point>>,
}

/// Liest Hershey-Daten (je Glyphe: 5 Zeichen Nummer, 3 Zeichen Punktanzahl, dann Punktpaare mit `R` als Null;
/// „ R“ hebt den Stift). Zeilenumbrüche innerhalb einer Glyphe sind erlaubt.
fn parse(text: &str) -> Vec<Glyph> {
    let s: Vec<u8> = text.bytes().filter(|b| *b != b'\n' && *b != b'\r').collect();
    let mut glyphs = Vec::new();
    let mut i = 0;
    while i + 8 <= s.len() {
        let Some(n) = std::str::from_utf8(&s[i + 5..i + 8]).ok().and_then(|t| t.trim().parse::<usize>().ok()) else {
            break;
        };
        let body = &s[i + 8..(i + 8 + 2 * n).min(s.len())];
        i += 8 + 2 * n;
        if body.len() < 2 {
            break;
        }
        let v = |b: u8| (b as i32 - b'R' as i32) as f64;
        let mut strokes = Vec::new();
        let mut cur = Vec::new();
        for pair in body[2..].as_chunks::<2>().0 {
            if pair == b" R" {
                if !cur.is_empty() {
                    strokes.push(std::mem::take(&mut cur));
                }
            } else {
                cur.push(Point::new(v(pair[0]), v(pair[1])));
            }
        }
        if !cur.is_empty() {
            strokes.push(cur);
        }
        glyphs.push(Glyph { left: v(body[0]), right: v(body[1]), strokes });
    }
    glyphs
}

/// Eine geladene Einlinienschrift. Maße in Schrifteinheiten.
pub struct Font {
    glyphs: Vec<Glyph>,
    baseline: f64,
    cap: f64,
    descent: f64,
    /// Punkt des „i“ (für Umlaute), um seine Mitte zentriert.
    dot: Vec<Point>,
    /// Höhe des i-Punktes über der Grundlinie.
    dot_height: f64,
    slant: f64,
}

/// Glyphe im Text: ASCII-Zeichen ab Leerzeichen, wahlweise mit Umlautpunkten.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphRef {
    index: usize,
    umlaut: bool,
}

fn bounds(strokes: &[Vec<Point>]) -> Option<(f64, f64)> {
    let ys = strokes.iter().flatten().map(|p| p.y);
    let min = ys.clone().fold(f64::INFINITY, f64::min);
    let max = ys.fold(f64::NEG_INFINITY, f64::max);
    (min <= max).then_some((min, max))
}

impl Font {
    fn load(file: &str, slant: f64) -> Font {
        let glyphs = parse(data(file));
        let get = |c: char| glyphs.get(c as usize - 32).and_then(|g| bounds(&g.strokes));
        let (top, baseline) = get('H').unwrap_or((-12.0, 9.0));
        let descent = get('g').map_or(7.0, |(_, max)| (max - baseline).max(0.0));
        // Punkt des „i“: der höchste Strich
        let (dot, dot_height) = glyphs
            .get('i' as usize - 32)
            .and_then(|g| {
                let s = g.strokes.iter().min_by(|a, b| {
                    let y = |s: &Vec<Point>| s.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
                    y(a).total_cmp(&y(b))
                })?;
                let (min, max) = bounds(std::slice::from_ref(s))?;
                let cx = s.iter().map(|p| p.x).sum::<f64>() / s.len() as f64;
                let cy = (min + max) / 2.0;
                Some((s.iter().map(|p| Point::new(p.x - cx, p.y - cy)).collect(), baseline - cy))
            })
            .unwrap_or((vec![Point::new(-0.5, 0.0), Point::new(0.5, 0.0)], (baseline - top) * 0.75));
        Font { glyphs, baseline, cap: baseline - top, descent, dot, dot_height, slant }
    }

    /// Ersetzt Zeichen ohne eigene Glyphe (ß → ss, typografische Anführungszeichen, Striche …).
    pub fn prepare(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            match c {
                'ß' => out.push_str("ss"),
                '„' | '“' | '”' | '«' | '»' => out.push('"'),
                '‚' | '‘' | '’' => out.push('\''),
                '–' | '—' => out.push('-'),
                '…' => out.push_str("..."),
                '\u{a0}' => out.push(' '),
                c => out.push(c),
            }
        }
        out
    }

    pub fn glyph(&self, ch: char) -> Option<GlyphRef> {
        let (base, umlaut) = match ch {
            'ä' => ('a', true),
            'ö' => ('o', true),
            'ü' => ('u', true),
            'Ä' => ('A', true),
            'Ö' => ('O', true),
            'Ü' => ('U', true),
            c if (' '..='~').contains(&c) => (c, false),
            _ => ('?', false),
        };
        let index = base as usize - 32;
        (index < self.glyphs.len()).then_some(GlyphRef { index, umlaut })
    }

    pub fn advance(&self, g: GlyphRef) -> f64 {
        let gl = &self.glyphs[g.index];
        gl.right - gl.left
    }

    pub fn cap_height(&self) -> f64 {
        self.cap
    }

    /// Zeilenabstand wie bei Schriften üblich: Ober- plus Unterlänge mit etwas Luft.
    pub fn line_height(&self) -> f64 {
        (self.cap + self.descent) * 1.25
    }

    /// Zeichnet die Glyphe mit Ursprung am linken Rand auf der Grundlinie, Y nach oben (wie Schriftumrisse).
    pub fn outline(&self, g: GlyphRef, pb: &mut PathBuilder, t: Affine) {
        let gl = &self.glyphs[g.index];
        let to_font = |p: Point| {
            let y = self.baseline - p.y;
            Point::new(p.x - gl.left + y * self.slant, y)
        };
        let mut stroke = |pts: &mut dyn Iterator<Item = Point>| {
            if let Some(first) = pts.next() {
                pb.move_to(t.apply(first));
                for p in pts {
                    pb.line_to(t.apply(p));
                }
            }
        };
        for s in &gl.strokes {
            stroke(&mut s.iter().map(|p| to_font(*p)));
        }
        if g.umlaut {
            let base = (g.index + 32) as u8 as char;
            let y = if base.is_ascii_uppercase() { self.cap * 1.22 } else { self.dot_height };
            let cx = (gl.left + gl.right) / 2.0;
            let dx = self.cap * 0.17;
            for x in [cx - dx, cx + dx] {
                // Punkt in Hershey-Koordinaten an (x, Grundlinie − y)
                stroke(&mut self.dot.iter().map(|d| to_font(Point::new(x + d.x, self.baseline - y + d.y))));
            }
        }
    }
}

/// Die eingebaute Einlinienschrift zu Familie und Schnitt, sonst `None`.
pub fn font(family: &str, bold: bool, italic: bool) -> Option<&'static Font> {
    static CACHE: OnceLock<std::sync::Mutex<Vec<(String, &'static Font)>>> = OnceLock::new();
    let f = FAMILIES.iter().find(|f| f.name == family)?;
    let file = match (bold, italic, f.bold, f.italic) {
        (true, _, Some(b), _) => b,
        (_, true, _, Some(i)) => i,
        _ => f.regular,
    };
    // Kursiv ohne eigenen Schnitt: neigen
    let slant = if italic && f.italic.is_none() { 0.2 } else { 0.0 };
    let key = format!("{file}/{slant}");
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, font)) = cache.iter().find(|(k, _)| *k == key) {
        return Some(font);
    }
    let font: &'static Font = Box::leak(Box::new(Font::load(file, slant)));
    cache.push((key, font));
    Some(font)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_fonts() {
        for f in &FAMILIES {
            for file in [Some(f.regular), f.bold, f.italic].into_iter().flatten() {
                let font = Font::load(file, 0.0);
                assert!(font.glyphs.len() >= 95, "{file}: {}", font.glyphs.len());
                assert!(font.cap > 10.0 && font.descent > 2.0, "{file}");
                assert!(!font.glyphs['H' as usize - 32].strokes.is_empty());
            }
        }
        assert!(font("Hershey Sans", false, false).is_some());
        assert!(font("Arial", false, false).is_none());
    }

    #[test]
    fn umlauts_add_two_dots() {
        let f = font("Hershey Sans", false, false).unwrap();
        let count = |c: char| {
            let mut pb = PathBuilder::new();
            f.outline(f.glyph(c).unwrap(), &mut pb, Affine::IDENTITY);
            let p = pb.finish();
            let top = p.bounds(&Affine::IDENTITY).max.y;
            (p.subpaths.len(), top)
        };
        let (a, a_top) = count('a');
        let (ae, ae_top) = count('ä');
        assert_eq!(ae, a + 2);
        assert!(ae_top > a_top);
        let (big, big_top) = count('Ü');
        assert_eq!(big, count('U').0 + 2);
        assert!(big_top > f.cap_height(), "Punkte über der Versalhöhe");
        assert_eq!(f.prepare("Straße „x“"), "Strasse \"x\"");
        assert_eq!(f.glyph('€'), f.glyph('?'));
    }
}
