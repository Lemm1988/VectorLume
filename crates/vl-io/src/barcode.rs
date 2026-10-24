//! Strichcodes als Vektorpfade: QR-Code und Data Matrix (Modulraster) sowie Code 128, Code 39, EAN-13, EAN-8
//! und UPC-A (Strichfolge). Benachbarte Module werden zu einem Umriss zusammengefasst, damit das Füllen nach der
//! Gerade-Ungerade-Regel an gemeinsamen Kanten keine Lücken lässt.

use crate::text::text_to_path;
use std::collections::HashMap;
use usvg::fontdb::Database;
use vl_core::ops::{Corner, offset};
use vl_core::text::{BarcodeKind, BarcodeSpec, QrLevel, TextData};
use vl_core::{Affine, PathBuilder, Point, Polyline, TextAlign, TextSpec, VectorPath};

/// Kodierter Strichcode: dunkle Module als Raster (1D-Codes haben eine Zeile).
#[derive(Debug)]
pub struct Modules {
    pub width: usize,
    pub height: usize,
    dark: Vec<bool>,
    /// Klartext unter 1D-Codes (bei EAN/UPC mit Prüfziffer).
    pub text: String,
}

impl Modules {
    fn is_dark(&self, x: i64, y: i64) -> bool {
        x >= 0
            && y >= 0
            && (x as usize) < self.width
            && (y as usize) < self.height
            && self.dark[y as usize * self.width + x as usize]
    }

    fn from_bars(bars: Vec<u8>, text: String) -> Self {
        Self { width: bars.len(), height: 1, dark: bars.into_iter().map(|b| b == 1).collect(), text }
    }
}

/// Größe eines Codes ohne Klartext.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarcodeInfo {
    /// Module ohne Ruhezone (Breite, Höhe; 1D-Codes: Höhe 1).
    pub modules: (usize, usize),
    /// Breite und Höhe in mm, beim Invertieren mit Ruhezone.
    pub size_mm: (f64, f64),
}

