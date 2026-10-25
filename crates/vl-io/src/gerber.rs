//! Gerber RS-274X (Leiterplatten): Kupfer-, Lötstopp- und Bestückungsdruck-Lagen werden zu gefüllten Flächen
//! vereinigt (Blenden, Leiterbahnen, Regionen, Löschen mit Polarität „clear“). Der Platinenumriss (Profil) wird als
//! Mittellinie übernommen, damit er sich direkt schneiden lässt.
//!
//! Gerber zählt Y nach oben, das Dokument nach unten: Y wird gespiegelt, die Lage der Dateien zueinander bleibt.

use crate::{Imported, ImportedItem};
use std::collections::HashMap;
use std::f64::consts::{PI, TAU};
use vl_core::ops::{self, BoolOp, Corner};
use vl_core::{Point, Polyline, VectorPath};

/// Übliche Endungen (KiCad, Eagle, Altium, EasyEDA).
pub const GERBER_EXTENSIONS: &[&str] =
    &["gbr", "ger", "gtl", "gbl", "gts", "gbs", "gto", "gbo", "gtp", "gbp", "gko", "gm1", "gml", "gm", "pho"];
/// Endungen, die üblicherweise den Platinenumriss enthalten.
pub const PROFILE_EXTENSIONS: &[&str] = &["gko", "gm1", "gml", "gm"];

/// Sehnenabweichung beim Annähern von Bögen (mm).
const TOL: f64 = 0.005;

/// Anzahl Sehnen für einen Bogen mit Radius `r` und Winkel `sweep`.
fn segments(r: f64, sweep: f64) -> usize {
    if r < 1e-9 {
        return 1;
    }
    let step = 2.0 * (1.0 - TOL / r).clamp(-1.0, 1.0).acos();
    ((sweep.abs() / step.max(1e-3)).ceil() as usize).clamp(1, 4000)
}

fn circle(c: Point, d: f64) -> Vec<Point> {
    let r = d / 2.0;
    let n = segments(r, TAU).max(8);
    (0..n).map(|i| c + Point::new(r, 0.0).rotated(TAU * i as f64 / n as f64)).collect()
}

/// Punkte eines Bogens um `center` von `from` nach `to` (ohne Anfangs-, mit Endpunkt). Abweichende Radien an Anfang
/// und Ende (Rundungsfehler der Datei) werden ausgeglichen.
fn arc_points(center: Point, from: Point, to: Point, ccw: bool, full: bool) -> Vec<Point> {
    let (r0, r1) = (from.distance(center), to.distance(center));
    let a0 = (from.y - center.y).atan2(from.x - center.x);
    let a1 = (to.y - center.y).atan2(to.x - center.x);
    let mut sweep = a1 - a0;
    if full {
        sweep = if ccw { TAU } else { -TAU };
    } else if ccw && sweep <= 0.0 {
        sweep += TAU;
    } else if !ccw && sweep >= 0.0 {
        sweep -= TAU;
    }
    let n = segments(r0.max(r1), sweep);
    let mut pts: Vec<Point> = (1..n)
        .map(|i| {
            let t = i as f64 / n as f64;
            center + Point::new(r0 + (r1 - r0) * t, 0.0).rotated(a0 + sweep * t)
        })
        .collect();
    pts.push(to);
    pts
}

fn closed(points: Vec<Point>) -> Polyline {
    Polyline::new(points, true)
}

/// Achsparalleles Rechteck mit Mitte `c`.
fn rect(c: Point, w: f64, h: f64) -> Vec<Point> {
    let (x, y) = (w / 2.0, h / 2.0);
    [(-x, -y), (x, -y), (x, y), (-x, y)].iter().map(|(dx, dy)| c + Point::new(*dx, *dy)).collect()
}

/// Dreht um den Ursprung (Grad, gegen den Uhrzeigersinn).
fn rotate_all(polys: &mut [Polyline], deg: f64) {
    if deg != 0.0 {
        let a = deg.to_radians();
        for q in polys.iter_mut().flat_map(|p| p.points.iter_mut()) {
            *q = q.rotated(a);
        }
    }
}

fn scale_all(polys: &mut [Polyline], f: f64) {
    for q in polys.iter_mut().flat_map(|p| p.points.iter_mut()) {
        *q = *q * f;
    }
}

/// Fläche einer Blende im eigenen Koordinatensystem (Gerade-Ungerade-Regel, Löcher als innere Konturen).
#[derive(Clone, Debug)]
enum Aperture {
    /// Kreis: Leiterbahnen werden mit runden Enden umrandet.
    Circle {
        d: f64,
        shape: Vec<Polyline>,
    },
    Shape(Vec<Polyline>),
}

impl Aperture {
    fn shape(&self) -> &[Polyline] {
        match self {
            Aperture::Circle { shape, .. } | Aperture::Shape(shape) => shape,
        }
    }
}

