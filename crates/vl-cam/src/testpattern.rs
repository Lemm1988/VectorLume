//! Materialtest: Raster aus Feldern mit gestaffelten Einstellungen (z. B. Leistung × Geschwindigkeit),
//! beschriftet und direkt als Job ausgebbar.

use crate::job::{Job, finish_job, placement};
use crate::machine::{JobSettings, MachineProfile, StartFrom};
use crate::optimize::order_polylines;
use crate::toolpath::{Block, BlockKind, Toolpath, hatch_ops, line_ops};
use serde::{Deserialize, Serialize};
use vl_core::path::bounds_of;
use vl_core::{Affine, CutSettings, LayerMode, Point, Polyline, Rect};

/// Liefert die Umrisse eines Textes mit der Oberkante bei y = 0 und dem linken Rand bei x = 0 (Höhe in mm).
pub type LabelFn<'a> = &'a dyn Fn(&str, f64) -> Vec<Polyline>;

pub const MAX_STEPS: u32 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestParam {
    Speed,
    Power,
    Passes,
    Interval,
    /// Fokustest (nur mit Z-Achse sinnvoll).
    Focus,
}

impl TestParam {
    pub const ALL: [TestParam; 5] =
        [TestParam::Speed, TestParam::Power, TestParam::Passes, TestParam::Interval, TestParam::Focus];

    pub fn label(self) -> &'static str {
        match self {
            TestParam::Speed => "Geschwindigkeit",
            TestParam::Power => "Leistung",
            TestParam::Passes => "Durchgänge",
            TestParam::Interval => "Zeilenabstand",
            TestParam::Focus => "Z-Versatz",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            TestParam::Speed => "mm/min",
            TestParam::Power => "%",
            TestParam::Passes => "x",
            TestParam::Interval | TestParam::Focus => "mm",
        }
    }

    /// Erlaubter Wertebereich.
    pub fn range(self) -> (f64, f64) {
        match self {
            TestParam::Speed => (1.0, 100_000.0),
            TestParam::Power => (0.0, 100.0),
            TestParam::Passes => (1.0, 50.0),
            TestParam::Interval => (0.02, 2.0),
            TestParam::Focus => (-20.0, 20.0),
        }
    }

    pub fn format(self, v: f64) -> String {
        match self {
            TestParam::Speed | TestParam::Passes => format!("{v:.0}"),
            TestParam::Power => {
                if (v - v.round()).abs() < 1e-6 {
                    format!("{v:.0}")
                } else {
                    format!("{v:.1}")
                }
            }
            TestParam::Interval => format!("{v:.2}"),
            TestParam::Focus => format!("{v:+.1}"),
        }
    }

    fn apply(self, cs: &mut CutSettings, v: f64) {
        match self {
            TestParam::Speed => cs.speed = v,
            TestParam::Power => cs.power_max = v,
            TestParam::Passes => cs.passes = v.round().max(1.0) as u32,
            TestParam::Interval => cs.interval = v,
            TestParam::Focus => cs.z_offset = v,
        }
    }
}

/// Eine Achse des Testrasters: ein Parameter, linear von `min` bis `max` in `count` Stufen.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TestAxis {
    pub param: TestParam,
    pub min: f64,
    pub max: f64,
    pub count: u32,
}

impl TestAxis {
    pub fn value(&self, i: u32) -> f64 {
        let n = self.count.max(1);
        let v = if n == 1 { self.min } else { self.min + (self.max - self.min) * i as f64 / (n - 1) as f64 };
        if self.param == TestParam::Passes { v.round() } else { v }
    }