/// Kodiert den Inhalt. Fehler sind deutsche Meldungen für die Oberfläche.
pub fn encode(kind: BarcodeKind, level: QrLevel, content: &str) -> Result<Modules, String> {
    if content.is_empty() {
        return Err("Der Inhalt ist leer.".into());
    }
    match kind {
        BarcodeKind::Qr => {
            let ec = match level {
                QrLevel::L => qrcode::EcLevel::L,
                QrLevel::M => qrcode::EcLevel::M,
                QrLevel::Q => qrcode::EcLevel::Q,
                QrLevel::H => qrcode::EcLevel::H,
            };
            let code = qrcode::QrCode::with_error_correction_level(content.as_bytes(), ec).map_err(|e| match e {
                qrcode::types::QrError::DataTooLong => "Zu viel Inhalt für einen QR-Code.".to_string(),
                _ => "Der Inhalt kann nicht als QR-Code kodiert werden.".to_string(),
            })?;
            let width = code.width();
            let dark = code.to_colors().into_iter().map(|c| c == qrcode::Color::Dark).collect();
            Ok(Modules { width, height: width, dark, text: String::new() })
        }
        BarcodeKind::DataMatrix => {
            let code =
                datamatrix::DataMatrix::encode_str(content, datamatrix::SymbolList::default()).map_err(|_| {
                    "Der Inhalt passt nicht in einen Data-Matrix-Code (zu lang oder Zeichen außerhalb von Latin-1)."
                        .to_string()
                })?;
            let bitmap = code.bitmap();
            let (width, height) = (bitmap.width(), bitmap.height());
            let mut dark = vec![false; width * height];
            for (x, y) in bitmap.pixels() {
                dark[y * width + x] = true;
            }
            Ok(Modules { width, height, dark, text: String::new() })
        }
        BarcodeKind::Code128 => {
            if !content.chars().all(|c| (' '..='~').contains(&c)) {
                return Err("Code 128 kann nur Zeichen aus dem ASCII-Bereich enthalten (keine Umlaute).".into());
            }
            let code = barcoders::sym::code128::Code128::new(code128_sets(content))
                .map_err(|_| "Der Inhalt kann nicht als Code 128 kodiert werden.".to_string())?;
            Ok(Modules::from_bars(code.encode(), content.to_string()))
        }
        BarcodeKind::Code39 => {
            let upper = content.to_uppercase();
            let code = barcoders::sym::code39::Code39::new(&upper)
                .map_err(|_| "Code 39 kann nur A–Z, 0–9, Leerzeichen und - . $ / + % enthalten.".to_string())?;
            Ok(Modules::from_bars(code.encode(), upper))
        }
        BarcodeKind::Ean13 | BarcodeKind::Ean8 | BarcodeKind::UpcA => {
            let digits = content.trim();
            let n = match kind {
                BarcodeKind::Ean13 => 12,
                BarcodeKind::Ean8 => 7,
                _ => 11,
            };
            if !digits.chars().all(|c| c.is_ascii_digit()) || !(n..=n + 1).contains(&digits.len()) {
                return Err(format!("{} braucht {} oder {} Ziffern.", kind.label(), n, n + 1));
            }
            let values: Vec<u8> = digits.bytes().map(|b| b - b'0').collect();
            let check = check_digit(&values[..n]);
            if values.len() == n + 1 && values[n] != check {
                return Err(format!("Die Prüfziffer stimmt nicht (richtig wäre {check})."));
            }
            let full = format!("{}{check}", &digits[..n]);
            let bars = match kind {
                BarcodeKind::Ean8 => barcoders::sym::ean8::EAN8::new(&full).map(|c| c.encode()),
                // UPC-A ist ein EAN-13 mit führender 0
                BarcodeKind::UpcA => barcoders::sym::ean13::EAN13::new(format!("0{full}")).map(|c| c.encode()),
                _ => barcoders::sym::ean13::EAN13::new(&full).map(|c| c.encode()),
            }
            .map_err(|_| format!("Der Inhalt kann nicht als {} kodiert werden.", kind.label()))?;
            Ok(Modules::from_bars(bars, full))
        }
    }
}

/// Prüfziffer nach Modulo 10 (EAN/UPC): von rechts abwechselnd Gewicht 3 und 1.
fn check_digit(digits: &[u8]) -> u8 {
    let sum: u32 = digits.iter().rev().enumerate().map(|(i, &d)| d as u32 * if i % 2 == 0 { 3 } else { 1 }).sum();
    ((10 - sum % 10) % 10) as u8
}

