//! Import von HPGL-Plotterdateien (.plt/.hpgl): Stiftbewegungen PU/PD/PA/PR, Kreise (CI) und Bögen (AA/AR).
//! Jeder Stift (SP) wird ein eigenes Objekt auf einer eigenen Ebene.

use crate::{Imported, ImportedItem};
use std::collections::BTreeMap;
use vl_core::{Affine, LAYER_COUNT, PathBuilder, Point, VectorPath};

/// Plottereinheit: 1/40 mm.
const UNIT_MM: f64 = 0.025;

#[derive(Default)]
struct Pen {
    pb: PathBuilder,
    /// Endpunkt des zuletzt gezeichneten Stücks (zum Fortsetzen ohne neuen Teilpfad).
    last: Option<Point>,
}

impl Pen {
    fn line(&mut self, from: Point, to: Point) {
        if self.last != Some(from) {
            self.pb.move_to(from);
        }
        self.pb.line_to(to);
        self.last = Some(to);
    }
}

pub fn import_hpgl(data: &[u8]) -> Result<Imported, String> {
    let text = String::from_utf8_lossy(data);
    let mut pens: BTreeMap<u32, Pen> = BTreeMap::new();
    let mut pen = 1u32;
    let mut down = false;
    let mut absolute = true;
    let mut pos = Point::ZERO;
    let mut unknown = 0usize;

    for cmd in commands(&text) {
        let (name, args) = cmd;
        let nums = numbers(args);
        let pts = || nums.as_chunks::<2>().0.iter().map(|&[x, y]| Point::new(x, y)).collect::<Vec<_>>();
        match name.as_str() {
            "IN" | "DF" => {
                down = false;
                absolute = true;
            }
            "SP" => pen = nums.first().map(|&n| n.max(0.0) as u32).unwrap_or(0),
            "PU" | "PD" | "PA" | "PR" => {
                match name.as_str() {
                    "PU" => down = false,
                    "PD" => down = true,
                    "PA" => absolute = true,
                    _ => absolute = false,
                }
                for p in pts() {
                    let to = if absolute { p } else { pos + p };
                    if down && pen > 0 {
                        pens.entry(pen).or_default().line(pos, to);
                    }
                    pos = to;
                }
            }
            "CI" => {
                if let Some(&r) = nums.first()
                    && r.abs() > 0.0
                    && pen > 0
                {
                    let p = pens.entry(pen).or_default();
                    p.pb.ellipse(pos, r.abs(), r.abs());
                    p.last = None;
                }
            }
            "AA" | "AR" => {
                if nums.len() >= 3 {
                    let c = Point::new(nums[0], nums[1]);
                    let center = if name == "AA" { c } else { pos + c };
                    let sweep = nums[2].to_radians();
                    let r = center.distance(pos);
                    let start = (pos.y - center.y).atan2(pos.x - center.x);
                    let end = center + Point::new((start + sweep).cos(), (start + sweep).sin()) * r;
                    if down && pen > 0 && r > 0.0 {
                        let p = pens.entry(pen).or_default();
                        if p.last != Some(pos) {
                            p.pb.move_to(pos);
                        }
                        p.pb.arc(center, r, r, 0.0, start, sweep);
                        p.last = Some(end);
                    }
                    pos = end;
                }
            }
            "" => {}
            _ => unknown += 1,
        }
    }

    // Plottereinheiten → mm, y nach unten (Oberkante der Zeichnung bei y = 0)
    let mut paths: Vec<(u32, VectorPath)> =
        pens.into_iter().map(|(n, p)| (n, p.pb.finish())).filter(|(_, p)| !p.is_empty()).collect();
    let top = paths.iter().map(|(_, p)| p.bounds(&Affine::IDENTITY).max.y).fold(f64::NEG_INFINITY, f64::max);
    let tf = Affine([UNIT_MM, 0.0, 0.0, -UNIT_MM, 0.0, top * UNIT_MM]);
    let mut out = Imported::default();
    for (n, path) in paths.drain(..) {
        out.items.push(ImportedItem::Path {
            name: format!("Stift {n}"),
            layer: Some((n as usize - 1) % (LAYER_COUNT - 1)),
            path: path.transformed(&tf),
        });
    }
    if out.items.is_empty() {
        return Err("In der HPGL-Datei wurden keine gezeichneten Linien gefunden.".into());
    }
    if unknown > 0 {
        out.warnings.push(format!("{unknown} HPGL-Befehl(e) werden nicht unterstützt und wurden übersprungen."));
    }
    Ok(out)
}

/// Zerlegt den Text in (Befehl, Argumente). Befehle sind zwei Buchstaben, getrennt durch `;` oder den
/// nächsten Befehl. Beschriftungen (LB … Endzeichen) werden übersprungen.
fn commands(text: &str) -> Vec<(String, &str)> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        if i + 1 >= b.len() || !b[i + 1].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let name = text[i..i + 2].to_ascii_uppercase();
        i += 2;
        if name == "LB" {
            // Beschriftung bis zum Endzeichen (Standard ETX = 0x03)
            while i < b.len() && b[i] != 0x03 {
                i += 1;
            }
            out.push((name, ""));
            continue;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_alphabetic() && b[i] != b';' {
            i += 1;
        }
        out.push((name, &text[start..i]));
    }
    out
}

fn numbers(args: &str) -> Vec<f64> {
    args.split(|c: char| c == ',' || c.is_whitespace()).filter_map(|s| s.trim().parse().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_and_circle_per_pen() {
        let plt = b"IN;SP1;PU0,0;PD400,0,400,400,0,400,0,0;PU;SP2;PA2000,2000;CI200;SP0;";
        let out = import_hpgl(plt).unwrap();
        assert_eq!(out.items.len(), 2);
        let ImportedItem::Path { path, layer, .. } = &out.items[0] else { panic!() };
        assert_eq!(*layer, Some(0));
        let b = path.bounds(&Affine::IDENTITY);
        assert!((b.width() - 10.0).abs() < 1e-9 && (b.height() - 10.0).abs() < 1e-9);
        let ImportedItem::Path { path, layer, .. } = &out.items[1] else { panic!() };
        assert_eq!(*layer, Some(1));
        let b = path.bounds(&Affine::IDENTITY);
        assert!((b.width() - 10.0).abs() < 0.01, "{b:?}");
        // höchster Punkt der Zeichnung (Kreis oben bei 2200) liegt bei y = 0
        assert!(b.min.y.abs() < 0.01);
    }

    #[test]
    fn relative_moves_and_arcs() {
        let plt = b"IN;SP1;PA0,0;PD;PR400,0;AA0,0,90;PU;";
        let out = import_hpgl(plt).unwrap();
        let ImportedItem::Path { path, .. } = &out.items[0] else { panic!() };
        let b = path.bounds(&Affine::IDENTITY);
        // Linie 10 mm nach rechts, dann Viertelkreis um den Ursprung bis (0, 10 mm)
        assert!((b.width() - 10.0).abs() < 0.01 && (b.height() - 10.0).abs() < 0.01, "{b:?}");
        assert_eq!(path.subpaths.len(), 1);
    }
}
