//! Abgebrochenen Job fortsetzen: G-Code ab einer Zeile, mit wiederhergestelltem Zustand (Einheiten, absolute
//! Koordinaten, Laser-Modus, Vorschub, Leistung, Luft) und Anfahrt mit ausgeschaltetem Laser. Fortgesetzt wird nur an
//! Leerfahrten – bei Füllungen also am Zeilenwechsel –, damit keine Linie mitten im Brennen beginnt.

use vl_core::{LineBuf, Point};

/// Stelle, an der ein Job fortgesetzt werden kann.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResumePoint {
    /// Index der ersten Zeile, die erneut gesendet wird.
    pub line: usize,
    /// Kopfposition vor dieser Zeile (Maschinenkoordinaten).
    pub at: Point,
    /// Wo danach als Erstes wieder gelasert wird.
    pub burn: Point,
}

#[derive(Clone, Debug, Default)]
struct State {
    inch: bool,
    relative: bool,
    /// Bewegungsart (0 = Eilgang, 1 = Vorschub, 2/3 = Bögen).
    motion: u8,
    feed: Option<f64>,
    power: f64,
    /// M3 oder M4; `None` = aus (M5).
    laser: Option<u8>,
    air: Option<u8>,
    pos: [Option<f64>; 3],
    /// Unbekannte Achse (z. B. A bei Drehachse) oder Stelle ohne festen Bezug: nicht fortsetzbar.
    unsupported: bool,
}

/// Wörter einer Zeile (Buchstabe, Zahl); Kommentare in `;` und `( )` werden übergangen.
fn words(line: &str) -> Vec<(char, f64)> {
    let code = line.find(';').map_or(line, |i| &line[..i]);
    let mut out = Vec::new();
    let mut chars = code.char_indices().peekable();
    let mut in_paren = false;
    while let Some((i, c)) = chars.next() {
        if in_paren {
            in_paren = c != ')';
            continue;
        }
        if c == '(' {
            in_paren = true;
            continue;
        }
        if !c.is_ascii_alphabetic() {
            continue;
        }
        let start = i + c.len_utf8();
        let mut end = start;
        while let Some(&(j, d)) = chars.peek() {
            if d.is_ascii_digit() || d == '.' || d == '-' || d == '+' {
                end = j + d.len_utf8();
                chars.next();
            } else {
                break;
            }
        }
        if let Ok(v) = code[start..end].parse::<f64>() {
            out.push((c.to_ascii_uppercase(), v));
        }
    }
    out
}

impl State {
    /// Wendet eine Zeile an; `Some(laser_on)` bei einer Bewegung.
    fn apply(&mut self, line: &str) -> Option<bool> {
        let trimmed = line.trim();
        if trimmed.starts_with('$') {
            return None;
        }
        let w = words(trimmed);
        let mut moved = false;
        for &(c, v) in &w {
            match c {
                'G' => match v as i64 {
                    0..=3 if v.fract() == 0.0 => self.motion = v as u8,
                    20 => self.inch = true,
                    21 => self.inch = false,
                    90 => self.relative = false,
                    91 => self.relative = true,
                    // G92 & Co. verschieben das Koordinatensystem: ab hier keine sichere Fortsetzung
                    92 | 28 | 30 | 53..=59 => self.unsupported = true,
                    _ => {}
                },
                'M' => match v as i64 {
                    3 | 4 => self.laser = Some(v as u8),
                    5 => self.laser = None,
                    7 | 8 => self.air = Some(v as u8),
                    9 => self.air = None,
                    _ => {}
                },
                'F' => self.feed = Some(v),
                'S' => self.power = v,
                'X' | 'Y' | 'Z' => {
                    let k = (c as u8 - b'X') as usize;
                    if self.relative {
                        self.unsupported = true;
                    }
                    self.pos[k] = Some(v);
                    moved = true;
                }
                'A' | 'B' | 'C' | 'U' | 'V' | 'W' => self.unsupported = true,
                _ => {}
            }
        }
        moved.then(|| self.motion != 0 && self.power > 0.0 && self.laser.is_some())
    }
}