/// Wählt die Zeichensätze für Code 128 (Schreibweise von `barcoders`: Ɓ = Satz B, Ć = Satz C):
/// längere Ziffernfolgen paarweise in Satz C, alles andere in Satz B.
fn code128_sets(content: &str) -> String {
    let chars: Vec<char> = content.chars().collect();
    let mut out = String::new();
    let mut set = None;
    let mut switch = |out: &mut String, s: char| {
        if set != Some(s) {
            out.push(s);
            set = Some(s);
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let run = chars[i..].iter().take_while(|c| c.is_ascii_digit()).count();
        let edge = i == 0 || i + run == chars.len();
        if run >= 6 || (run >= 4 && edge) || (run == chars.len() && run % 2 == 0) {
            // ungerade Anzahl: die erste Ziffer noch in Satz B
            if run % 2 == 1 {
                switch(&mut out, 'Ɓ');
                out.push(chars[i]);
                i += 1;
            }
            switch(&mut out, 'Ć');
            out.extend(&chars[i..i + run / 2 * 2]);
            i += run / 2 * 2;
        } else {
            switch(&mut out, 'Ɓ');
            let n = run.max(1);
            out.extend(&chars[i..i + n]);
            i += n;
        }
    }
    out
}

/// Größe des Codes für die Anzeige im Dialog.
pub fn barcode_info(spec: &BarcodeSpec, content: &str) -> Result<BarcodeInfo, String> {
    let m = encode(spec.kind, spec.level, content)?;
    let (w, h) = code_size(spec, &m);
    Ok(BarcodeInfo { modules: (m.width, m.height), size_mm: (w, h) })
}

/// Rand um den Code in mm (nur beim Invertieren: dann wird die Ruhezone mitgraviert).
fn margins(spec: &BarcodeSpec) -> (f64, f64) {
    if !spec.invert {
        return (0.0, 0.0);
    }
    let q = spec.kind.quiet_zone() as f64 * spec.module_mm;
    // 1D-Codes brauchen die Ruhezone nur links und rechts
    if spec.kind.is_2d() { (q, q) } else { (q, 2.0 * spec.module_mm) }
}

fn code_size(spec: &BarcodeSpec, m: &Modules) -> (f64, f64) {
    let (mx, my) = margins(spec);
    let h = if spec.kind.is_2d() { m.height as f64 * spec.module_mm } else { spec.height_mm };
    (m.width as f64 * spec.module_mm + 2.0 * mx, h + 2.0 * my)
}

/// Gravierte Flächen des Codes in mm, linke obere Ecke bei (0, 0).
fn code_outlines(spec: &BarcodeSpec, m: &Modules) -> Vec<Polyline> {
    let s = spec.module_mm.max(0.01);
    let (mx, my) = margins(spec);
    let mut polys: Vec<Polyline> = if spec.kind.is_2d() {
        grid_outlines(m)
            .into_iter()
            .map(|lp| {
                let pts = lp.into_iter().map(|(x, y)| Point::new(mx + x as f64 * s, my + y as f64 * s)).collect();
                Polyline::new(pts, true)
            })
            .collect()
    } else {
        // zusammenhängende dunkle Module ergeben einen Strich
        let mut bars = Vec::new();
        let mut x = 0;
        while x < m.width {
            if !m.is_dark(x as i64, 0) {
                x += 1;
                continue;
            }
            let start = x;
            while x < m.width && m.is_dark(x as i64, 0) {
                x += 1;
            }
            let (x0, x1, y0, y1) = (mx + start as f64 * s, mx + x as f64 * s, my, my + spec.height_mm);
            bars.push(Polyline::new(
                vec![Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)],
                true,
            ));
        }
        bars
    };
    if spec.invert {
        // Rahmen um alles: nach der Gerade-Ungerade-Regel werden dann die hellen Module graviert
        let (w, h) = code_size(spec, m);
        polys.push(Polyline::new(
            vec![Point::new(0.0, 0.0), Point::new(w, 0.0), Point::new(w, h), Point::new(0.0, h)],
            true,
        ));
    }
    if spec.shrink_mm > 0.0 {
        polys = offset(&polys, -spec.shrink_mm / 2.0, Corner::Sharp);
    }
    polys
}

