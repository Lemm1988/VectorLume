//! Laufzeitschätzung mit einfachem Beschleunigungsmodell.

use crate::toolpath::{Op, Toolpath};
use vl_core::Point;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Estimate {
    pub seconds: f64,
    /// Strecke mit Laser an (mm).
    pub burn_distance: f64,
    /// Strecke ohne Laser (mm), inkl. Overscan.
    pub travel_distance: f64,
}

impl Estimate {
    pub fn format_duration(&self) -> String {
        format_duration(self.seconds)
    }
}

pub fn format_duration(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    let (h, m, s) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02} h") } else { format!("{m}:{s:02} min") }
}

/// Richtungswechsel ab diesem Winkel gelten als Halt (Kosinus von ca. 25°).
const SMOOTH_COS: f64 = 0.906;

struct Motion {
    len: f64,
    speed: f64,
    dir: Point,
}

pub fn estimate(tp: &Toolpath, start: Point, rapid_rate: f64, acceleration: f64) -> Estimate {
    let accel = acceleration.max(1.0);
    let mut est = Estimate::default();
    let mut motions: Vec<Motion> = Vec::new();
    let mut cur = start;
    for block in &tp.blocks {
        let feed = block.feed.max(1.0) / 60.0;
        for op in &block.ops {
            let (to, speed, burning) = match *op {
                Op::Rapid(p) => (p, rapid_rate.max(1.0) / 60.0, false),
                Op::Move { to, power } => (to, feed, power > 0.0),
            };
            let len = cur.distance(to);
            if len > 1e-9 {
                if burning {
                    est.burn_distance += len;
                } else {
                    est.travel_distance += len;
                }
                motions.push(Motion { len, speed, dir: (to - cur) * (1.0 / len) });
            }
            cur = to;
        }
    }
    for (i, m) in motions.iter().enumerate() {
        let joins = |a: &Motion, b: &Motion| a.dir.dot(b.dir) >= SMOOTH_COS && (a.speed - b.speed).abs() < 1e-6;
        let stop_before = i == 0 || !joins(&motions[i - 1], m);
        let stop_after = i + 1 == motions.len() || !joins(m, &motions[i + 1]);
        let stops = stop_before as u8 + stop_after as u8;
        let ramp_len = m.speed * m.speed / (2.0 * accel);
        est.seconds += if stops == 2 && m.len < 2.0 * ramp_len {
            2.0 * (m.len / accel).sqrt()
        } else {
            m.len / m.speed + stops as f64 * m.speed / (2.0 * accel)
        };
    }
    est
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolpath::{Block, BlockKind};

    #[test]
    fn straight_line_time() {
        let tp = Toolpath {
            blocks: vec![Block {
                layer: 0,
                kind: BlockKind::Line,
                pass: 0,
                feed: 600.0,
                air: false,
                ops: vec![Op::Move { to: Point::new(100.0, 0.0), power: 1.0 }],
                raster: None,
                z: None,
            }],
            rasters: vec![],
        };
        // 100 mm bei 10 mm/s = 10 s, plus Anfahren/Bremsen 2 * 10/(2*1000)
        let e = estimate(&tp, Point::ZERO, 6000.0, 1000.0);
        assert!((e.seconds - 10.01).abs() < 1e-9, "{}", e.seconds);
        assert_eq!(e.burn_distance, 100.0);
        assert_eq!(format_duration(3725.0), "1:02:05 h");
    }
}