fn with_hole(mut shape: Vec<Polyline>, hole: Option<f64>) -> Vec<Polyline> {
    if let Some(h) = hole.filter(|h| *h > 0.0) {
        shape.push(closed(circle(Point::ZERO, h)));
    }
    shape
}

fn obround(w: f64, h: f64) -> Vec<Polyline> {
    let r = w.min(h) / 2.0;
    let (dx, dy) = if w >= h { ((w - h) / 2.0, 0.0) } else { (0.0, (h - w) / 2.0) };
    if dx == 0.0 && dy == 0.0 {
        return vec![closed(circle(Point::ZERO, 2.0 * r))];
    }
    let line = Polyline::new(vec![Point::new(-dx, -dy), Point::new(dx, dy)], false);
    ops::offset(&[line], r, Corner::Round)
}

fn regular(d: f64, n: usize, rot_deg: f64) -> Vec<Point> {
    (0..n).map(|i| Point::new(d / 2.0, 0.0).rotated(rot_deg.to_radians() + TAU * i as f64 / n as f64)).collect()
}

// ------------------------------------------------------------------ Blendenmakros

/// Rechenausdruck in Makros: Zahlen, `$n`, + - x X / und Klammern.
fn eval(expr: &str, vars: &HashMap<usize, f64>) -> Result<f64, String> {
    let chars: Vec<char> = expr.chars().filter(|c| !c.is_whitespace()).collect();
    let mut pos = 0;
    let v = sum(&chars, &mut pos, vars);
    match v {
        Some(v) if pos == chars.len() => Ok(v),
        _ => Err(format!("Makro-Ausdruck „{expr}“ nicht lesbar.")),
    }
}

fn sum(c: &[char], pos: &mut usize, vars: &HashMap<usize, f64>) -> Option<f64> {
    let mut v = product(c, pos, vars)?;
    while let Some(&op) = c.get(*pos).filter(|o| matches!(o, '+' | '-')) {
        *pos += 1;
        let r = product(c, pos, vars)?;
        v = if op == '+' { v + r } else { v - r };
    }
    Some(v)
}

fn product(c: &[char], pos: &mut usize, vars: &HashMap<usize, f64>) -> Option<f64> {
    let mut v = factor(c, pos, vars)?;
    while let Some(&op) = c.get(*pos).filter(|o| matches!(o, 'x' | 'X' | '/')) {
        *pos += 1;
        let r = factor(c, pos, vars)?;
        v = if op == '/' { v / r } else { v * r };
    }
    Some(v)
}

fn factor(c: &[char], pos: &mut usize, vars: &HashMap<usize, f64>) -> Option<f64> {
    let take_while = |pos: &mut usize, f: fn(char) -> bool| {
        let start = *pos;
        while c.get(*pos).is_some_and(|d| f(*d)) {
            *pos += 1;
        }
        c[start..*pos].iter().collect::<String>()
    };
    match c.get(*pos)? {
        '-' => {
            *pos += 1;
            Some(-factor(c, pos, vars)?)
        }
        '+' => {
            *pos += 1;
            factor(c, pos, vars)
        }
        '(' => {
            *pos += 1;
            let v = sum(c, pos, vars)?;
            (c.get(*pos) == Some(&')')).then(|| *pos += 1)?;
            Some(v)
        }
        '$' => {
            *pos += 1;
            let n: usize = take_while(pos, |d| d.is_ascii_digit()).parse().ok()?;
            Some(vars.get(&n).copied().unwrap_or(0.0))
        }
        d if d.is_ascii_digit() || *d == '.' => take_while(pos, |d| d.is_ascii_digit() || d == '.').parse().ok(),
        _ => None,
    }
}

