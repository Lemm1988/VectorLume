//! G-Code-Erzeugung für GRBL-Lasersteuerungen (GRBL 1.1 im Laser-Modus `$32=1`).

use crate::toolpath::{BlockKind, Op, Toolpath};
use vl_core::{Firmware, Point};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoordMode {
    /// G90: absolute Koordinaten.
    Absolute,
    /// G91: relative Bewegungen ab der aktuellen Kopfposition (Start = 0/0).
    Relative,
}

#[derive(Clone, Debug)]
pub struct GcodeOptions {
    /// Dialekt: Laser-Einschaltbefehl und Format von S.
    pub firmware: Firmware,
    pub s_max: f64,
    pub dynamic_power: bool,
    pub air_on: String,
    pub air_off: String,
    pub air_supported: bool,
    /// Z-Bewegungen der Blöcke ausgeben.
    pub z_axis: bool,
    /// Die zweite Koordinate als A statt Y ausgeben (Drehachse an eigener Achse).
    pub rotary_a: bool,
    pub coord_mode: CoordMode,
    /// Nach dem Job im Eilgang hierhin fahren (Ausgabe-Koordinaten).
    pub return_to: Option<Point>,
    pub comments: Vec<String>,
    pub layer_names: Vec<String>,
}

/// Koordinaten werden intern in Mikrometern gerundet. Dadurch sind relative Bewegungen exakt
/// (kein Aufsummieren von Rundungsfehlern).
fn to_um(v: f64) -> i64 {
    (v * 1000.0).round() as i64
}

fn fmt_um(v: i64) -> String {
    fmt_fixed(v, 3)
}

/// Ganzzahl mit `decimals` gedachten Nachkommastellen, ohne überflüssige Nullen.
fn fmt_fixed(v: i64, decimals: u32) -> String {
    let mut s = String::new();
    push_fixed(&mut s, v, decimals);
    s
}

/// Wie [`fmt_fixed`], hängt aber an `out` an (ohne Zwischen-Strings – bei Millionen Zeilen spürbar).
fn push_fixed(out: &mut String, v: i64, decimals: u32) {
    use std::fmt::Write;
    if v < 0 {
        out.push('-');
    }
    let a = v.unsigned_abs();
    let unit = 10u64.pow(decimals);
    let (int, mut frac) = (a / unit, a % unit);
    let _ = write!(out, "{int}");
    if frac == 0 {
        return;
    }
    let mut width = decimals as usize;
    while frac % 10 == 0 {
        frac /= 10;
        width -= 1;
    }
    let _ = write!(out, ".{frac:0width$}");
}

fn fmt_num(v: f64) -> String {
    fmt_um(to_um(v))
}

struct Emitter<'a> {
    out: Vec<String>,
    opts: &'a GcodeOptions,
    /// Zuletzt ausgegebene Position; `None` = unbekannt (Absolutmodus vor der ersten Bewegung).
    pos: Option<(i64, i64)>,
    motion: Option<u8>,
    feed: Option<i64>,
    s: Option<i64>,
    /// Z-Höhe in µm relativ zur Fokushöhe beim Jobstart.
    z: i64,
}