fn xy(pos: [Option<f64>; 3]) -> Option<Point> {
    match pos {
        [Some(x), Some(y), _] => Some(Point::new(x, y)),
        _ => None,
    }
}

/// Stellen, an denen sich der Job (gesendete Zeilen ohne Kommentare) fortsetzen lässt: der Beginn jeder Leerfahrt nach
/// gebranntem Weg, sofern danach noch etwas gelasert wird. Leer, wenn der Job relativ (G91), mit Drehachse oder mit
/// verschobenem Nullpunkt arbeitet.
pub fn resume_points(lines: &LineBuf) -> Vec<ResumePoint> {
    let mut st = State::default();
    let mut out = Vec::new();
    // Stelle, deren erste Brennstelle noch aussteht
    let mut pending: Option<(usize, Point)> = None;
    let mut burned_before = true;
    for (i, line) in lines.iter().enumerate() {
        let before = st.pos;
        let Some(on) = st.apply(line) else { continue };
        if st.unsupported {
            return Vec::new();
        }
        // Vor der ersten Bewegung ist die Position unbekannt: dann zählt ihr Ziel
        let at = xy(before).or(xy(st.pos));
        if on {
            if let (Some((line, at)), Some(burn)) = (pending.take(), xy(before)) {
                out.push(ResumePoint { line, at, burn });
            }
        } else if burned_before && let Some(at) = at {
            pending = Some((i, at));
        }
        burned_before = on;
    }
    out
}

/// Wie viele Zeilen vor der zuletzt bestätigten noch gesucht wird: GRBL bestätigt eine Zeile, sobald sie im Planer
/// liegt (bis zu 15–16 Bewegungen), nicht erst, wenn sie gefahren ist.
const LOOKBACK: usize = 40;

/// Zeile, die beim Abbruch vermutlich gerade lief. Mit bekannter Kopfposition die Zeile kurz vor `acked`, deren Weg
/// dem Kopf am nächsten kommt (bei gleichem Abstand die frühere); sonst `acked` abzüglich eines Puffers.
pub fn running_line(lines: &LineBuf, acked: usize, head: Option<Point>) -> usize {
    let acked = acked.min(lines.len());
    let Some(head) = head else { return acked.saturating_sub(LOOKBACK / 2) };
    let first = acked.saturating_sub(LOOKBACK);
    let mut st = State::default();
    let mut best: Option<(f64, usize)> = None;
    for (i, line) in lines.iter().enumerate().take(acked + 1) {
        let before = xy(st.pos);
        if st.apply(line).is_none() || i < first {
            continue;
        }
        let (Some(a), Some(b)) = (before.or(xy(st.pos)), xy(st.pos)) else { continue };
        let d = distance_to_segment(head, a, b);
        if best.is_none_or(|(bd, _)| d < bd - 1e-6) {
            best = Some((d, i));
        }
    }
    best.map_or(acked.saturating_sub(LOOKBACK / 2), |(_, i)| i)
}

fn distance_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.dot(ab);
    let t = if len2 > 0.0 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

/// Letzte Fortsetzungsstelle an oder vor Zeile `line` (Index in `points`).
pub fn point_before(points: &[ResumePoint], line: usize) -> usize {
    points.iter().rposition(|p| p.line <= line).unwrap_or(0)
}

