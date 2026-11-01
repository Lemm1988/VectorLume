//! Umkehrspiel messen: ein Nonius-Muster wie beim Messschieber, je eines für X und Y.
//!
//! Die obere Strichreihe wird mit Teilung 1 mm gelasert, jeder Strich aus der einen Richtung angefahren; die untere
//! mit Teilung 1,05 mm, jeder Strich aus der Gegenrichtung. Ohne Spiel decken sich die beiden Mittelstriche. Mit Spiel
//! landet die untere Reihe um das (noch nicht ausgeglichene) Spiel versetzt – dann decken sich die Striche `k` Stellen
//! neben der Mitte, und das Restspiel beträgt `k · 0,05 mm`. Die Seite ist mit „+“ bzw. „−“ markiert: auf der
//! „+“-Seite muss der Ausgleich größer werden, auf der „−“-Seite kleiner. Das Muster läuft mit dem eingestellten
//! Ausgleich – nach dem Übernehmen der Ablesung decken sich beim nächsten Versuch die Mittelstriche.

use crate::job::{Job, finish_job};
use crate::machine::{JobSettings, MAX_BACKLASH, MachineProfile};
use crate::toolpath::{Block, BlockKind, Op, Toolpath};
use serde::{Deserialize, Serialize};
use vl_core::{Point, Rect};

/// Striche je Seite der Mitte (Messbereich ± STEPS · VERNIER).
pub const STEPS: i32 = 10;
/// Teilung der oberen Reihe (mm).
pub const PITCH: f64 = 1.0;
/// Auflösung: Unterschied der Teilungen (mm).
pub const VERNIER: f64 = 0.05;
/// Strichlänge; jeder fünfte Strich ist länger, der Mittelstrich am längsten.
const MARK: f64 = 4.0;
/// Lücke zwischen den beiden Reihen (mm).
const GAP: f64 = 0.3;
/// Anfahrweg vor jedem Strich (mm), länger als jedes zulässige Spiel.
const APPROACH: f64 = MAX_BACKLASH + 0.5;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BacklashTest {
    /// mm/min
    pub speed: f64,
    /// Prozent
    pub power: f64,
    /// Ecke des Musters, die dem Maschinen-Nullpunkt am nächsten liegt (Maschinenkoordinaten).
    pub position: Point,
}

impl Default for BacklashTest {
    fn default() -> Self {
        Self { speed: 1500.0, power: 20.0, position: Point::new(10.0, 10.0) }
    }
}

/// Neuer Ausgleich aus der Ablesung: `steps` Striche neben der Mitte, auf der „+“-Seite (`plus`) oder der „−“-Seite.
pub fn corrected(current: f64, plus: bool, steps: u32) -> f64 {
    let delta = steps as f64 * VERNIER;
    let v = if plus { current + delta } else { current - delta };
    // auf 1 µm runden: 0,15 + 0,05 soll 0,2 ergeben, nicht 0,20000000000000004
    ((v * 1000.0).round() / 1000.0).clamp(0.0, MAX_BACKLASH)
}

fn mark_len(k: i32) -> f64 {
    match k {
        0 => MARK + 2.5,
        k if k % 5 == 0 => MARK + 1.2,
        _ => MARK,
    }
}