/// Wertet ein Makro mit den Parametern der Blendendefinition aus: Primitive mit Belichtung „an“ werden vereinigt,
/// mit „aus“ abgezogen. Ergebnis in Dateieinheiten.
fn macro_shape(body: &[String], params: &[f64], warnings: &mut Vec<String>) -> Result<Vec<Polyline>, String> {
    let mut vars: HashMap<usize, f64> = params.iter().enumerate().map(|(i, v)| (i + 1, *v)).collect();
    let mut acc: Vec<Polyline> = Vec::new();
    for stmt in body.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let unreadable = || format!("Makro-Zeile „{stmt}“ nicht lesbar.");
        // Kommentare beginnen mit dem Primitiv 0
        if stmt.starts_with('0') {
            continue;
        }
        if let Some(rest) = stmt.strip_prefix('$') {
            let (n, expr) = rest.split_once('=').ok_or_else(unreadable)?;
            vars.insert(n.trim().parse().map_err(|_| unreadable())?, eval(expr, &vars)?);
            continue;
        }
        let mut parts = stmt.split(',');
        let code: u32 = parts.next().unwrap_or("").trim().parse().map_err(|_| unreadable())?;
        let a: Vec<f64> = parts.map(|p| eval(p, &vars)).collect::<Result<_, _>>()?;
        let arg = |i: usize| a.get(i).copied().unwrap_or(0.0);
        let (mut prim, rotation, exposure_on) = match code {
            1 => (vec![closed(circle(Point::new(arg(2), arg(3)), arg(1)))], arg(4), arg(0) != 0.0),
            2 | 20 => {
                let (s, e, w) = (Point::new(arg(2), arg(3)), Point::new(arg(4), arg(5)), arg(1));
                let d = e - s;
                let len = d.length();
                let shape = if len < 1e-12 {
                    Vec::new()
                } else {
                    let n = Point::new(-d.y, d.x) * (w / 2.0 / len);
                    vec![closed(vec![s + n, e + n, e - n, s - n])]
                };
                (shape, arg(6), arg(0) != 0.0)
            }
            21 => (vec![closed(rect(Point::new(arg(3), arg(4)), arg(1), arg(2)))], arg(5), arg(0) != 0.0),
            22 => {
                let c = Point::new(arg(3) + arg(1) / 2.0, arg(4) + arg(2) / 2.0);
                (vec![closed(rect(c, arg(1), arg(2)))], arg(5), arg(0) != 0.0)
            }
            4 => {
                let n = arg(1).max(0.0) as usize;
                let pts: Vec<Point> = (0..=n).map(|i| Point::new(arg(2 + 2 * i), arg(3 + 2 * i))).collect();
                (vec![closed(pts)], arg(4 + 2 * n), arg(0) != 0.0)
            }
            5 => {
                let n = (arg(1) as usize).clamp(3, 12);
                let c = Point::new(arg(2), arg(3));
                (vec![closed(regular(arg(4), n, 0.0).into_iter().map(|p| p + c).collect())], arg(5), arg(0) != 0.0)
            }
            7 => {
                // Wärmefalle: Ring, durch ein Kreuz aus zwei Spalten in vier Stücke geteilt
                let (c, outer, inner, gap) = (Point::new(arg(0), arg(1)), arg(2), arg(3), arg(4));
                let ring = [closed(circle(c, outer)), closed(circle(c, inner))];
                let cross = [closed(rect(c, outer * 1.1, gap)), closed(rect(c, gap, outer * 1.1))];
                (ops::boolean(&ring, &ops::weld(&[cross.to_vec()]), BoolOp::Difference), arg(5), true)
            }
            6 => {
                warnings.push("Moiré-Blenden werden nicht unterstützt und ausgelassen.".into());
                continue;
            }
            _ => return Err(format!("Unbekanntes Makro-Primitiv {code}.")),
        };
        rotate_all(&mut prim, rotation);
        acc = ops::boolean(&acc, &prim, if exposure_on { BoolOp::Union } else { BoolOp::Difference });
    }
    Ok(acc)
}

// ------------------------------------------------------------------ Einlesen

#[derive(Clone, Copy, PartialEq)]
enum Interp {
    Linear,
    Cw,
    Ccw,
}

#[derive(Clone, Copy)]
struct Format {
    int_digits: u32,
    dec_digits: u32,
    /// Führende Nullen weggelassen (üblich); sonst die nachfolgenden.
    omit_leading: bool,
}

/// Zahl im Koordinatenformat (ohne Einheit); mit Dezimalpunkt wird sie direkt gelesen.
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
    let digits = if fmt.omit_leading { digits.to_string() } else { format!("{digits:0<total$}") };
    let v = digits.parse::<f64>().map_err(|_| bad())? / 10f64.powi(fmt.dec_digits as i32);
    Ok(if neg { -v } else { v })
}

struct State {
    fmt: Format,
    incremental: bool,
    /// mm je Dateieinheit.
    scale: f64,
    apertures: HashMap<u32, Aperture>,
    macros: HashMap<String, Vec<String>>,
    current: Option<u32>,
    pos: Point,
    interp: Interp,
    multi_quadrant: bool,
    region: Option<Vec<Vec<Point>>>,
    last_op: Option<u32>,
    clear: bool,
    profile: bool,
    /// Zusammenhängende Bahn mit der aktuellen Blende (mm, Y nach oben).
    track: Vec<Point>,
    /// Gesammelte Flächen der aktuellen Polarität, jede für sich nach Gerade-Ungerade.
    pending: Vec<Vec<Polyline>>,
    /// Ergebnis der bisherigen Polaritätswechsel.
    acc: Vec<Polyline>,
    /// Mittellinien (Profil).
    lines: Vec<Polyline>,
    warnings: Vec<String>,
}