/// G-Code ab Zeile `from`: Zustand wiederherstellen, mit ausgeschaltetem Laser an die Stelle fahren, dann den Rest.
/// Die erste Zeile erhält Bewegungsart, Vorschub und Leistung ausdrücklich, falls sie diese nur geerbt hätte.
pub fn resume_gcode(lines: &LineBuf, from: usize) -> LineBuf {
    let mut st = State::default();
    for line in lines.iter().take(from) {
        st.apply(line);
    }
    let fmt = crate::gcode::fmt_mm;
    let mut out = LineBuf::new();
    out.push(if st.inch { "G20" } else { "G21" });
    out.push("G90");
    out.push("M5");
    if let Some(z) = st.pos[2] {
        out.push(&format!("G0 Z{}", fmt(z)));
    }
    if let [Some(x), Some(y), _] = st.pos {
        out.push(&format!("G0 X{} Y{}", fmt(x), fmt(y)));
    }
    if let Some(a) = st.air {
        out.push(&format!("M{a}"));
    }
    if let Some(m) = st.laser {
        out.push(&format!("M{m} S0"));
    }
    if let Some(first) = lines.get(from) {
        let w = words(first);
        let has = |c: char| w.iter().any(|&(k, _)| k == c);
        let moves = w.iter().any(|&(k, _)| matches!(k, 'X' | 'Y' | 'Z'));
        let mut line = first.trim().to_string();
        if moves {
            if !w.iter().any(|&(k, v)| k == 'G' && (0.0..=3.0).contains(&v)) {
                line = format!("G{} {line}", st.motion);
            }
            if st.motion != 0
                && !has('F')
                && let Some(f) = st.feed
            {
                line.push_str(&format!(" F{}", fmt(f)));
            }
            if st.laser.is_some() && !has('S') {
                line.push_str(&format!(" S{}", fmt(st.power)));
            }
        }
        out.push(&line);
        out.extend(lines.iter().skip(from + 1));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> LineBuf {
        LineBuf::from([
            "G21",
            "G90",
            "M4 S0",
            "G0 X10 Y50",
            "G1 X12 F8000",
            "X20 S150",
            "X22 S0",
            "X22 Y49.9",
            "X20",
            "X12 S150",
            "X10 S0",
            "G0 X30 Y30",
            "G1 X40 S500 F1000",
            "M5",
            "G0 X0 Y0",
        ])
    }

    #[test]
    fn points_are_starts_of_travel_after_burning() {
        let p = resume_points(&job());
        let lines: Vec<usize> = p.iter().map(|r| r.line).collect();
        // erste Bewegung, nach dem ersten Brennen, nach dem zweiten; nach der letzten Linie folgt nichts mehr
        assert_eq!(lines, vec![3, 6, 10]);
        assert_eq!(p[1].at, Point::new(20.0, 50.0));
        assert_eq!(p[1].burn, Point::new(20.0, 49.9));
        assert_eq!(p[2].at, Point::new(12.0, 49.9));
        assert_eq!(p[2].burn, Point::new(30.0, 30.0));
        assert_eq!(point_before(&p, 9), 1);
        assert_eq!(point_before(&p, 2), 0);
    }

    #[test]
    fn running_line_follows_head_position() {
        let job = job();
        // Kopf auf dem Rückweg der zweiten Zeile, obwohl schon alles bestätigt ist
        assert_eq!(running_line(&job, 15, Some(Point::new(16.0, 49.9))), 9);
        assert_eq!(running_line(&job, 15, Some(Point::new(11.0, 49.9))), 10);
        // ohne Kopfposition mit Puffer
        assert_eq!(running_line(&job, 30, None), 0);
    }

    #[test]
    fn resumed_gcode_restores_state_and_moves_with_laser_off() {
        let g = resume_gcode(&job(), 6);
        assert_eq!(
            g,
            [
                "G21",
                "G90",
                "M5",
                "G0 X20 Y50",
                "M4 S0",
                "G1 X22 S0 F8000",
                "X22 Y49.9",
                "X20",
                "X12 S150",
                "X10 S0",
                "G0 X30 Y30",
                "G1 X40 S500 F1000",
                "M5",
                "G0 X0 Y0",
            ]
        );
        // fehlende Wörter werden ergänzt
        let g = resume_gcode(&job(), 7);
        assert_eq!(&g[5], "G1 X22 Y49.9 F8000 S0");
    }

    #[test]
    fn relative_or_rotary_jobs_cannot_be_resumed() {
        assert!(resume_points(&LineBuf::from(["G91", "G0 X1 Y1", "G1 X2 S100 F100", "G0 X3"])).is_empty());
        assert!(resume_points(&LineBuf::from(["G90", "M4 S0", "G0 X1 A10", "G1 X2 S100 F100", "G0 X3"])).is_empty());
    }

    #[test]
    fn comments_are_ignored() {
        let w = words("G1 X1.5 (Kommentar X9) Y-2 ; S100");
        assert_eq!(w, vec![('G', 1.0), ('X', 1.5), ('Y', -2.0)]);
    }
}
