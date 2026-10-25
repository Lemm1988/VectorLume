//! Excellon-Bohrdateien (Leiterplatten): Bohrungen als Kreise im Werkzeugdurchmesser, Langlöcher (G85 oder
//! Fräsweg M15…M16) als Langloch-Umriss. Je Werkzeug ein Pfad; Y wird wie bei Gerber gespiegelt, damit Bohr- und
//! Kupferlage deckungsgleich bleiben.

use crate::{Imported, ImportedItem};
use std::collections::BTreeMap;
use vl_core::ops::{self, Corner};
use vl_core::{Point, Polyline, VectorPath};

pub const DRILL_EXTENSIONS: &[&str] = &["drl", "xln", "exc", "drd"];

/// Bohrungen (Mittelpunkte) und Langlöcher (Anfang, Ende) eines Werkzeugs.
type Holes = (Vec<Point>, Vec<(Point, Point)>);

#[derive(Clone, Copy)]
struct Format {
    int_digits: u32,
    dec_digits: u32,
    /// LZ: führende Nullen stehen in der Datei, nachfolgende fehlen.
    leading_kept: bool,
}

fn number(fmt: Format, s: &str) -> Result<f64, String> {
    let bad = || format!("Koordinate „{s}“ nicht lesbar.");
    if s.contains('.') {
        return s.parse::<f64>().map_err(|_| bad());
    }
    let (neg, digits) = match s.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    let total = (fmt.int_digits + fmt.dec_digits) as usize;
    let digits = if fmt.leading_kept { format!("{digits:0<total$}") } else { digits.to_string() };
    let v = digits.parse::<f64>().map_err(|_| bad())? / 10f64.powi(fmt.dec_digits as i32);
    Ok(if neg { -v } else { v })
}

/// Erkennt Excellon am Kopf `M48`.
pub fn looks_like_excellon(data: &[u8]) -> bool {
    String::from_utf8_lossy(&data[..data.len().min(2048)]).lines().any(|l| l.trim() == "M48")
}

/// Zerlegt eine Zeile in Buchstabe + Wert, z. B. „X1.5Y-2G85X3Y4“.
fn words(line: &str) -> Vec<(char, &str)> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let letter = b[i] as char;
        let mut j = i + 1;
        while j < b.len() && (b[j].is_ascii_digit() || matches!(b[j], b'-' | b'+' | b'.')) {
            j += 1;
        }
        if letter.is_ascii_alphabetic() {
            out.push((letter.to_ascii_uppercase(), &line[i + 1..j]));
        }
        i = j;
    }
    out
}