/// Werkzeugweg in lokalen Dokumentkoordinaten (y nach unten). `dir` = Richtung von Maschinen-+X bzw. +Y im Dokument.
fn layout(power: f32, dir: Point) -> Vec<Op> {
    let mut ops = Vec::new();
    let burn = |ops: &mut Vec<Op>, from: Point, to: Point| {
        ops.push(Op::Rapid(from));
        ops.push(Op::Move { to, power });
    };
    // Nonius für eine Achse. `along` = Dokumentrichtung der gemessenen Achse (Maschine +), `across` = Strichrichtung.
    let vernier = |ops: &mut Vec<Op>, origin: Point, along: Point, across: Point| {
        for row in [false, true] {
            let pitch = if row { PITCH + VERNIER } else { PITCH };
            // obere Reihe aus Richtung −, untere aus Richtung + angefahren (letzter Zug jeweils länger als das Spiel)
            let from_side = if row { 1.0 } else { -1.0 };
            for k in -STEPS..=STEPS {
                let at = origin + along * (k as f64 * pitch);
                let (start, end) = if row {
                    (at + across * (GAP / 2.0), at + across * (GAP / 2.0 + mark_len(k)))
                } else {
                    (at - across * (GAP / 2.0), at - across * (GAP / 2.0 + mark_len(k)))
                };
                ops.push(Op::Rapid(start + along * (from_side * APPROACH)));
                ops.push(Op::Rapid(start));
                ops.push(Op::Move { to: end, power });
            }
        }
        // „+“ am Ende der Maschinen-−-Seite, „−“ an der +-Seite (dort decken sich die Striche bei zu wenig bzw. zu
        // viel Ausgleich). Querstrich jeweils in Strichrichtung gesehen waagerecht.
        let reach = (STEPS as f64 + 3.0) * (PITCH + VERNIER);
        let plus = origin - along * reach;
        let minus = origin + along * reach;
        let h = Point::new(1.0, 0.0) * 1.2;
        let v = Point::new(0.0, 1.0) * 1.2;
        burn(ops, plus - h, plus + h);
        burn(ops, plus - v, plus + v);
        burn(ops, minus - h, minus + h);
    };
    let x_dir = Point::new(dir.x, 0.0);
    let y_dir = Point::new(0.0, dir.y);
    let span = (STEPS as f64 + 4.5) * (PITCH + VERNIER);
    // X-Nonius waagerecht, Striche senkrecht; Y-Nonius rechts daneben, Striche waagerecht
    let x_origin = Point::new(span, 12.0);
    vernier(&mut ops, x_origin, x_dir, Point::new(0.0, 1.0));
    let y_origin = Point::new(2.0 * span + 10.0, span);
    vernier(&mut ops, y_origin, y_dir, Point::new(1.0, 0.0));
    // Achsbuchstaben aus Strichen, über bzw. neben dem jeweiligen Nonius
    let x0 = Point::new(1.0, 1.0);
    burn(&mut ops, x0, x0 + Point::new(3.0, 4.0));
    burn(&mut ops, x0 + Point::new(0.0, 4.0), x0 + Point::new(3.0, 0.0));
    let y0 = Point::new(2.0 * span + 18.0, 1.0);
    burn(&mut ops, y0, y0 + Point::new(1.5, 2.0));
    burn(&mut ops, y0 + Point::new(3.0, 0.0), y0 + Point::new(1.5, 2.0));
    burn(&mut ops, y0 + Point::new(1.5, 2.0), y0 + Point::new(1.5, 4.0));
    ops
}

impl BacklashTest {
    pub fn validate(&self) -> Result<(), String> {
        if !(self.speed > 0.0 && self.speed.is_finite()) {
            return Err("Die Geschwindigkeit muss größer als 0 sein.".into());
        }
        if !(self.power > 0.0 && self.power <= 100.0) {
            return Err("Die Leistung muss zwischen 0 und 100 % liegen.".into());
        }
        Ok(())
    }