    fn title(&self) -> String {
        format!("{} ({})", self.param.label(), self.param.unit())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TestPattern {
    /// Grundeinstellungen für alle Felder (Modus Linie oder Füllen); die Achsen überschreiben einzelne Werte.
    pub base: CutSettings,
    pub columns: TestAxis,
    pub rows: TestAxis,
    pub cell_width: f64,
    pub cell_height: f64,
    pub gap: f64,
    pub labels: bool,
    pub label_height: f64,
    pub label_speed: f64,
    pub label_power: f64,
    /// Linke untere Ecke in Maschinenkoordinaten (nur bei „Absolute Koordinaten“ von Bedeutung).
    pub position: Point,
}

impl Default for TestPattern {
    fn default() -> Self {
        Self {
            base: CutSettings::default_for(LayerMode::Fill),
            columns: TestAxis { param: TestParam::Speed, min: 2000.0, max: 10000.0, count: 5 },
            rows: TestAxis { param: TestParam::Power, min: 10.0, max: 50.0, count: 5 },
            cell_width: 8.0,
            cell_height: 8.0,
            gap: 3.0,
            labels: true,
            label_height: 2.5,
            label_speed: 3000.0,
            label_power: 20.0,
            position: Point::new(10.0, 10.0),
        }
    }
}

/// Lage aller Elemente in Dokumentkoordinaten.
#[derive(Clone, Debug, Default)]
pub struct TestLayout {
    /// (Zeile, Spalte, Feld)
    pub cells: Vec<(u32, u32, Rect)>,
    pub labels: Vec<Polyline>,
    pub bounds: Rect,
}

impl TestPattern {
    pub fn cell_count(&self) -> u32 {
        self.rows.count.max(1) * self.columns.count.max(1)
    }

    /// Einstellungen eines Feldes.
    pub fn cell_settings(&self, row: u32, col: u32) -> CutSettings {
        let mut cs = self.base.clone();
        if cs.mode == LayerMode::Image {
            cs.mode = LayerMode::Fill;
        }
        self.columns.param.apply(&mut cs, self.columns.value(col));
        self.rows.param.apply(&mut cs, self.rows.value(row));
        cs
    }

    pub fn validate(&self) -> Result<(), String> {
        for (name, axis) in [("Spalten", &self.columns), ("Zeilen", &self.rows)] {
            if !(1..=MAX_STEPS).contains(&axis.count) {
                return Err(format!("{name}: 1 bis {MAX_STEPS} Stufen erlaubt."));
            }
            let (lo, hi) = axis.param.range();
            for v in [axis.min, axis.max] {
                if !(lo..=hi).contains(&v) {
                    return Err(format!(
                        "{name}: {} muss zwischen {} und {} {} liegen.",
                        axis.param.label(),
                        axis.param.format(lo),
                        axis.param.format(hi),
                        axis.param.unit()
                    ));
                }
            }
        }
        if self.columns.param == self.rows.param {
            return Err("Zeilen und Spalten müssen verschiedene Parameter testen.".into());
        }
        if self.cell_width < 1.0 || self.cell_height < 1.0 {
            return Err("Felder müssen mindestens 1 × 1 mm groß sein.".into());
        }
        if self.base.speed <= 0.0 || (self.labels && self.label_speed <= 0.0) {
            return Err("Die Geschwindigkeit muss größer als 0 sein.".into());
        }
        Ok(())
    }

    /// Ordnet Felder und Beschriftung an; die linke obere Ecke des Ganzen liegt bei `origin` (Dokument).
    pub fn layout_at(&self, origin: Point, label: LabelFn) -> TestLayout {
        let (cols, rows) = (self.columns.count.max(1), self.rows.count.max(1));
        let h = self.label_height;
        let pad = (h * 0.6).max(1.0);
        let mut labels = Vec::new();
        let mut col_labels = Vec::new();
        let mut row_labels = Vec::new();
        let (mut left, mut top) = (0.0, 0.0);
        if self.labels {
            for c in 0..cols {
                col_labels.push(label(&self.columns.param.format(self.columns.value(c)), h));
            }
            for r in 0..rows {
                row_labels.push(label(&self.rows.param.format(self.rows.value(r)), h));
            }
            let row_label_w = row_labels.iter().map(|p| bounds_of(p).max.x).fold(0.0, f64::max);
            // Platz links: senkrechter Achsentitel + Zeilenwerte; oben: Achsentitel + Spaltenwerte.
            left = h + pad + row_label_w + pad;
            top = 2.0 * (h + pad);
        }
        let step_x = self.cell_width + self.gap;
        let step_y = self.cell_height + self.gap;
        let grid_w = cols as f64 * step_x - self.gap;
        let grid_h = rows as f64 * step_y - self.gap;
        let grid = Point::new(origin.x + left, origin.y + top);

        let mut cells = Vec::with_capacity((cols * rows) as usize);
        for r in 0..rows {
            for c in 0..cols {
                let rect = Rect::from_xywh(
                    grid.x + c as f64 * step_x,
                    grid.y + r as f64 * step_y,
                    self.cell_width,
                    self.cell_height,
                );
                cells.push((r, c, rect));
            }
        }

        if self.labels {
            let mut place = |polys: Vec<Polyline>, t: Affine| {
                labels.extend(polys.into_iter().map(|mut p| {
                    p.transform(&t);
                    p
                }));
            };
            // Spaltenwerte zentriert über den Feldern, Spaltentitel darüber linksbündig.
            for (c, polys) in col_labels.into_iter().enumerate() {
                let w = bounds_of(&polys).max.x;
                let x = grid.x + c as f64 * step_x + (self.cell_width - w) / 2.0;
                place(polys, Affine::translate(x, grid.y - pad - h));
            }
            place(label(&self.columns.title(), h), Affine::translate(grid.x, origin.y));
            // Zeilenwerte rechtsbündig links der Felder, Zeilentitel senkrecht ganz links (von unten nach oben lesbar).
            for (r, polys) in row_labels.into_iter().enumerate() {
                let w = bounds_of(&polys).max.x;
                let y = grid.y + r as f64 * step_y + (self.cell_height - h) / 2.0;
                place(polys, Affine::translate(grid.x - pad - w, y));
            }
            let title = label(&self.rows.title(), h);
            let tw = bounds_of(&title).max.x;
            let ty = grid.y + (grid_h + tw) / 2.0;
            place(title, Affine::rotate(-std::f64::consts::FRAC_PI_2).then(&Affine::translate(origin.x, ty)));
        }

        let mut bounds = Rect::from_xywh(grid.x, grid.y, grid_w, grid_h);
        bounds = bounds.union(&bounds_of(&labels));
        TestLayout { cells, labels, bounds }
    }

    /// Anordnung im Dokument; bei absoluten Koordinaten liegt die linke untere Ecke an `position`.
    pub fn layout(&self, profile: &MachineProfile, label: LabelFn) -> TestLayout {
        let at_zero = self.layout_at(Point::ZERO, label);
        let b = at_zero.bounds;
        let m0 = profile.doc_to_machine(b.min);
        let m1 = profile.doc_to_machine(b.max);
        let machine_min = Point::new(m0.x.min(m1.x), m0.y.min(m1.y));
        // Dokumentpunkt, der nach der Verschiebung auf der Maschine bei m0 + Verschiebung liegt.
        let target = profile.machine_to_doc(m0 + (self.position - machine_min));
        self.layout_at(target - b.min, label)
    }

    /// Erzeugt den fertigen Job (Beschriftung zuerst, dann die Felder zeilenweise im Zickzack).
    pub fn build(&self, profile: &MachineProfile, settings: &JobSettings, label: LabelFn) -> Result<Job, String> {
        self.validate()?;
        let settings = JobSettings { selected_only: false, ..settings.clone() };
        let layout = self.layout(profile, label);
        let (_, _, start) = placement(&layout.bounds, profile, &settings);
        let mut tp = Toolpath::default();
        let mut names = Vec::new();

        if !layout.labels.is_empty() {
            let ordered = order_polylines(layout.labels.clone(), start, false);
            let power = (self.label_power / 100.0).clamp(0.0, 1.0) as f32;
            tp.blocks.push(Block {
                layer: 0,
                kind: BlockKind::Line,
                pass: 0,
                feed: self.label_speed,
                air: self.base.air_assist,
                ops: line_ops(&ordered, power),
                raster: None,
                z: self.base.z_for_pass(0),
            });
            names.push("Beschriftung".to_string());
        }

        let cols = self.columns.count.max(1);
        for r in 0..self.rows.count.max(1) {
            let order: Vec<u32> = if r % 2 == 0 { (0..cols).collect() } else { (0..cols).rev().collect() };
            for c in order {
                let rect = layout.cells[(r * cols + c) as usize].2;
                let cs = self.cell_settings(r, c);
                let power = (cs.power_max / 100.0).clamp(0.0, 1.0) as f32;
                let outline = Polyline::new(rect.corners().to_vec(), true);
                let (kind, ops) = match cs.mode {
                    LayerMode::Line => (BlockKind::Line, line_ops(&[outline], power)),
                    _ => {
                        let polys = [outline];
                        let mut ops =
                            hatch_ops(&polys, cs.interval, cs.fill_angle, cs.bidirectional, cs.overscan, power);
                        if cs.crosshatch {
                            ops.extend(hatch_ops(
                                &polys,
                                cs.interval,
                                cs.fill_angle + 90.0,
                                cs.bidirectional,
                                cs.overscan,
                                power,
                            ));
                        }
                        (BlockKind::Fill, ops)
                    }
                };
                let layer = names.len();
                names.push(format!(
                    "Feld Z{} S{}: {} {} / {} {}",
                    r + 1,
                    c + 1,
                    self.rows.param.format(self.rows.value(r)),
                    self.rows.param.unit(),
                    self.columns.param.format(self.columns.value(c)),
                    self.columns.param.unit()
                ));
                for pass in 0..cs.passes.max(1) {
                    tp.blocks.push(Block {
                        layer,
                        kind,
                        pass,
                        feed: cs.speed,
                        air: cs.air_assist,
                        ops: ops.clone(),
                        raster: None,
                        z: cs.z_for_pass(pass),
                    });
                }
            }
        }
        let title = format!("Materialtest: Zeilen {}, Spalten {}", self.rows.param.label(), self.columns.param.label());
        let mut job = finish_job(tp, layout.bounds, profile, &settings, names, Some(title));
        if settings.start_from != StartFrom::Absolute {
            // Die Position gilt nur für absolute Koordinaten; sonst bestimmt der Ankerpunkt die Lage.
            job.warnings.retain(|w| !w.contains("außerhalb"));
        }
        Ok(job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gcode::strip_for_streaming;

    /// Ersatz für echte Schrift: ein Rechteck je Zeichen (0,6 × Höhe breit).
    fn boxes(text: &str, h: f64) -> Vec<Polyline> {
        (0..text.chars().count())
            .map(|i| {
                let r = Rect::from_xywh(i as f64 * 0.6 * h, 0.0, 0.5 * h, h);
                Polyline::new(r.corners().to_vec(), true)
            })
            .collect()
    }

    #[test]
    fn values_are_spread_linearly() {
        let a = TestAxis { param: TestParam::Power, min: 10.0, max: 50.0, count: 5 };
        let v: Vec<f64> = (0..5).map(|i| a.value(i)).collect();
        assert_eq!(v, vec![10.0, 20.0, 30.0, 40.0, 50.0]);
        let p = TestAxis { param: TestParam::Passes, min: 1.0, max: 4.0, count: 3 };
        assert_eq!(p.value(1), 3.0);
    }

    #[test]
    fn cell_settings_follow_axes() {
        let t = TestPattern::default();
        let cs = t.cell_settings(4, 0);
        assert_eq!(cs.power_max, 50.0);
        assert_eq!(cs.speed, 2000.0);
        assert_eq!(cs.mode, LayerMode::Fill);
    }

    #[test]
    fn absolute_position_is_lower_left_corner() {
        let t = TestPattern { position: Point::new(20.0, 30.0), ..Default::default() };
        let profile = MachineProfile::default();
        let layout = t.layout(&profile, &boxes);
        let a = profile.doc_to_machine(layout.bounds.min);
        let b = profile.doc_to_machine(layout.bounds.max);
        assert!((a.x.min(b.x) - 20.0).abs() < 1e-9);
        assert!((a.y.min(b.y) - 30.0).abs() < 1e-9);
        // Beschriftung liegt außerhalb der Felder.
        let cells = layout.cells.iter().fold(Rect::EMPTY, |r, c| r.union(&c.2));
        for l in &layout.labels {
            assert!(!cells.intersects(&l.bounds()));
        }
    }

    #[test]
    fn job_contains_every_cell_with_its_settings() {
        let t = TestPattern {
            base: CutSettings::default_for(LayerMode::Line),
            columns: TestAxis { param: TestParam::Speed, min: 1000.0, max: 3000.0, count: 3 },
            rows: TestAxis { param: TestParam::Passes, min: 1.0, max: 2.0, count: 2 },
            ..Default::default()
        };
        let job = t.build(&MachineProfile::default(), &JobSettings::default(), &boxes).unwrap();
        assert!(job.warnings.is_empty(), "{:?}", job.warnings);
        // Beschriftung + 3 Felder × 1 Durchgang + 3 Felder × 2 Durchgänge
        assert_eq!(job.toolpath.blocks.len(), 1 + 3 + 6);
        let feeds: Vec<f64> = job.toolpath.blocks[1..4].iter().map(|b| b.feed).collect();
        assert_eq!(feeds, vec![1000.0, 2000.0, 3000.0]);
        // Zweite Zeile läuft rückwärts (Zickzack).
        assert_eq!(job.toolpath.blocks[4].feed, 3000.0);
        let code = strip_for_streaming(&job.gcode);
        assert!(code.iter().any(|l| l.contains("F1000")));
        assert!(code.iter().any(|l| l.contains("F3000")));
    }

    #[test]
    fn invalid_settings_are_rejected() {
        let mut t = TestPattern::default();
        t.rows.param = TestParam::Speed;
        assert!(t.validate().is_err());
        let mut t = TestPattern::default();
        t.rows.max = 150.0;
        assert!(t.validate().is_err());
        let mut t = TestPattern::default();
        t.columns.count = 0;
        assert!(t.validate().is_err());
    }
}