pub fn import_excellon(data: &[u8], name: &str) -> Result<Imported, String> {
    let text = String::from_utf8_lossy(data);
    if !looks_like_excellon(data) {
        return Err("Keine Excellon-Bohrdatei (Kopf M48 fehlt).".into());
    }
    let mut metric = true;
    let mut fmt: Option<Format> = None;
    let mut leading_kept = false;
    let mut tools: BTreeMap<u32, f64> = BTreeMap::new();
    // je Werkzeug: Bohrungen und Langlöcher (Anfang, Ende)
    let mut holes: BTreeMap<u32, Holes> = BTreeMap::new();
    let mut tool = None;
    let mut pos = Point::ZERO;
    let mut header = false;
    let mut routing = false;
    let mut plunged = false;
    let mut warnings = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        // KiCad: „; FORMAT={-:-/ absolute / metric / decimal}“ bzw. „;FILE_FORMAT=4:3“
        if let Some(c) = line.strip_prefix(';') {
            if let Some(f) = c.trim().strip_prefix("FILE_FORMAT=")
                && let Some((a, d)) = f.split_once(':')
                && let (Ok(a), Ok(d)) = (a.trim().parse(), d.trim().parse())
            {
                fmt = Some(Format { int_digits: a, dec_digits: d, leading_kept });
            }
            continue;
        }
        match line {
            "M48" => {
                header = true;
                continue;
            }
            "%" | "M95" => {
                header = false;
                continue;
            }
            "M30" | "M00" => break,
            "M71" | "METRIC" => {
                metric = true;
                continue;
            }
            "M72" | "INCH" => {
                metric = false;
                continue;
            }
            "M15" => {
                plunged = true;
                continue;
            }
            "M16" | "M17" => {
                plunged = false;
                continue;
            }
            "G05" | "G5" => {
                routing = false;
                continue;
            }
            _ => {}
        }
        if let Some(rest) = line.strip_prefix("METRIC").or_else(|| line.strip_prefix("INCH")) {
            metric = line.starts_with("METRIC");
            for part in rest.split(',').map(str::trim) {
                match part {
                    "LZ" => leading_kept = true,
                    "TZ" => leading_kept = false,
                    p if p.contains('.') => {
                        // „000.000“: Stellen vor und nach dem Punkt
                        let (a, d) = p.split_once('.').unwrap_or((p, ""));
                        fmt = Some(Format { int_digits: a.len() as u32, dec_digits: d.len() as u32, leading_kept });
                    }
                    _ => {}
                }
            }
            if let Some(f) = &mut fmt {
                f.leading_kept = leading_kept;
            }
            continue;
        }
        let w = words(line);
        // Werkzeugdefinition „T1C0.8“ (auch mit F/S dazwischen) im Kopf oder im Programm
        if let Some((_, t)) = w.first().filter(|(l, _)| *l == 'T') {
            let n: u32 = t.parse().map_err(|_| format!("Werkzeug „{line}“ nicht lesbar."))?;
            if let Some((_, c)) = w.iter().find(|(l, _)| *l == 'C') {
                let d: f64 = c.parse().map_err(|_| format!("Werkzeug „{line}“ nicht lesbar."))?;
                tools.insert(n, d * if metric { 1.0 } else { 25.4 });
            }
            if !header {
                tool = (n != 0).then_some(n);
            }
            continue;
        }
        if header {
            continue;
        }
        let f = fmt.unwrap_or(if metric {
            Format { int_digits: 3, dec_digits: 3, leading_kept }
        } else {
            Format { int_digits: 2, dec_digits: 4, leading_kept }
        });
        let unit = if metric { 1.0 } else { 25.4 };
        let mut g = None;
        let mut first = pos;
        let mut second: Option<Point> = None;
        for (l, v) in &w {
            let target = if second.is_some() || g == Some(85) { second.get_or_insert(first) } else { &mut first };
            match l {
                'G' => g = v.parse::<u32>().ok().or(g),
                'X' => target.x = number(f, v)? * unit,
                'Y' => target.y = number(f, v)? * unit,
                _ => {}
            }
        }
        if !w.iter().any(|(l, _)| matches!(l, 'X' | 'Y')) {
            if g == Some(0) {
                routing = true;
            }
            continue;
        }
        let Some(t) = tool else {
            warnings.push("Koordinaten ohne gewähltes Werkzeug wurden übersprungen.".into());
            continue;
        };
        let entry = holes.entry(t).or_default();
        match (g, second) {
            (Some(85), Some(end)) => entry.1.push((first, end)),
            (Some(0), _) => routing = true,
            (Some(1..=3), _) | (None, _) if routing && plunged => entry.1.push((pos, first)),
            _ if routing => {}
            _ => entry.0.push(first),
        }
        pos = second.unwrap_or(first);
    }
    let mut out = Imported { warnings, ..Default::default() };
    for (t, (drills, slots)) in holes {
        let Some(&d) = tools.get(&t) else {
            out.warnings.push(format!("Werkzeug T{t} hat keinen Durchmesser und wurde übersprungen."));
            continue;
        };
        if d <= 0.0 {
            continue;
        }
        let mut path = VectorPath::default();
        let flip = |p: Point| Point::new(p.x, -p.y);
        for p in drills {
            path.append(VectorPath::ellipse(flip(p), d / 2.0, d / 2.0));
        }
        for (a, b) in slots {
            let line = Polyline::new(vec![flip(a), flip(b)], false);
            for o in ops::offset(&[line], d / 2.0, Corner::Round) {
                path.append(VectorPath::polygon(&o.points, true));
            }
        }
        if !path.subpaths.is_empty() {
            let label = format!("{name} T{t} Ø{d:.2}");
            out.items.push(ImportedItem::Path { name: label, layer: None, path });
        }
    }
    if out.items.is_empty() {
        return Err("Die Bohrdatei enthält keine Bohrungen.".into());
    }
    Ok(out)
}