impl State {
    fn new(profile: bool) -> Self {
        Self {
            fmt: Format { int_digits: 3, dec_digits: 6, omit_leading: true },
            incremental: false,
            scale: 1.0,
            apertures: HashMap::new(),
            macros: HashMap::new(),
            current: None,
            pos: Point::ZERO,
            interp: Interp::Linear,
            multi_quadrant: true,
            region: None,
            last_op: None,
            clear: false,
            profile,
            track: Vec::new(),
            pending: Vec::new(),
            acc: Vec::new(),
            lines: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn add_object(&mut self, shape: Vec<Polyline>) {
        if !shape.is_empty() {
            self.pending.push(shape);
        }
    }

    fn flush_track(&mut self) {
        let mut track = std::mem::take(&mut self.track);
        if track.len() < 2 {
            return;
        }
        if self.profile {
            let is_closed = track.len() > 3 && track[0].distance(track[track.len() - 1]) < 1e-6;
            if is_closed {
                track.pop();
            }
            self.lines.push(Polyline::new(track, is_closed));
            return;
        }
        let Some(Aperture::Circle { d, .. }) = self.current.and_then(|c| self.apertures.get(&c)) else { return };
        let d = *d;
        if d <= 0.0 {
            return;
        }
        if track.windows(2).all(|w| w[0].distance(w[1]) < 1e-9) {
            self.add_object(vec![closed(circle(track[0], d))]);
        } else {
            self.add_object(ops::offset(&[Polyline::new(track, false)], d / 2.0, Corner::Round));
        }
    }

    /// Flächen der aktuellen Polarität mit dem bisherigen Ergebnis verrechnen.
    fn flush_polarity(&mut self) {
        self.flush_track();
        let pending = std::mem::take(&mut self.pending);
        if pending.is_empty() {
            return;
        }
        if self.clear {
            self.acc = ops::boolean(&self.acc, &ops::weld(&pending), BoolOp::Difference);
        } else {
            let mut all = Vec::with_capacity(pending.len() + 1);
            if !self.acc.is_empty() {
                all.push(std::mem::take(&mut self.acc));
            }
            all.extend(pending);
            self.acc = ops::weld(&all);
        }
    }

    fn define_aperture(&mut self, def: &str) -> Result<(), String> {
        // ADD10C,0.5X0.2 · ADD11RoundRect,0.1X…
        let bad = || format!("Blendendefinition „{def}“ nicht lesbar.");
        let rest = def.strip_prefix("ADD").ok_or_else(bad)?;
        let split = rest.find(|c: char| !c.is_ascii_digit()).ok_or_else(bad)?;
        let num: u32 = rest[..split].parse().map_err(|_| bad())?;
        let (name, params) = rest[split..].split_once(',').unwrap_or((&rest[split..], ""));
        let p: Vec<f64> = if params.is_empty() {
            Vec::new()
        } else {
            params.split(['X', 'x']).map(|v| v.trim().parse::<f64>().map_err(|_| bad())).collect::<Result<_, _>>()?
        };
        let mm = |i: usize| p.get(i).map(|v| v * self.scale);
        let ap = match name {
            "C" => {
                let d = mm(0).ok_or_else(bad)?;
                let shape = if d > 0.0 { with_hole(vec![closed(circle(Point::ZERO, d))], mm(1)) } else { Vec::new() };
                Aperture::Circle { d, shape }
            }
            "R" => Aperture::Shape(with_hole(
                vec![closed(rect(Point::ZERO, mm(0).ok_or_else(bad)?, mm(1).ok_or_else(bad)?))],
                mm(2),
            )),
            "O" => Aperture::Shape(with_hole(obround(mm(0).ok_or_else(bad)?, mm(1).ok_or_else(bad)?), mm(2))),
            "P" => {
                let n = (p.get(1).copied().unwrap_or(3.0) as usize).clamp(3, 12);
                let rot = p.get(2).copied().unwrap_or(0.0);
                Aperture::Shape(with_hole(vec![closed(regular(mm(0).ok_or_else(bad)?, n, rot))], mm(3)))
            }
            m => {
                let body = self.macros.get(m).ok_or_else(|| format!("Blendenmakro „{m}“ ist nicht definiert."))?;
                let mut shape = macro_shape(body, &p, &mut self.warnings)?;
                scale_all(&mut shape, self.scale);
                Aperture::Shape(shape)
            }
        };
        self.apertures.insert(num, ap);
        Ok(())
    }

    fn extended(&mut self, stmts: &[String]) -> Result<(), String> {
        let Some(first) = stmts.first() else { return Ok(()) };
        if let Some(name) = first.strip_prefix("AM") {
            self.macros.insert(name.to_string(), stmts[1..].to_vec());
            return Ok(());
        }
        for s in stmts {
            if let Some(f) = s.strip_prefix("FS") {
                // FSLAX46Y46: Nullen (L/T), Notation (A/I), Stellen vor/nach dem Komma
                let b = f.as_bytes();
                self.fmt.omit_leading = b.first() != Some(&b'T');
                self.incremental = b.get(1) == Some(&b'I');
                if let Some(i) = f.find('X')
                    && let (Some(a), Some(d)) = (b.get(i + 1), b.get(i + 2))
                    && a.is_ascii_digit()
                    && d.is_ascii_digit()
                {
                    self.fmt.int_digits = (a - b'0') as u32;
                    self.fmt.dec_digits = (d - b'0') as u32;
                }
            } else if s == "MOMM" {
                self.scale = 1.0;
            } else if s == "MOIN" {
                self.scale = 25.4;
            } else if s.starts_with("ADD") {
                self.define_aperture(s)?;
            } else if s == "LPD" || s == "LPC" {
                let clear = s == "LPC";
                if clear != self.clear {
                    self.flush_polarity();
                    self.clear = clear;
                }
            } else if let Some(sr) = s.strip_prefix("SR") {
                let count = |k: char| {
                    sr.split_once(k).map_or(1, |(_, r)| {
                        r.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(1)
                    })
                };
                if count('X') > 1 || count('Y') > 1 {
                    self.warnings.push("Schritt-und-Wiederhol-Blöcke (SR) werden nur einmal übernommen.".into());
                }
            } else if s.starts_with("TF.FileFunction,Profile") {
                self.profile = true;
            }
        }
        Ok(())
    }

    fn aperture(&self) -> Option<&Aperture> {
        self.current.and_then(|c| self.apertures.get(&c))
    }

    /// Ein Datenblock (bis `*`). `false` am Dateiende (M02).
    fn word(&mut self, w: &str) -> Result<bool, String> {
        if w.starts_with("G04") || w == "G4" || w.starts_with("G4 ") {
            return Ok(true);
        }
        let mut coords: HashMap<char, &str> = HashMap::new();
        let mut d_code = None;
        let b = w.as_bytes();
        let mut i = 0;
        while i < b.len() {
            let letter = b[i] as char;
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_digit() || matches!(b[j], b'-' | b'+' | b'.')) {
                j += 1;
            }
            let val = &w[i + 1..j];
            match letter {
                'G' => match val.parse::<u32>().unwrap_or(u32::MAX) {
                    1 => self.interp = Interp::Linear,
                    2 => self.interp = Interp::Cw,
                    3 => self.interp = Interp::Ccw,
                    74 => self.multi_quadrant = false,
                    75 => self.multi_quadrant = true,
                    36 => {
                        self.flush_track();
                        self.region = Some(Vec::new());
                    }
                    37 => self.end_region(),
                    70 => self.scale = 25.4,
                    71 => self.scale = 1.0,
                    90 => self.incremental = false,
                    91 => self.incremental = true,
                    _ => {}
                },
                'D' => d_code = val.parse::<u32>().ok(),
                'M' if matches!(val.parse::<u32>(), Ok(0..=2)) => return Ok(false),
                'X' | 'Y' | 'I' | 'J' => {
                    coords.insert(letter, val);
                }
                _ => {}
            }
            i = j;
        }
        match d_code {
            Some(n) if n >= 10 => {
                self.flush_track();
                if !self.apertures.contains_key(&n) {
                    return Err(format!("Blende D{n} wird benutzt, ist aber nicht definiert."));
                }
                self.current = Some(n);
                return Ok(true);
            }
            Some(op @ 1..=3) => self.last_op = Some(op),
            _ if !coords.is_empty() => {}
            _ => return Ok(true),
        }
        let Some(op) = d_code.or(self.last_op) else { return Ok(true) };
        let read = |k: char, prev: f64| -> Result<f64, String> {
            match coords.get(&k) {
                Some(v) => {
                    let v = number(self.fmt, v)? * self.scale;
                    Ok(if self.incremental { prev + v } else { v })
                }
                None => Ok(prev),
            }
        };
        let target = Point::new(read('X', self.pos.x)?, read('Y', self.pos.y)?);
        // Mittelpunktsabstände sind immer relativ zum Anfangspunkt
        let offset = |k: char| coords.get(&k).map_or(Ok(0.0), |v| number(self.fmt, v).map(|v| v * self.scale));
        match op {
            1 => {
                let pts = match self.interp {
                    Interp::Linear => vec![target],
                    dir => self.arc(target, offset('I')?, offset('J')?, dir == Interp::Ccw),
                };
                self.interpolate(pts);
            }
            2 => match &mut self.region {
                Some(region) => {
                    if region.last().is_some_and(|c| c.len() == 1) {
                        region.pop();
                    }
                    region.push(vec![target]);
                }
                None => self.flush_track(),
            },
            3 => {
                self.flush_track();
                if !self.profile
                    && let Some(ap) = self.aperture()
                {
                    let shape = ap
                        .shape()
                        .iter()
                        .map(|p| Polyline::new(p.points.iter().map(|q| *q + target).collect(), true))
                        .collect();
                    self.add_object(shape);
                }
            }
            _ => {}
        }
        self.pos = target;
        Ok(true)
    }

    /// D01: Region erweitern, Bahn verlängern oder (eckige Blende) Teilstücke als Hülle anlegen.
    fn interpolate(&mut self, pts: Vec<Point>) {
        let from = self.pos;
        if let Some(region) = &mut self.region {
            if region.is_empty() {
                region.push(vec![from]);
            }
            region.last_mut().expect("Kontur").extend(pts);
            return;
        }
        if self.profile || matches!(self.aperture(), Some(Aperture::Circle { .. })) {
            if self.track.is_empty() {
                self.track.push(from);
            }
            self.track.extend(pts);
            return;
        }
        let Some(outline) = self.aperture().and_then(|a| a.shape().first()).map(|p| p.points.clone()) else { return };
        let mut a = from;
        for b in pts {
            let hull: Vec<Point> = outline.iter().map(|q| *q + a).chain(outline.iter().map(|q| *q + b)).collect();
            self.add_object(vec![closed(ops::convex_hull(&hull))]);
            a = b;
        }
    }

    fn arc(&self, target: Point, i: f64, j: f64, ccw: bool) -> Vec<Point> {
        let from = self.pos;
        if self.multi_quadrant {
            let full = from.distance(target) < 1e-9;
            return arc_points(from + Point::new(i, j), from, target, ccw, full);
        }
        // Einzelquadrant: I/J ohne Vorzeichen – den Mittelpunkt wählen, der zu beiden Enden passt und höchstens
        // 90° überstreicht
        let mut best: Option<(f64, Point)> = None;
        for (si, sj) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
            let c = from + Point::new(si * i.abs(), sj * j.abs());
            let a0 = (from.y - c.y).atan2(from.x - c.x);
            let a1 = (target.y - c.y).atan2(target.x - c.x);
            let mut sweep = a1 - a0;
            if ccw && sweep < 0.0 {
                sweep += TAU;
            } else if !ccw && sweep > 0.0 {
                sweep -= TAU;
            }
            if sweep.abs() > PI / 2.0 + 1e-6 {
                continue;
            }
            let err = (from.distance(c) - target.distance(c)).abs();
            if best.is_none_or(|(e, _)| err < e) {
                best = Some((err, c));
            }
        }
        match best {
            Some((_, c)) => arc_points(c, from, target, ccw, false),
            None => vec![target],
        }
    }