impl Emitter<'_> {
    /// Hängt die Achsenwörter für `p` an `line` an; `false`, wenn sich die Position nicht ändert.
    fn push_axes(&mut self, line: &mut String, p: Point) -> bool {
        let target = (to_um(p.x), to_um(p.y));
        if self.pos == Some(target) {
            return false;
        }
        let (dx, dy) = match self.pos {
            Some(q) => (target.0 - q.0, target.1 - q.1),
            None => (1, 1),
        };
        let (vx, vy) = match self.opts.coord_mode {
            CoordMode::Absolute => (target.0, target.1),
            CoordMode::Relative => (dx, dy),
        };
        if dx != 0 {
            line.push('X');
            push_fixed(line, vx, 3);
        }
        if dy != 0 {
            if dx != 0 {
                line.push(' ');
            }
            line.push(if self.opts.rotary_a { 'A' } else { 'Y' });
            push_fixed(line, vy, 3);
        }
        self.pos = Some(target);
        true
    }

    fn rapid(&mut self, p: Point) {
        let mut line = String::with_capacity(24);
        line.push_str("G0 ");
        if self.push_axes(&mut line, p) {
            self.motion = Some(0);
            self.out.push(line);
        }
    }

    /// Fährt im Eilgang (Laser aus) auf die Höhe `z`.
    fn z_move(&mut self, z: f64) {
        let target = to_um(z);
        if target == self.z {
            return;
        }
        let v = match self.opts.coord_mode {
            CoordMode::Absolute => target,
            CoordMode::Relative => target - self.z,
        };
        self.out.push(format!("G0 Z{}", fmt_um(v)));
        self.motion = Some(0);
        self.z = target;
    }

    fn feed_move(&mut self, p: Point, feed: f64, power: f32) {
        use std::fmt::Write;
        let mut line = String::with_capacity(32);
        if self.motion != Some(1) {
            line.push_str("G1 ");
        }
        if !self.push_axes(&mut line, p) {
            return;
        }
        self.motion = Some(1);
        let f = feed.round() as i64;
        if self.feed != Some(f) {
            let _ = write!(line, " F{f}");
            self.feed = Some(f);
        }
        let s = self.s_key(power as f64);
        if self.s != Some(s) {
            line.push_str(" S");
            if self.opts.firmware == Firmware::Smoothieware {
                push_fixed(&mut line, s, 4);
            } else {
                let _ = write!(line, "{s}");
            }
            self.s = Some(s);
        }
        self.out.push(line);
    }

    /// S als Vergleichswert: Smoothieware erwartet einen Anteil 0…1 (hier in Zehntausendstel), die anderen ganze
    /// Zahlen bis S-Max.
    fn s_key(&self, power: f64) -> i64 {
        let v = power * self.opts.s_max;
        if self.opts.firmware == Firmware::Smoothieware { (v * 10_000.0).round() as i64 } else { v.round() as i64 }
    }
}

fn ascii_comment(text: &str) -> String {
    // GRBL-Steuerungen vertragen nicht immer Umlaute, deshalb nur ASCII in Kommentaren.
    let cleaned: String = text
        .chars()
        .map(|c| match c {
            'ä' => "ae".into(),
            'ö' => "oe".into(),
            'ü' => "ue".into(),
            'Ä' => "Ae".into(),
            'Ö' => "Oe".into(),
            'Ü' => "Ue".into(),
            'ß' => "ss".into(),
            c if c.is_ascii() && !c.is_ascii_control() => c.to_string(),
            _ => "?".into(),
        })
        .collect();
    format!("; {cleaned}")
}

/// Erzeugt G-Code. `map` rechnet Dokument- in Ausgabekoordinaten um.
pub fn emit(tp: &Toolpath, map: &dyn Fn(Point) -> Point, opts: &GcodeOptions) -> Vec<String> {
    let start = match opts.coord_mode {
        CoordMode::Absolute => None,
        CoordMode::Relative => Some((0, 0)),
    };
    let ops: usize = tp.blocks.iter().map(|b| b.ops.len()).sum();
    let out = Vec::with_capacity(ops + 2 * tp.blocks.len() + opts.comments.len() + 16);
    let mut e = Emitter { out, opts, pos: start, motion: None, feed: None, s: None, z: 0 };
    for c in &opts.comments {
        e.out.push(ascii_comment(c));
    }
    e.out.push("G21".into());
    e.out.push(match opts.coord_mode {
        CoordMode::Absolute => "G90".into(),
        CoordMode::Relative => "G91".into(),
    });
    let laser_on = |constant: bool| {
        let m = if opts.dynamic_power && !constant { "M4" } else { "M3" };
        match opts.firmware {
            Firmware::Grbl => Some(format!("{m} S0")),
            // Inline-Modus: Leistung gilt nur für G1-Bewegungen, G0 bleibt dunkel.
            Firmware::Marlin => Some(format!("{m} I S0")),
            // Das Lasermodul feuert nur bei G1 mit S; kein Einschaltbefehl.
            Firmware::Smoothieware => None,
        }
    };
    let mut mode = laser_on(false);
    e.out.extend(mode.clone());
    e.s = mode.is_some().then_some(0);

    let mut air = false;
    for block in &tp.blocks {
        if block.ops.is_empty() {
            continue;
        }
        let kind = match block.kind {
            BlockKind::Line => "Linie",
            BlockKind::Fill => "Fuellen",
            BlockKind::Raster => "Bild",
        };
        let name = opts.layer_names.get(block.layer).cloned().unwrap_or_else(|| format!("L{:02}", block.layer + 1));
        e.out.push(ascii_comment(&format!("Ebene {name} - {kind} - Durchgang {}", block.pass + 1)));
        let want_air = block.air && opts.air_supported;
        if want_air != air {
            e.out.push(if want_air { opts.air_on.clone() } else { opts.air_off.clone() });
            air = want_air;
        }
        if opts.z_axis {
            e.z_move(block.z.unwrap_or(0.0));
        }
        let want = laser_on(block.constant_power);
        if want != mode {
            e.out.extend(want.clone());
            e.s = Some(0);
            mode = want;
        }
        for op in &block.ops {
            match *op {
                Op::Rapid(p) => e.rapid(map(p)),
                Op::Move { to, power } => e.feed_move(map(to), block.feed, power),
            }
        }
    }
    if opts.firmware != Firmware::Smoothieware {
        e.out.push("M5".into());
    }
    if opts.z_axis {
        e.z_move(0.0);
    }
    if air {
        e.out.push(opts.air_off.clone());
    }
    if let Some(p) = opts.return_to {
        e.rapid(p);
    }
    if opts.coord_mode == CoordMode::Relative {
        e.out.push("G90".into());
    }
    e.out.push(ascii_comment("Ende"));
    e.out
}