/// Für Tests: ungefähre Anzahl Kreise einer Bohrung (ohne Langlöcher) erkennen.
#[cfg(test)]
fn holes_of(item: &ImportedItem) -> usize {
    match item {
        ImportedItem::Path { path, .. } => path.subpaths.len(),
        ImportedItem::Image { .. } => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::Affine;

    #[test]
    fn kicad_decimal_metric() {
        let src = "M48\n; DRILL file {KiCad 8}\n;FORMAT={-:-/ absolute / metric / decimal}\nFMAT,2\nMETRIC\n\
                   T1C0.800\nT2C1.000\n%\nG90\nG05\nT1\nX10.0Y-5.0\nX20.0Y-5.0\nT2\nX15.0Y-10.0\n\
                   T2\nG00X30.0Y-10.0\nM15\nG01X34.0Y-10.0\nM16\nG05\nT0\nM30\n";
        let imp = import_excellon(src.as_bytes(), "b.drl").unwrap();
        assert_eq!(imp.items.len(), 2);
        assert_eq!(holes_of(&imp.items[0]), 2);
        assert_eq!(holes_of(&imp.items[1]), 2, "Bohrung und Langloch");
        let ImportedItem::Path { name, path, .. } = &imp.items[0] else { panic!() };
        assert_eq!(name, "b.drl T1 Ø0.80");
        let b = path.bounds(&Affine::IDENTITY);
        assert!((b.min.x - 9.6).abs() < 1e-9 && (b.max.x - 20.4).abs() < 1e-9 && (b.center().y - 5.0).abs() < 1e-9);
        let ImportedItem::Path { path, .. } = &imp.items[1] else { panic!() };
        let b = path.bounds(&Affine::IDENTITY);
        assert!((b.max.x - 34.5).abs() < 0.01 && (b.min.x - 14.5).abs() < 1e-9, "{b:?}");
    }

    #[test]
    fn inch_implicit_formats_and_g85() {
        // Zoll mit führenden Nullen (LZ), Format 2.4: „X01Y005“ → 01|0000 und 00|5000 = 1.0 und 0.5 Zoll
        let src = "M48\nINCH,LZ\nT01C0.0400\n%\nT01\nX01Y005\nX015Y005G85X02Y005\nM30\n";
        let imp = import_excellon(src.as_bytes(), "z.drl").unwrap();
        let ImportedItem::Path { path, .. } = &imp.items[0] else { panic!() };
        assert_eq!(path.subpaths.len(), 2);
        let b = path.bounds(&Affine::IDENTITY);
        let r = 0.02 * 25.4;
        assert!((b.min.x - (25.4 - r)).abs() < 1e-6 && (b.max.x - (50.8 + r)).abs() < 0.01, "{b:?}");
        assert!((b.center().y + 12.7).abs() < 1e-6);
        // Metrisch ohne Punkt, TZ (führende Nullen fehlen), Format 3.3: „X1500“ = 1.5 mm
        let src = "M48\nMETRIC,TZ,000.000\nT1C1.0\n%\nT1\nX1500Y-2000\nM30\n";
        let imp = import_excellon(src.as_bytes(), "m.drl").unwrap();
        let ImportedItem::Path { path, .. } = &imp.items[0] else { panic!() };
        assert!(path.bounds(&Affine::IDENTITY).center().distance(Point::new(1.5, 2.0)) < 1e-9);
    }

    #[test]
    fn errors() {
        assert!(import_excellon(b"G90\nX1Y1\n", "x").is_err());
        assert!(import_excellon(b"M48\nMETRIC\n%\nM30\n", "x").is_err());
    }
}
