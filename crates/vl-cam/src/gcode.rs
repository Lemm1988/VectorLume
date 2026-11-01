//! G-Code-Erzeugung für GRBL-Lasersteuerungen (GRBL 1.1 im Laser-Modus `$32=1`).

use crate::toolpath::{BlockKind, Op, Toolpath};
use vl_core::{Firmware, LineBuf, Point};

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
    /// Umkehrspiel je Ausgabeachse in mm (0 = kein Ausgleich), siehe [`Emitter::plan`].
    pub backlash: Point,
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
    out: LineBuf,
    /// Wiederverwendeter Puffer für die gerade entstehende Zeile.
    scratch: String,
    opts: &'a GcodeOptions,
    /// Zuletzt ausgegebene Motorposition; `None` = unbekannt (Absolutmodus vor der ersten Bewegung).
    pos: Option<(i64, i64)>,
    motion: Option<u8>,
    feed: Option<i64>,
    s: Option<i64>,
    /// Z-Höhe in µm relativ zur Fokushöhe beim Jobstart.
    z: i64,
    /// Umkehrspiel je Achse in µm.
    backlash: [i64; 2],
    /// Sollposition des Kopfes (ohne Spielausgleich) nach der letzten Bewegung.
    target: Option<(i64, i64)>,
    /// Letzte Bewegungsrichtung je Achse (+1/−1, 0 = unbekannt).
    dir: [i8; 2],
    /// Aktueller Ausgleich: Motorposition = Sollposition + `comp`.
    comp: [i64; 2],
    /// Der laufende Block wird ausgeglichen (nur Linienebenen, siehe [`emit`]).
    compensate: bool,
}