/// Zeilen einer fremden G-Code-Datei zum Senden: ohne Kommentare, Leerzeilen, Programmklammern `%` und BOM.
pub fn lines_from_file(text: &str) -> Vec<String> {
    let lines: Vec<String> = text.trim_start_matches('\u{feff}').lines().map(str::to_string).collect();
    strip_for_streaming(&lines).into_iter().filter(|l| l != "%").collect()
}

/// Bereitet Zeilen für das Senden an die Steuerung vor: Kommentare und Leerzeilen entfernen.
pub fn strip_for_streaming(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|l| {
            let code = match l.find(';') {
                Some(i) => &l[..i],
                None => l.as_str(),
            };
            let code = code.trim();
            (!code.is_empty()).then(|| code.to_string())
        })
        .collect()
}

pub fn fmt_mm(v: f64) -> String {
    fmt_num(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreign_file_lines() {
        let text = "\u{feff}%\r\n; Kopf\r\nG21 G90 ; mm\r\n\r\nM4 S0\r\nG1 X10 F600 S500\r\nM5\r\n%\r\n";
        assert_eq!(lines_from_file(text), vec!["G21 G90", "M4 S0", "G1 X10 F600 S500", "M5"]);
    }
    use crate::toolpath::Block;

    fn opts(mode: CoordMode) -> GcodeOptions {
        GcodeOptions {
            firmware: Firmware::Grbl,
            s_max: 1000.0,
            dynamic_power: true,
            air_on: "M8".into(),
            air_off: "M9".into(),
            air_supported: true,
            z_axis: false,
            rotary_a: false,
            coord_mode: mode,
            return_to: Some(Point::ZERO),
            comments: vec!["Test äöü".into()],
            layer_names: vec![],
        }
    }

    fn square() -> Toolpath {
        let pts = [Point::new(10.0, 10.0), Point::new(20.0, 10.0), Point::new(20.0, 20.0), Point::new(10.0, 20.0)];
        let mut ops = vec![Op::Rapid(pts[0])];
        for p in pts[1..].iter().chain(std::iter::once(&pts[0])) {
            ops.push(Op::Move { to: *p, power: 0.5 });
        }
        Toolpath {
            blocks: vec![Block {
                layer: 0,
                kind: BlockKind::Line,
                pass: 0,
                feed: 1200.0,
                air: true,
                constant_power: false,
                ops,
                raster: None,
                z: None,
            }],
            rasters: vec![],
        }
    }

    #[test]
    fn z_per_pass_returns_to_focus() {
        let mut tp = square();
        let mut second = tp.blocks[0].clone();
        tp.blocks[0].z = Some(1.0);
        second.pass = 1;
        second.z = Some(0.5);
        tp.blocks.push(second);
        for mode in [CoordMode::Absolute, CoordMode::Relative] {
            let mut o = opts(mode);
            o.z_axis = true;
            let code = strip_for_streaming(&emit(&tp, &|p| p, &o));
            let z: Vec<&String> = code.iter().filter(|l| l.starts_with("G0 Z")).collect();
            match mode {
                CoordMode::Absolute => assert_eq!(z, ["G0 Z1", "G0 Z0.5", "G0 Z0"]),
                CoordMode::Relative => assert_eq!(z, ["G0 Z1", "G0 Z-0.5", "G0 Z-0.5"]),
            }
        }
        // ohne Z-Achse keine Z-Wörter
        let code = emit(&tp, &|p| p, &opts(CoordMode::Absolute));
        assert!(!code.iter().any(|l| l.contains('Z')));
    }

    #[test]
    fn firmware_dialects() {
        let tp = square();
        let marlin = GcodeOptions { firmware: Firmware::Marlin, s_max: 255.0, ..opts(CoordMode::Absolute) };
        let code = strip_for_streaming(&emit(&tp, &|p| p, &marlin));
        assert!(code.contains(&"M4 I S0".to_string()), "{code:?}");
        assert!(code.iter().any(|l| l.ends_with("S128")), "{code:?}");
        assert!(code.contains(&"M5".to_string()));
        let smoothie = GcodeOptions { firmware: Firmware::Smoothieware, s_max: 1.0, ..opts(CoordMode::Absolute) };
        let code = strip_for_streaming(&emit(&tp, &|p| p, &smoothie));
        assert!(!code.iter().any(|l| l.starts_with("M3") || l.starts_with("M4") || l == "M5"), "{code:?}");
        assert!(code.iter().any(|l| l.starts_with("G1") && l.ends_with("S0.5")), "{code:?}");
    }

    #[test]
    fn constant_power_switches_to_m3_per_block() {
        let mut tp = square();
        let mut second = tp.blocks[0].clone();
        second.constant_power = true;
        tp.blocks.push(second);
        tp.blocks.push(tp.blocks[0].clone());
        let code = strip_for_streaming(&emit(&tp, &|p| p, &opts(CoordMode::Absolute)));
        let modes: Vec<&String> = code.iter().filter(|l| l.starts_with("M3") || l.starts_with("M4")).collect();
        assert_eq!(modes, ["M4 S0", "M3 S0", "M4 S0"]);
        // nach dem Umschalten steht S wieder in der nächsten Bewegung
        let i = code.iter().position(|l| l == "M3 S0").unwrap();
        assert!(
            code[i + 1..].iter().find(|l| l.contains('X') || l.contains('Y')).unwrap().ends_with("S500"),
            "{code:?}"
        );
        // Gerät arbeitet ohnehin mit M3: kein Umschalten
        let m3 = GcodeOptions { dynamic_power: false, ..opts(CoordMode::Absolute) };
        let code = strip_for_streaming(&emit(&tp, &|p| p, &m3));
        assert_eq!(code.iter().filter(|l| l.starts_with("M3") || l.starts_with("M4")).count(), 1);
    }

    #[test]
    fn number_format() {
        assert_eq!(fmt_num(1.0), "1");
        assert_eq!(fmt_num(-0.0004), "0");
        assert_eq!(fmt_num(-1.25), "-1.25");
        assert_eq!(fmt_num(12.3456), "12.346");
    }

    #[test]
    fn absolute_square() {
        let g = emit(&square(), &|p| p, &opts(CoordMode::Absolute));
        let code = strip_for_streaming(&g);
        assert_eq!(
            code,
            vec![
                "G21",
                "G90",
                "M4 S0",
                "M8",
                "G0 X10 Y10",
                "G1 X20 F1200 S500",
                "Y20",
                "X10",
                "Y10",
                "M5",
                "M9",
                "G0 X0 Y0",
            ]
        );
        assert!(g[0].is_ascii());
    }

    #[test]
    fn relative_moves_sum_to_absolute() {
        let g = emit(&square(), &|p| p, &opts(CoordMode::Relative));
        let code = strip_for_streaming(&g);
        assert_eq!(code[1], "G91");
        assert_eq!(code.last().unwrap(), "G90");
        // Alle relativen Bewegungen addiert ergeben die Rückkehr zum Start.
        let (mut x, mut y) = (0i64, 0i64);
        for line in &code {
            for word in line.split_whitespace() {
                if let Some(v) = word.strip_prefix('X') {
                    x += to_um(v.parse::<f64>().unwrap());
                }
                if let Some(v) = word.strip_prefix('Y') {
                    y += to_um(v.parse::<f64>().unwrap());
                }
            }
        }
        assert_eq!((x, y), (0, 0));
    }
}