/// Umrisse der dunklen Zellen (Kantenlänge 1, Ursprung oben links, Y nach unten). Jede Zelle liegt rechts ihrer
/// Umrisslinien; an Ecken, an denen sich zwei Zellen nur diagonal berühren, biegt der Umriss ab, die Zellen
/// bleiben also getrennte Flächen. Gerade Stücke werden zu einer Strecke zusammengefasst.
fn grid_outlines(m: &Modules) -> Vec<Vec<(i64, i64)>> {
    let mut next: HashMap<(i64, i64), Vec<(i64, i64)>> = HashMap::new();
    let mut add = |a: (i64, i64), b: (i64, i64)| next.entry(a).or_default().push(b);
    for y in 0..m.height as i64 {
        for x in 0..m.width as i64 {
            if !m.is_dark(x, y) {
                continue;
            }
            if !m.is_dark(x, y - 1) {
                add((x, y), (x + 1, y));
            }
            if !m.is_dark(x + 1, y) {
                add((x + 1, y), (x + 1, y + 1));
            }
            if !m.is_dark(x, y + 1) {
                add((x + 1, y + 1), (x, y + 1));
            }
            if !m.is_dark(x - 1, y) {
                add((x, y + 1), (x, y));
            }
        }
    }
    let mut starts: Vec<(i64, i64)> = next.keys().copied().collect();
    starts.sort_by_key(|&(x, y)| (y, x));
    let mut loops = Vec::new();
    for s in starts {
        while let Some(first) = next.get_mut(&s).and_then(Vec::pop) {
            let mut pts = vec![s];
            let (mut prev, mut cur) = (s, first);
            while cur != s {
                pts.push(cur);
                let d = (cur.0 - prev.0, cur.1 - prev.1);
                // Rechtskurve (auf dem Bildschirm) bevorzugen
                let right = (cur.0 - d.1, cur.1 + d.0);
                let outs = next.get_mut(&cur).expect("geschlossener Umriss");
                let i = outs.iter().position(|&p| p == right).unwrap_or(0);
                (prev, cur) = (cur, outs.swap_remove(i));
            }
            loops.push(drop_collinear(pts));
        }
    }
    loops
}

/// Entfernt Punkte, an denen sich die Richtung nicht ändert.
fn drop_collinear(pts: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    let n = pts.len();
    let dir = |a: (i64, i64), b: (i64, i64)| ((b.0 - a.0).signum(), (b.1 - a.1).signum());
    (0..n)
        .filter(|&i| {
            let (a, p, b) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            dir(a, p) != dir(p, b)
        })
        .map(|i| pts[i])
        .collect()
}