    /// Fertiger Job. Läuft mit dem Spielausgleich des Profils, misst also das noch verbleibende Spiel.
    pub fn build(&self, profile: &MachineProfile, settings: &JobSettings) -> Result<Job, String> {
        self.validate()?;
        let power = (self.power / 100.0).clamp(0.0, 1.0) as f32;
        let dir = profile.screen_to_machine_delta(Point::new(1.0, 1.0));
        let mut ops = layout(power, dir);
        // lokal → Dokument: die Ecke am Maschinen-Nullpunkt auf `position` legen
        let mut b = Rect::EMPTY;
        for op in &ops {
            b.include(op.target());
        }
        let corner =
            Point::new(if dir.x > 0.0 { b.min.x } else { b.max.x }, if dir.y > 0.0 { b.min.y } else { b.max.y });
        let shift = profile.machine_to_doc(self.position) - corner;
        for op in &mut ops {
            match op {
                Op::Rapid(p) => *p += shift,
                Op::Move { to, .. } => *to += shift,
            }
        }
        let bounds = Rect::from_points(b.min + shift, b.max + shift);
        let tp = Toolpath {
            blocks: vec![Block {
                layer: 0,
                kind: BlockKind::Line,
                pass: 0,
                feed: self.speed,
                air: false,
                constant_power: false,
                ops,
                raster: None,
                z: None,
            }],
            rasters: vec![],
        };
        let settings = JobSettings { selected_only: false, ..settings.clone() };
        Ok(finish_job(tp, bounds, profile, &settings, vec!["Spieltest".into()], Some("Spiel messen".into())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OriginCorner;

    /// Mechanik mit Spiel je Achse (siehe `gcode`-Tests): Kopf h ∈ [m, m + b].
    fn run(code: &vl_core::LineBuf, b: [f64; 2]) -> Vec<([f64; 2], [f64; 2])> {
        let (mut m, mut h) = ([0.0f64; 2], [0.0f64; 2]);
        let (mut motion, mut s) = (0, 0.0);
        let mut lines = Vec::new();
        for line in code.iter() {
            let line = line.split(';').next().unwrap();
            let mut to = m;
            let mut moved = false;
            for w in line.split_whitespace() {
                let (c, v) = w.split_at(1);
                match c {
                    "G" if v == "0" || v == "1" => motion = v.parse().unwrap(),
                    "X" | "Y" => {
                        to[(c == "Y") as usize] = v.parse().unwrap();
                        moved = true;
                    }
                    "S" => s = v.parse().unwrap(),
                    _ => {}
                }
            }
            if !moved {
                continue;
            }
            let h0 = h;
            m = to;
            for k in 0..2 {
                h[k] = h[k].clamp(m[k], m[k] + b[k]);
            }
            if motion == 1 && s > 0.0 {
                lines.push((h0, h));
            }
        }
        lines
    }

    /// Liest den Nonius ab wie ein Mensch: wo sich obere und untere Striche am besten decken. Ergebnis: Index der Stelle
    /// (negativ = Maschinen-−-Seite) für X und Y.
    fn read(lines: &[([f64; 2], [f64; 2])]) -> [i32; 2] {
        std::array::from_fn(|axis| {
            // Striche quer zur Achse: Anfang und Ende haben dieselbe Koordinate auf der gemessenen Achse
            let marks: Vec<(f64, f64)> = lines
                .iter()
                .filter(|(a, b)| (a[axis] - b[axis]).abs() < 1e-9 && (a[1 - axis] - b[1 - axis]).abs() > 3.0)
                .map(|(a, b)| (a[axis], (a[1 - axis] + b[1 - axis]) / 2.0))
                .collect();
            assert_eq!(marks.len(), 2 * (2 * STEPS as usize + 1), "Achse {axis}");
            let mid = marks.iter().map(|m| m.1).sum::<f64>() / marks.len() as f64;
            let mut upper: Vec<f64> = marks.iter().filter(|m| m.1 < mid).map(|m| m.0).collect();
            let mut lower: Vec<f64> = marks.iter().filter(|m| m.1 > mid).map(|m| m.0).collect();
            upper.sort_by(f64::total_cmp);
            lower.sort_by(f64::total_cmp);
            // Reihe mit Teilung 1 mm = kleinere Spannweite
            let span = |v: &[f64]| v[v.len() - 1] - v[0];
            if span(&upper) > span(&lower) {
                std::mem::swap(&mut upper, &mut lower);
            }
            let best = (0..upper.len())
                .min_by(|&i, &j| (upper[i] - lower[i]).abs().total_cmp(&(upper[j] - lower[j]).abs()))
                .unwrap();
            best as i32 - STEPS
        })
    }

    #[test]
    fn reading_the_vernier_gives_the_backlash_and_corrects_it() {
        let b = [0.2, 0.35];
        for origin in OriginCorner::ALL {
            let mut profile = MachineProfile { origin, ..MachineProfile::default() };
            let test = BacklashTest::default();
            // 1. Durchgang ohne Ausgleich: das volle Spiel, auf der Maschinen-−-Seite („+“: mehr Ausgleich)
            let job = test.build(&profile, &JobSettings::default()).unwrap();
            let k = read(&run(&job.gcode, b));
            assert_eq!(k, [-4, -7], "{origin:?}");
            // Ablesung übernehmen
            profile.backlash_x = corrected(profile.backlash_x, k[0] < 0, k[0].unsigned_abs());
            profile.backlash_y = corrected(profile.backlash_y, k[1] < 0, k[1].unsigned_abs());
            assert_eq!((profile.backlash_x, profile.backlash_y), (0.2, 0.35));
            // 2. Durchgang: die Mittelstriche decken sich
            let job = test.build(&profile, &JobSettings::default()).unwrap();
            assert_eq!(read(&run(&job.gcode, b)), [0, 0], "{origin:?}");
            // Zu viel Ausgleich zeigt sich auf der „−“-Seite
            profile.backlash_x = 0.3;
            let job = test.build(&profile, &JobSettings::default()).unwrap();
            assert_eq!(read(&run(&job.gcode, b))[0], 2, "{origin:?}");
            // Das Muster liegt an der gewählten Stelle auf der Arbeitsfläche
            assert!(job.warnings.is_empty(), "{:?}", job.warnings);
            let mb = job.mapping.map(job.design_bounds.min);
            let mb2 = job.mapping.map(job.design_bounds.max);
            let (lo_x, lo_y) = (mb.x.min(mb2.x), mb.y.min(mb2.y));
            assert!((lo_x - 10.0).abs() < 1e-6 && (lo_y - 10.0).abs() < 1e-6, "{origin:?}: {mb:?} {mb2:?}");
        }
    }

    #[test]
    fn corrected_rounds_and_clamps() {
        assert_eq!(corrected(0.15, true, 1), 0.2);
        assert_eq!(corrected(0.1, false, 4), 0.0);
        assert_eq!(corrected(1.9, true, 10), MAX_BACKLASH);
        assert!(BacklashTest { power: 0.0, ..Default::default() }.validate().is_err());
    }
}