impl Emitter<'_> {
    /// Umkehrspiel ausgleichen. Ein Riemen- oder Spindelantrieb hat Spiel: kehrt eine Achse die Richtung um, dreht der
    /// Motor erst um das Spiel `b`, bevor sich der Kopf bewegt. Nach einer Bewegung in −-Richtung steht der Kopf deshalb
    /// um `b` weiter im Plus als der Motor. Ausgleich: in −-Richtung wird `b` weniger weit gefahren (Motor = Soll − b),
    /// in +-Richtung genau auf das Soll.
    ///
    /// Gibt die Motorposition für `target` zurück und – nur bei Vorschub und einer echten Umkehr – zusätzlich die
    /// Motorposition, an der das Spiel aufgeholt wird: Dieser kurze Zug bewegt nur den Motor, der Kopf steht still,
    /// danach verläuft die Bahn ohne Knick. Bei Eilgängen wird der Ausgleich einfach mitgefahren.
    fn plan(&mut self, target: (i64, i64), feed: bool) -> ((i64, i64), Option<(i64, i64)>) {
        let mut reversed = false;
        let before = self.target;
        if let Some(cur) = before {
            let delta = [target.0 - cur.0, target.1 - cur.1];
            for (dir, delta) in self.dir.iter_mut().zip(delta) {
                let d = delta.signum() as i8;
                if d != 0 && d != *dir {
                    reversed |= *dir != 0;
                    *dir = d;
                }
            }
        }
        let comp = std::array::from_fn(|k| if self.compensate && self.dir[k] < 0 { -self.backlash[k] } else { 0 });
        // Absolut nie unter 0: Maschinen mit Softlimits meldeten sonst einen Alarm. Am Nullpunktrand (näher als das
        // Spiel) bleibt der Ausgleich dadurch unvollständig.
        let absolute = self.opts.coord_mode == CoordMode::Absolute;
        let motor = |t: (i64, i64)| {
            let m = (t.0 + comp[0], t.1 + comp[1]);
            if absolute {
                (if t.0 >= 0 { m.0.max(0) } else { m.0 }, if t.1 >= 0 { m.1.max(0) } else { m.1 })
            } else {
                m
            }
        };
        let take_up = match before {
            Some(cur) if feed && reversed && comp != self.comp => Some(motor(cur)),
            _ => None,
        };
        self.comp = comp;
        self.target = Some(target);
        (motor(target), take_up)
    }

    /// Hängt die Achsenwörter für die Motorposition `target` (µm) an `line` an; `false`, wenn sie sich nicht ändert.
    fn push_axes(&mut self, line: &mut String, target: (i64, i64)) -> bool {
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
        let (motor, _) = self.plan((to_um(p.x), to_um(p.y)), false);
        self.rapid_motor(motor);
    }

    fn rapid_motor(&mut self, motor: (i64, i64)) {
        let mut line = std::mem::take(&mut self.scratch);
        line.clear();
        line.push_str("G0 ");
        if self.push_axes(&mut line, motor) {
            self.motion = Some(0);
            self.out.push(&line);
        }
        self.scratch = line;
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
        self.out.push(&format!("G0 Z{}", fmt_um(v)));
        self.motion = Some(0);
        self.z = target;
    }

    fn feed_move(&mut self, p: Point, feed: f64, power: f32) {
        let (motor, take_up) = self.plan((to_um(p.x), to_um(p.y)), true);
        if let Some(t) = take_up {
            // gleiche Leistung: keine Leerfahrt mitten in der Kontur (sonst entstünde dort eine Fortsetzungsstelle)
            self.feed_motor(t, feed, power);
        }
        self.feed_motor(motor, feed, power);
    }

    fn feed_motor(&mut self, motor: (i64, i64), feed: f64, power: f32) {
        use std::fmt::Write;
        let mut line = std::mem::take(&mut self.scratch);
        line.clear();
        if self.motion != Some(1) {
            line.push_str("G1 ");
        }
        if !self.push_axes(&mut line, motor) {
            self.scratch = line;
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
        self.out.push(&line);
        self.scratch = line;
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
pub fn emit(tp: &Toolpath, map: &dyn Fn(Point) -> Point, opts: &GcodeOptions) -> LineBuf {
    let start = match opts.coord_mode {
        CoordMode::Absolute => None,
        CoordMode::Relative => Some((0, 0)),
    };
    let ops: usize = tp.blocks.iter().map(|b| b.ops.len()).sum();
    // grob 20 Zeichen je Rasterzeile
    let lines = ops + 2 * tp.blocks.len() + opts.comments.len() + 16;
    let out = LineBuf::with_capacity(lines, lines * 20);
    let backlash = [to_um(opts.backlash.x.max(0.0)), to_um(opts.backlash.y.max(0.0))];
    let mut e = Emitter {
        out,
        scratch: String::with_capacity(48),
        opts,
        pos: start,
        motion: None,
        feed: None,
        s: None,
        z: 0,
        backlash,
        target: start,
        dir: [0; 2],
        comp: [0; 2],
        compensate: false,
    };
    for c in &opts.comments {
        e.out.push(&ascii_comment(c));
    }
    e.out.push("G21");
    e.out.push(match opts.coord_mode {
        CoordMode::Absolute => "G90",
        CoordMode::Relative => "G91",
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
    if backlash != [0, 0] && tp.blocks.iter().any(|b| b.kind == BlockKind::Line && !b.ops.is_empty()) {
        prime_backlash(&mut e, tp, map);
    }

    let mut air = false;
    for block in &tp.blocks {
        if block.ops.is_empty() {
            continue;
        }
        // Füll- und Bildzeilen gleicht der gemessene Scan-Versatz aus (er enthält das Spiel bereits).
        e.compensate = block.kind == BlockKind::Line;
        let kind = match block.kind {
            BlockKind::Line => "Linie",
            BlockKind::Fill => "Fuellen",
            BlockKind::Raster => "Bild",
        };
        let name = opts.layer_names.get(block.layer).cloned().unwrap_or_else(|| format!("L{:02}", block.layer + 1));
        e.out.push(&ascii_comment(&format!("Ebene {name} - {kind} - Durchgang {}", block.pass + 1)));
        let want_air = block.air && opts.air_supported;
        if want_air != air {
            e.out.push(if want_air { &opts.air_on } else { &opts.air_off });
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
        e.out.push("M5");
    }
    if opts.z_axis {
        e.z_move(0.0);
    }
    if air {
        e.out.push(&opts.air_off);
    }
    if let Some(p) = opts.return_to {
        // Rückfahrt ohne Ausgleich: danach stimmen Motor- und Startposition wieder genau überein.
        e.compensate = false;
        e.rapid(p);
    }
    if opts.coord_mode == CoordMode::Relative {
        e.out.push("G90");
    }
    e.out.push(&ascii_comment("Ende"));
    e.out
}

/// Vor dem ersten Zug ist unbekannt, auf welcher Seite das Spiel gerade anliegt. Deshalb wird der erste Punkt von
/// einem Hilfspunkt aus angefahren, der etwas weiter Richtung Mitte des Jobs liegt (also auf der Arbeitsfläche): Diese
/// letzte Anfahrt ist länger als das Spiel, danach ist die Richtung jeder Achse bekannt.
fn prime_backlash(e: &mut Emitter, tp: &Toolpath, map: &dyn Fn(Point) -> Point) {
    let Some(first) = tp.blocks.iter().flat_map(|b| &b.ops).next().map(|op| map(op.target())) else { return };
    let mut bounds = vl_core::Rect::EMPTY;
    for op in tp.blocks.iter().flat_map(|b| &b.ops) {
        bounds.include(map(op.target()));
    }
    let c = bounds.center();
    let reach = (2.0 * e.opts.backlash.x.max(e.opts.backlash.y)).max(0.5);
    let side = |v: f64, mid: f64| if mid >= v { reach } else { -reach };
    let aux = Point::new(first.x + side(first.x, c.x), first.y + side(first.y, c.y));
    let aux_um = (to_um(aux.x), to_um(aux.y));
    e.rapid_motor(aux_um);
    // Ab hier gilt die Richtung der nächsten Bewegung (zum ersten Punkt) als bekannt, ohne Aufholzug.
    e.target = Some(aux_um);
    e.dir = [0; 2];
    e.comp = [0; 2];
}

/// Zeilen einer fremden G-Code-Datei zum Senden: ohne Kommentare, Leerzeilen, Programmklammern `%` und BOM.
pub fn lines_from_file(text: &str) -> LineBuf {
    stripped(text.trim_start_matches('\u{feff}').lines()).filter(|l| *l != "%").collect()
}

fn stripped<'a>(lines: impl Iterator<Item = &'a str>) -> impl Iterator<Item = &'a str> {
    lines.filter_map(|l| {
        let code = l.find(';').map_or(l, |i| &l[..i]).trim();
        (!code.is_empty()).then_some(code)
    })
}

/// Bereitet Zeilen für das Senden an die Steuerung vor: Kommentare und Leerzeilen entfernen.
pub fn strip_for_streaming(lines: &LineBuf) -> LineBuf {
    let mut out = LineBuf::with_capacity(lines.len(), lines.text().len());
    out.extend(stripped(lines.iter()));
    out
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
            backlash: Point::ZERO,
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
            let code = strip_for_streaming(&emit(&tp, &|p| p, &o)).to_vec();
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
        let code = strip_for_streaming(&emit(&tp, &|p| p, &marlin)).to_vec();
        assert!(code.contains(&"M4 I S0".to_string()), "{code:?}");
        assert!(code.iter().any(|l| l.ends_with("S128")), "{code:?}");
        assert!(code.contains(&"M5".to_string()));
        let smoothie = GcodeOptions { firmware: Firmware::Smoothieware, s_max: 1.0, ..opts(CoordMode::Absolute) };
        let code = strip_for_streaming(&emit(&tp, &|p| p, &smoothie)).to_vec();
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
        let code = strip_for_streaming(&emit(&tp, &|p| p, &opts(CoordMode::Absolute))).to_vec();
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
        let code = strip_for_streaming(&emit(&tp, &|p| p, &m3)).to_vec();
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
        let code = strip_for_streaming(&g).to_vec();
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

    /// Mechanik mit Umkehrspiel: der Kopf `h` folgt dem Motor `m` erst, wenn das Spiel aufgeholt ist (h ∈ [m, m + b]).
    struct Slack {
        m: [f64; 2],
        h: [f64; 2],
        b: [f64; 2],
    }

    impl Slack {
        fn go(&mut self, to: [f64; 2]) {
            self.m = to;
            for ((h, m), b) in self.h.iter_mut().zip(to).zip(self.b) {
                *h = h.clamp(m, m + b);
            }
        }
    }

    /// Fährt G-Code (absolut) auf der Mechanik ab. Ergebnis: Kopfpositionen am Ende jedes Brennzugs, bei dem sich der
    /// Kopf bewegt, und die Zahl der Knicke – Brennzüge, bei denen der Kopf dem Motor nur teilweise folgt. Erlaubt ist
    /// nur: genau mitfahren (gerade Linie) oder ganz stillstehen (Spiel wird aufgeholt).
    fn run_on_slack(
        code: &[String],
        slack: &mut Slack,
        checked: impl Fn([f64; 2]) -> bool,
    ) -> (Vec<(i64, i64)>, Vec<String>) {
        let (mut motion, mut s) = (0, 0.0);
        let mut burned = Vec::new();
        let mut kinks = Vec::new();
        for line in code {
            let mut to = slack.m;
            let mut moves = false;
            for w in line.split_whitespace() {
                let (c, v) = w.split_at(1);
                match c {
                    "G" => motion = v.parse().unwrap(),
                    "X" | "Y" => {
                        to[(c == "Y") as usize] = v.parse().unwrap();
                        moves = true;
                    }
                    "S" => s = v.parse().unwrap(),
                    _ => {}
                }
            }
            if !moves {
                continue;
            }
            let (m0, h0) = (slack.m, slack.h);
            slack.go(to);
            if motion == 1 && s > 0.0 && checked(to) {
                let follows = (0..2).all(|k| ((slack.h[k] - h0[k]) - (slack.m[k] - m0[k])).abs() < 1e-9);
                let still = (0..2).all(|k| (slack.h[k] - h0[k]).abs() < 1e-9);
                if !(follows || still) {
                    kinks.push(format!("{line}: Kopf {h0:?} → {:?}", slack.h));
                }
                if !still {
                    burned.push((to_um(slack.h[0]), to_um(slack.h[1])));
                }
            }
        }
        (burned, kinks)
    }

    #[test]
    fn backlash_compensation_lands_exactly() {
        // Kontur mit Schlitz (Umkehr in X und Y), dann eine Füllzeile hin und zurück, dann noch eine Kontur.
        let slot = [(10.0, 10.0), (30.0, 10.0), (30.0, 20.0), (21.9, 20.0), (21.9, 14.0), (18.1, 14.0), (18.1, 20.0)];
        let contour = |dx: f64| {
            let mut ops = vec![Op::Rapid(Point::new(slot[0].0 + dx, slot[0].1))];
            for &(x, y) in slot[1..].iter().chain([(10.0, 20.0), slot[0]].iter()) {
                ops.push(Op::Move { to: Point::new(x + dx, y), power: 0.8 });
            }
            ops
        };
        let block = |kind, ops| Block {
            layer: 0,
            kind,
            pass: 0,
            feed: 600.0,
            air: false,
            constant_power: false,
            ops,
            raster: None,
            z: None,
        };
        let fill = vec![
            Op::Rapid(Point::new(40.0, 5.0)),
            Op::Move { to: Point::new(50.0, 5.0), power: 0.3 },
            Op::Move { to: Point::new(50.0, 5.1), power: 0.0 },
            Op::Move { to: Point::new(40.0, 5.1), power: 0.3 },
        ];
        let tp = Toolpath {
            blocks: vec![
                block(BlockKind::Line, contour(0.0)),
                block(BlockKind::Fill, fill),
                block(BlockKind::Line, contour(50.0)),
            ],
            rasters: vec![],
        };
        let expected: Vec<(i64, i64)> = [&tp.blocks[0], &tp.blocks[2]]
            .iter()
            .flat_map(|b| &b.ops)
            .filter_map(|op| match *op {
                Op::Move { to, .. } => Some((to_um(to.x), to_um(to.y))),
                Op::Rapid(_) => None,
            })
            .collect();
        // Füllzeilen gleicht der Scan-Versatz aus, nicht der Spielausgleich
        let outside_fill = |p: [f64; 2]| p[0] < 39.0 || p[0] > 51.0;
        let b = [0.15, 0.25];
        let o = GcodeOptions { backlash: Point::new(b[0], b[1]), return_to: None, ..opts(CoordMode::Absolute) };
        let code = strip_for_streaming(&emit(&tp, &|p| p, &o)).to_vec();
        // Unbekannter Zustand vor dem Job: Spiel liegt auf der einen oder der anderen Seite an
        for start_h in [[0.0, 0.0], [b[0], b[1]], [0.0, b[1]]] {
            let mut slack = Slack { m: [0.0; 2], h: start_h, b };
            let (burned, kinks) = run_on_slack(&code, &mut slack, outside_fill);
            assert!(kinks.is_empty(), "Knicke in der Bahn: {kinks:?}");
            assert_eq!(burned, expected, "Startzustand {start_h:?}\n{}", code.join("\n"));
        }
        // Ohne Ausgleich landet derselbe Schlitz auf der Mechanik um das Spiel daneben
        let plain = strip_for_streaming(&emit(&tp, &|p| p, &opts(CoordMode::Absolute))).to_vec();
        let mut slack = Slack { m: [0.0; 2], h: [0.0; 2], b };
        let (burned, kinks) = run_on_slack(&plain, &mut slack, outside_fill);
        assert!(!kinks.is_empty() && burned != expected, "ohne Ausgleich ist die Mechanik nicht exakt");
        // Ohne Spiel ändert sich nichts gegenüber der bisherigen Ausgabe (keine zusätzlichen Zeilen)
        let zero = GcodeOptions { backlash: Point::ZERO, ..opts(CoordMode::Absolute) };
        assert_eq!(strip_for_streaming(&emit(&tp, &|p| p, &zero)).to_vec(), plain);
    }

    #[test]
    fn backlash_take_up_keeps_power_and_is_not_a_travel_move() {
        let o = GcodeOptions { backlash: Point::new(0.2, 0.2), ..opts(CoordMode::Absolute) };
        let code = strip_for_streaming(&emit(&square(), &|p| p, &o)).to_vec();
        // Aufholzüge brennen mit derselben Leistung weiter: keine zusätzliche Fortsetzungsstelle in der Kontur
        let plain = opts(CoordMode::Absolute);
        let with = crate::resume::resume_points(&strip_for_streaming(&emit(&square(), &|p| p, &o)));
        let without = crate::resume::resume_points(&strip_for_streaming(&emit(&square(), &|p| p, &plain)));
        assert_eq!(with.len(), without.len(), "{code:?}");
        assert_eq!(code.iter().filter(|l| l.contains(" S")).count(), 2, "nur Einschalten und erster Zug: {code:?}");
        // Absolut nie unter 0 (Softlimits): die Rückfahrt zu 0/0 bleibt 0/0
        assert!(!code.iter().any(|l| l.contains("X-") || l.contains("Y-")), "{code:?}");
        assert_eq!(code.last().unwrap(), "G0 X0 Y0");
        // Relativ: alle Bewegungen zusammen ergeben wieder den Start (der Ausgleich hebt sich auf)
        let rel = GcodeOptions { backlash: Point::new(0.2, 0.2), ..opts(CoordMode::Relative) };
        let code = strip_for_streaming(&emit(&square(), &|p| p, &rel)).to_vec();
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
        assert_eq!((x, y), (0, 0), "{code:?}");
    }

    #[test]
    fn relative_moves_sum_to_absolute() {
        let g = emit(&square(), &|p| p, &opts(CoordMode::Relative));
        let code = strip_for_streaming(&g).to_vec();
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
