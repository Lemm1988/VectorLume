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
    let sign = if v < 0 { "-" } else { "" };
    let a = v.unsigned_abs();
    let unit = 10u64.pow(decimals);
    let (int, frac) = (a / unit, a % unit);
    if frac == 0 {
        return format!("{sign}{int}");
    }
    let frac = format!("{frac:0w$}", w = decimals as usize);
    format!("{sign}{int}.{}", frac.trim_end_matches('0'))
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
    fn axes(&mut self, p: Point) -> Option<String> {
        let target = (to_um(p.x), to_um(p.y));
        if self.pos == Some(target) {
            return None;
        }
        let mut words = String::new();
        let (dx, dy) = match self.pos {
            Some(q) => (target.0 - q.0, target.1 - q.1),
            None => (1, 1),
        };
        let (vx, vy) = match self.opts.coord_mode {
            CoordMode::Absolute => (target.0, target.1),
            CoordMode::Relative => (dx, dy),
        };
        if dx != 0 {
            words.push_str(&format!("X{}", fmt_um(vx)));
        }
        if dy != 0 {
            if !words.is_empty() {
                words.push(' ');
            }
            words.push(if self.opts.rotary_a { 'A' } else { 'Y' });
            words.push_str(&fmt_um(vy));
        }
        self.pos = Some(target);
        Some(words)
    }

    fn rapid(&mut self, p: Point) {
        if let Some(axes) = self.axes(p) {
            self.motion = Some(0);
            self.out.push(format!("G0 {axes}"));
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
        let Some(axes) = self.axes(p) else { return };
        let mut line = String::new();
        if self.motion != Some(1) {
            line.push_str("G1 ");
            self.motion = Some(1);
        }
        line.push_str(&axes);
        let f = feed.round() as i64;
        if self.feed != Some(f) {
            line.push_str(&format!(" F{f}"));
            self.feed = Some(f);
        }
        let (s, text) = self.s_value(power as f64);
        if self.s != Some(s) {
            line.push_str(&format!(" S{text}"));
            self.s = Some(s);
        }
        self.out.push(line);
    }

    /// S als Vergleichswert und Text. Smoothieware erwartet einen Anteil 0…1, die anderen ganze Zahlen.
    fn s_value(&self, power: f64) -> (i64, String) {
        let v = power * self.opts.s_max;
        if self.opts.firmware == Firmware::Smoothieware {
            let k = (v * 10_000.0).round() as i64;
            (k, fmt_fixed(k, 4))
        } else {
            let k = v.round() as i64;
            (k, k.to_string())
        }
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
    let mut e = Emitter { out: Vec::new(), opts, pos: start, motion: None, feed: None, s: None, z: 0 };
    for c in &opts.comments {
        e.out.push(ascii_comment(c));
    }
    e.out.push("G21".into());
    e.out.push(match opts.coord_mode {
        CoordMode::Absolute => "G90".into(),
        CoordMode::Relative => "G91".into(),
    });
    let laser_on = if opts.dynamic_power { "M4" } else { "M3" };
    match opts.firmware {
        Firmware::Grbl => e.out.push(format!("{laser_on} S0")),
        // Inline-Modus: Leistung gilt nur für G1-Bewegungen, G0 bleibt dunkel.
        Firmware::Marlin => e.out.push(format!("{laser_on} I S0")),
        // Das Lasermodul feuert nur bei G1 mit S; kein Einschaltbefehl.
        Firmware::Smoothieware => {}
    }
    e.s = (opts.firmware != Firmware::Smoothieware).then_some(0);

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