    fn end_region(&mut self) {
        let Some(region) = self.region.take() else { return };
        for c in region.into_iter().filter(|c| c.len() >= 3) {
            if self.profile {
                self.lines.push(Polyline::new(c, true));
            } else {
                self.add_object(vec![closed(c)]);
            }
        }
    }
}

/// Zerlegt die Datei in erweiterte Befehle (`%…%`, mehrere Anweisungen) und Datenblöcke (bis `*`).
enum Block {
    Extended(Vec<String>),
    Word(String),
}

fn blocks(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut rest = text;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if let Some(r) = rest.strip_prefix('%') {
            let (inner, after) = r.split_once('%').unwrap_or((r, ""));
            let stmts = inner
                .split('*')
                .map(|s| s.chars().filter(|c| !c.is_whitespace() || *c == ' ').collect::<String>().trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            out.push(Block::Extended(stmts));
            rest = after;
        } else {
            let (word, after) = rest.split_once('*').unwrap_or((rest, ""));
            let word: String = word.chars().filter(|c| *c != '\r' && *c != '\n').collect();
            out.push(Block::Word(word.trim().to_string()));
            rest = after;
        }
    }
    out
}

/// Erkennt Gerber am Inhalt (Format- oder Blendenangaben).
pub fn looks_like_gerber(data: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&data[..data.len().min(4096)]);
    head.contains("%FS") || head.contains("%MO") || head.contains("%ADD")
}