/// Erzeugt einen Strichcode mit linker oberer Ecke bei (0, 0). Der Klartext unter 1D-Codes verwendet die Schrift
/// aus `data.spec` und wird mittig unter die Striche gesetzt.
pub fn render_barcode(db: &Database, data: &TextData, spec: &BarcodeSpec, content: &str) -> Result<VectorPath, String> {
    let m = encode(spec.kind, spec.level, content)?;
    let mut pb = PathBuilder::new();
    for p in code_outlines(spec, &m) {
        let mut pts = p.points.iter();
        let Some(&first) = pts.next() else { continue };
        pb.move_to(first);
        for &q in pts {
            pb.line_to(q);
        }
        pb.close();
    }
    let mut path = pb.finish();
    if !spec.kind.is_2d() && spec.show_text && !m.text.is_empty() {
        let (w, h) = code_size(spec, &m);
        let height = (spec.module_mm * 8.0).max(1.5);
        let label = text_to_path(
            db,
            &TextSpec { text: m.text, height_mm: height, align: TextAlign::Center, ..data.spec.clone() },
        )?;
        let b = label.bounds(&Affine::IDENTITY);
        let t = Affine::translate(w / 2.0 - b.center().x, h + height * 0.35 - b.min.y);
        path.append(label.transformed(&t));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::path::point_in_polylines;

    fn spec(kind: BarcodeKind) -> BarcodeSpec {
        BarcodeSpec { kind, module_mm: 1.0, height_mm: 10.0, show_text: false, ..Default::default() }
    }

    /// Prüft, ob die Mitte jedes Moduls genau dann gefüllt ist, wenn das Modul dunkel ist.
    fn assert_matches_modules(spec: &BarcodeSpec, m: &Modules, polys: &[Polyline]) {
        let (mx, my) = margins(spec);
        for y in 0..m.height {
            for x in 0..m.width {
                let c = Point::new(mx + x as f64 + 0.5, my + y as f64 + 0.5);
                let engraved = point_in_polylines(c, polys);
                assert_eq!(engraved, m.is_dark(x as i64, y as i64) != spec.invert, "Modul {x},{y}");
            }
        }
    }

    #[test]
    fn qr_outlines_match_modules() {
        let s = spec(BarcodeKind::Qr);
        let m = encode(s.kind, s.level, "https://example.com/VectorLume").unwrap();
        assert_eq!((m.width, m.height), (29, 29));
        let polys = code_outlines(&s, &m);
        // Umrisse fassen Module zusammen und haben nur Ecken, keine Zwischenpunkte
        let dark = m.dark.iter().filter(|d| **d).count();
        assert!(polys.len() < dark / 3, "{} Umrisse für {dark} Module", polys.len());
        for p in &polys {
            assert!(p.points.len() >= 4 && p.points.len() % 2 == 0);
        }
        assert_matches_modules(&s, &m, &polys);
        let inv = BarcodeSpec { invert: true, ..s };
        let polys = code_outlines(&inv, &m);
        assert_matches_modules(&inv, &m, &polys);
        // Ruhezone wird mitgraviert
        assert!(point_in_polylines(Point::new(0.5, 0.5), &polys));
        let info = barcode_info(&inv, "https://example.com/VectorLume").unwrap();
        assert_eq!(info.size_mm, (37.0, 37.0));
    }

    /// Rastert die gravierten Flächen (dunkel = graviert) und liest den QR-Code mit einem unabhängigen Decoder.
    fn decode_qr(polys: &[Polyline], size: f64, dark_is_engraved: bool) -> String {
        let px = 0.25;
        let n = ((size + 8.0) / px) as usize;
        let img = image::GrayImage::from_fn(n as u32, n as u32, |x, y| {
            let p = Point::new(x as f64 * px - 4.0, y as f64 * px - 4.0);
            let dark = point_in_polylines(p, polys) == dark_is_engraved;
            image::Luma([if dark { 0 } else { 255 }])
        });
        let mut prepared = rqrr::PreparedImage::prepare(img);
        let grids = prepared.detect_grids();
        assert_eq!(grids.len(), 1, "kein QR-Code erkannt");
        grids[0].decode().expect("QR-Code nicht lesbar").1
    }

    #[test]
    fn qr_code_is_readable() {
        let text = "WIFI:T:WPA;S:Werkstatt;P:geheim123;;";
        let mut s = spec(BarcodeKind::Qr);
        let m = encode(s.kind, s.level, text).unwrap();
        let size = m.width as f64;
        assert_eq!(decode_qr(&code_outlines(&s, &m), size, true), text);
        // mit Strichbreiten-Ausgleich und invertiert (helle Module graviert)
        s.shrink_mm = 0.2;
        assert_eq!(decode_qr(&code_outlines(&s, &m), size, true), text);
        s.invert = true;
        let polys: Vec<Polyline> = code_outlines(&s, &m)
            .into_iter()
            .map(|mut p| {
                p.transform(&Affine::translate(-4.0, -4.0));
                p
            })
            .collect();
        assert_eq!(decode_qr(&polys, size, false), text);
    }

    #[test]
    fn diagonal_cells_stay_separate() {
        // Schachbrett 2×2: zwei Zellen, die sich nur an einer Ecke berühren
        let m = Modules { width: 2, height: 2, dark: vec![true, false, false, true], text: String::new() };
        let loops = grid_outlines(&m);
        assert_eq!(loops.len(), 2);
        assert!(loops.iter().all(|l| l.len() == 4));
        // Ring mit Loch: Außenkontur und Lochkontur
        let ring = Modules {
            width: 3,
            height: 3,
            dark: vec![true, true, true, true, false, true, true, true, true],
            text: String::new(),
        };
        let loops = grid_outlines(&ring);
        assert_eq!(loops.len(), 2);
        assert!(loops.iter().all(|l| l.len() == 4));
    }

    #[test]
    fn shrink_reduces_bar_width() {
        let mut s = spec(BarcodeKind::Code128);
        let m = encode(s.kind, s.level, "VL-2026").unwrap();
        let plain = code_outlines(&s, &m);
        s.shrink_mm = 0.2;
        let thin = code_outlines(&s, &m);
        assert_eq!(plain.len(), thin.len());
        // Reihenfolge nach dem Versatz ist beliebig: nach linker Kante sortiert vergleichen
        let sorted = |polys: &[Polyline]| {
            let mut b: Vec<_> = polys.iter().map(|p| p.bounds()).collect();
            b.sort_by(|a, b| a.min.x.total_cmp(&b.min.x));
            b
        };
        for (a, b) in sorted(&plain).iter().zip(&sorted(&thin)) {
            assert!((a.width() - b.width() - 0.2).abs() < 1e-6, "{} -> {}", a.width(), b.width());
            assert!((a.min.x + 0.1 - b.min.x).abs() < 1e-6);
        }
    }

    #[test]
    fn one_dimensional_codes() {
        // EAN-13: 95 Module, Prüfziffer wird ergänzt bzw. geprüft
        let m = encode(BarcodeKind::Ean13, QrLevel::M, "400638133393").unwrap();
        assert_eq!((m.width, m.text.as_str()), (95, "4006381333931"));
        assert!(encode(BarcodeKind::Ean13, QrLevel::M, "4006381333931").is_ok());
        assert!(encode(BarcodeKind::Ean13, QrLevel::M, "4006381333932").unwrap_err().contains("wäre 1"));
        assert!(encode(BarcodeKind::Ean13, QrLevel::M, "12345").is_err());
        assert_eq!(encode(BarcodeKind::Ean8, QrLevel::M, "9638507").unwrap().text, "96385074");
        let upc = encode(BarcodeKind::UpcA, QrLevel::M, "03600029145").unwrap();
        assert_eq!((upc.width, upc.text.as_str()), (95, "036000291452"));
        // Code 39 in Großbuchstaben, Code 128 nur ASCII
        assert_eq!(encode(BarcodeKind::Code39, QrLevel::M, "abc-1").unwrap().text, "ABC-1");
        assert!(encode(BarcodeKind::Code128, QrLevel::M, "Größe").is_err());
        assert!(encode(BarcodeKind::Qr, QrLevel::M, "").is_err());
        let s = spec(BarcodeKind::Ean13);
        let m = encode(s.kind, s.level, "400638133393").unwrap();
        let polys = code_outlines(&s, &m);
        assert_matches_modules(&s, &m, &polys);
        assert!(polys.iter().all(|p| (p.bounds().height() - 10.0).abs() < 1e-9));
    }

    #[test]
    fn code128_charset_choice() {
        assert_eq!(code128_sets("123456"), "Ć123456");
        assert_eq!(code128_sets("12345"), "Ɓ1Ć2345");
        assert_eq!(code128_sets("AB12"), "ƁAB12");
        assert_eq!(code128_sets("AB1234"), "ƁABĆ1234");
        assert_eq!(code128_sets("A1234567B"), "ƁA1Ć234567ƁB");
        // kürzer als nur Satz B
        let len = |s: &str| barcoders::sym::code128::Code128::new(s).unwrap().encode().len();
        assert!(len(&code128_sets("SN 20260001")) < len("ƁSN 20260001"));
        assert!(len(&code128_sets("SN 20260001")) > 0);
    }

    #[test]
    fn data_matrix_and_label() {
        let s = spec(BarcodeKind::DataMatrix);
        let m = encode(s.kind, s.level, "VL-0001").unwrap();
        assert!(m.width >= 10 && m.height >= 10);
        assert_matches_modules(&s, &m, &code_outlines(&s, &m));
        let db = crate::text::system_fonts();
        let Some(family) = crate::text::any_family(&db) else { return };
        let data = TextData { spec: TextSpec { family, ..Default::default() }, ..Default::default() };
        let s = BarcodeSpec { show_text: true, ..spec(BarcodeKind::Code39) };
        let p = render_barcode(&db, &data, &s, "VL1").unwrap();
        let b = p.bounds(&Affine::IDENTITY);
        // Klartext liegt unter den Strichen und ist mittig
        assert!(b.max.y > 15.0 && b.min.y.abs() < 1e-9, "{b:?}");
    }
}