/// Liest eine Gerber-Datei. `profile`: als Platinenumriss (Mittellinien) statt als Flächen übernehmen – wird auch
/// über das Attribut `TF.FileFunction,Profile` erkannt.
pub fn import_gerber(data: &[u8], name: &str, profile: bool) -> Result<Imported, String> {
    let text = std::str::from_utf8(data).map_err(|_| "Die Gerber-Datei ist keine Textdatei.".to_string())?;
    if !looks_like_gerber(data) {
        return Err("Keine Gerber-Datei (Formatangabe %FS fehlt).".into());
    }
    let mut st = State::new(profile);
    for block in blocks(text) {
        match block {
            Block::Extended(stmts) => st.extended(&stmts)?,
            Block::Word(w) if w.is_empty() => {}
            Block::Word(w) => {
                if !st.word(&w)? {
                    break;
                }
            }
        }
    }
    st.end_region();
    st.flush_polarity();
    let polys = if st.profile {
        st.flush_track();
        std::mem::take(&mut st.lines)
    } else {
        std::mem::take(&mut st.acc)
    };
    if polys.is_empty() {
        return Err("Die Gerber-Datei enthält keine Formen.".into());
    }
    let mut path = VectorPath::default();
    for p in &polys {
        // Y spiegeln: Gerber zählt nach oben
        let pts: Vec<Point> = p.points.iter().map(|q| Point::new(q.x, -q.y)).collect();
        path.append(VectorPath::polygon(&pts, p.closed));
    }
    let mut out = Imported { warnings: st.warnings, ..Default::default() };
    out.items.push(ImportedItem::Path { name: name.to_string(), layer: None, path });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::Affine;

    fn area(path: &VectorPath) -> f64 {
        // Gerade-Ungerade-Fläche: Summe der vorzeichenbehafteten Flächen nach der Normalisierung
        let polys = path.flatten(&Affine::IDENTITY, 0.001);
        ops::boolean(&polys, &[], BoolOp::Union).iter().map(|p| p.signed_area()).sum::<f64>().abs()
    }

    fn import(src: &str) -> VectorPath {
        let imp = import_gerber(src.as_bytes(), "t.gbr", false).unwrap();
        let ImportedItem::Path { path, .. } = imp.items.into_iter().next().unwrap() else { panic!() };
        path
    }

    #[test]
    fn pads_tracks_and_union() {
        // Zwei runde Pads (Ø 2) und eine 1 mm breite Leiterbahn dazwischen, 10 mm Abstand
        let src = "%FSLAX46Y46*%\n%MOMM*%\n%ADD10C,2*%\n%ADD11C,1*%\nD10*\nX0Y0D03*\nX10000000Y0D03*\n\
                   D11*\nX0Y0D02*\nX10000000Y0D01*\nM02*\n";
        let path = import(src);
        let b = path.bounds(&Affine::IDENTITY);
        assert!((b.min.x + 1.0).abs() < 1e-3 && (b.max.x - 11.0).abs() < 1e-3, "{b:?}");
        assert!((b.height() - 2.0).abs() < 1e-3);
        // Fläche: zwei Kreise + Rechteck 10×1 ohne die überlappenden Teile; vereinigt = eine Kontur
        assert_eq!(path.subpaths.len(), 1, "Pads und Bahn verschmolzen");
        let expect = 2.0 * PI + 10.0 - 2.0 * (0.5 * (2.0 * (0.5f64).asin() + 2.0 * 0.5 * (0.75f64).sqrt()));
        // Kreise als Vielecke mit höchstens 5 µm Sehnenabstand: etwas kleiner als exakt
        assert!((area(&path) - expect).abs() < 0.05, "{} vs {expect}", area(&path));
    }

    #[test]
    fn inch_format_trailing_zeros_and_y_flip() {
        // Zoll, Format 2.4, nachfolgende Nullen weggelassen: „X01“ wird zu 01|0000 = 1 Zoll
        let src = "%FSTAX24Y24*%%MOIN*%%ADD10R,0.1X0.2*%D10*X01Y01D03*M02*";
        let path = import(src);
        let b = path.bounds(&Affine::IDENTITY);
        assert!((b.center().x - 25.4).abs() < 1e-6 && (b.center().y + 25.4).abs() < 1e-6, "{b:?}");
        assert!((b.width() - 2.54).abs() < 1e-6 && (b.height() - 5.08).abs() < 1e-6);
    }

    #[test]
    fn regions_arcs_and_clear_polarity() {
        // Quadrat 10×10 als Region, darin kreisrundes Loch Ø 4 per LPC
        let src = "%FSLAX33Y33*%%MOMM*%%ADD10C,4*%\
                   G36*X0Y0D02*G01X10000Y0D01*Y10000D01*X0D01*Y0D01*G37*\
                   %LPC*%D10*X5000Y5000D03*%LPD*%M02*";
        let a = area(&import(src));
        assert!((a - (100.0 - PI * 4.0)).abs() < 0.05, "{a}");
        // Halbkreis-Region mit G75/G03: Fläche π·5²/2
        let src = "%FSLAX33Y33*%%MOMM*%G75*G36*X-5000Y0D02*G01X5000Y0D01*G03X-5000Y0I-5000J0D01*G37*M02*";
        let a = area(&import(src));
        assert!((a - PI * 12.5).abs() < 0.05, "{a}");
        // Vollkreis aus einem Bogen mit Start = Ende
        let src = "%FSLAX33Y33*%%MOMM*%G75*G36*X3000Y0D02*G02X3000Y0I-3000J0D01*G37*M02*";
        let a = area(&import(src));
        assert!((a - PI * 9.0).abs() < 0.1, "{a}");
    }

    #[test]
    fn aperture_macro_with_variables_and_exposure_off() {
        // Rechteck 4×2 (Primitiv 21) mit Loch Ø 1 (Primitiv 1, Belichtung aus); Breite über Variable verdoppelt
        let src = "%FSLAX33Y33*%%MOMM*%\
                   %AMBOX*0 Rechteck mit Loch*$3=$1x2*21,1,$3,$2,0,0,0*1,0,1,0,0*%\
                   %ADD20BOX,2X2*%D20*X0Y0D03*M02*";
        let path = import(src);
        let b = path.bounds(&Affine::IDENTITY);
        assert!((b.width() - 4.0).abs() < 1e-6 && (b.height() - 2.0).abs() < 1e-6, "{b:?}");
        assert!((area(&path) - (8.0 - PI * 0.25)).abs() < 0.01, "{}", area(&path));
        // Ausdrücke
        let vars = HashMap::from([(1, 3.0), (2, 0.5)]);
        assert_eq!(eval("$1x2+$2", &vars).unwrap(), 6.5);
        assert_eq!(eval("($1+1)/-2", &vars).unwrap(), -2.0);
        assert!(eval("2x", &vars).is_err());
    }

    /// Abgerundetes Rechteck, wie KiCad es für SMD-Pads als Makro schreibt (Kommentare, Ausdrücke wie `$1+$1`).
    #[test]
    fn kicad_roundrect_macro() {
        let src = "%TF.GenerationSoftware,KiCad,Pcbnew,8.0*%
%FSLAX46Y46*%
G04 Gerber Fmt 4.6*
%MOMM*%
%LPD*%
                   %AMRoundRect*
0 Rectangle with rounded corners*
0 $1 Rounding radius*
                   0 $2 $3 $4 $5 $6 $7 $8 $9 X,Y pos of 4 corners*
4,1,4,$2,$3,$4,$5,$6,$7,$8,$9,$2,$3,0*
                   1,1,$1+$1,$2,$3*
1,1,$1+$1,$4,$5*
1,1,$1+$1,$6,$7*
1,1,$1+$1,$8,$9*
                   20,1,$1+$1,$2,$3,$4,$5,0*
20,1,$1+$1,$4,$5,$6,$7,0*
20,1,$1+$1,$6,$7,$8,$9,0*
                   20,1,$1+$1,$8,$9,$2,$3,0*%
                   %TA.AperFunction,SMDPad,CuDef*%
                   %ADD11RoundRect,0.250000X-0.450000X-0.600000X0.450000X-0.600000X0.450000X0.600000X-0.450000X0.600000X0*%
                   %TD*%
G04 #@! TO.N,GND*
D11*
X5000000Y-3000000D03*
M02*
";
        let path = import(src);
        let b = path.bounds(&Affine::IDENTITY);
        assert!((b.width() - 1.4).abs() < 1e-3 && (b.height() - 1.7).abs() < 1e-3, "{b:?}");
        assert!(b.center().distance(Point::new(5.0, 3.0)) < 1e-6);
        let expect = 0.9 * 1.2 + 2.0 * (0.9 + 1.2) * 0.25 + PI * 0.25 * 0.25;
        assert!((area(&path) - expect).abs() < 0.01, "{} vs {expect}", area(&path));
        assert_eq!(path.subpaths.len(), 1);
    }

    #[test]
    fn rect_aperture_stroke_and_profile() {
        // Rechteckblende 1×1 von (0,0) nach (10,0): Fläche 11×1
        let src = "%FSLAX33Y33*%%MOMM*%%ADD10R,1X1*%D10*X0Y0D02*X10000Y0D01*M02*";
        assert!((area(&import(src)) - 11.0).abs() < 1e-6);
        // Umriss: geschlossener Linienzug als Mittellinie
        let src = "%FSLAX33Y33*%%MOMM*%%TF.FileFunction,Profile,NP*%%ADD10C,0.1*%D10*\
                   X0Y0D02*X20000Y0D01*Y10000D01*X0D01*Y0D01*M02*";
        let imp = import_gerber(src.as_bytes(), "rand.gbr", false).unwrap();
        let ImportedItem::Path { path, .. } = &imp.items[0] else { panic!() };
        assert_eq!(path.subpaths.len(), 1);
        assert!(path.subpaths[0].closed);
        let b = path.bounds(&Affine::IDENTITY);
        assert_eq!((b.width(), b.height()), (20.0, 10.0));
    }

    #[test]
    fn errors() {
        assert!(import_gerber(b"hallo", "x", false).is_err());
        assert!(import_gerber(b"%FSLAX33Y33*%%MOMM*%D11*X0Y0D03*M02*", "x", false).is_err(), "Blende fehlt");
        assert!(import_gerber(b"%FSLAX33Y33*%%MOMM*%M02*", "x", false).is_err(), "leer");
    }
}
