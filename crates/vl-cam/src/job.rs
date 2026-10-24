//! Aus einem Dokument einen fertigen Job erzeugen: Werkzeugweg, Prüfungen, Zeitschätzung und G-Code.

use crate::estimate::{Estimate, estimate};
use crate::gcode::{CoordMode, GcodeOptions, emit, fmt_mm};
use crate::machine::{JobSettings, MachineProfile, StartFrom};
use crate::optimize::order_polylines;
use crate::toolpath::{Block, BlockKind, Op, RasterPreview, Toolpath, hatch_ops, line_ops, raster_ops};
use vl_core::{Document, LayerMode, Object, ObjectId, Point, Polyline, Rect, Shape};

/// Toleranz beim Abflachen von Kurven für die Ausgabe (mm).
pub const OUTPUT_TOLERANCE: f64 = 0.01;

/// Umrechnung Dokument → Ausgabekoordinaten des G-Codes.
#[derive(Clone, Debug)]
pub struct OutputMapping {
    profile: MachineProfile,
    anchor_machine: Point,
    base: Point,
    pub mode: CoordMode,
}

impl OutputMapping {
    pub fn new(profile: &MachineProfile, settings: &JobSettings, anchor_doc: Point) -> Self {
        let anchor_machine = profile.doc_to_machine(anchor_doc);
        let (anchor_machine, base, mode) = match settings.start_from {
            StartFrom::Absolute => (Point::ZERO, Point::ZERO, CoordMode::Absolute),
            StartFrom::UserOrigin => (anchor_machine, settings.user_origin, CoordMode::Absolute),
            StartFrom::CurrentPosition => (anchor_machine, Point::ZERO, CoordMode::Relative),
        };
        Self { profile: profile.clone(), anchor_machine, base, mode }
    }

    pub fn map(&self, p: Point) -> Point {
        self.profile.doc_to_machine(p) - self.anchor_machine + self.base
    }
}

#[derive(Clone, Debug)]
pub struct Job {
    pub toolpath: Toolpath,
    /// Umrandung der ausgegebenen Objekte (Dokument).
    pub design_bounds: Rect,
    /// Umrandung aller Bewegungen inkl. Overscan (Dokument).
    pub path_bounds: Rect,
    pub anchor: Point,
    pub start: Point,
    pub mapping: OutputMapping,
    pub warnings: Vec<String>,
    pub estimate: Estimate,
    pub gcode: Vec<String>,
}

/// Objekte, die ausgegeben werden (Ebene aktiv, ggf. nur Auswahl).
pub fn output_objects<'a>(doc: &'a Document, selection: &[ObjectId], settings: &JobSettings) -> Vec<&'a Object> {
    doc.objects
        .iter()
        .filter(|o| doc.layers[o.layer].settings.output)
        .filter(|o| !settings.selected_only || selection.contains(&o.id))
        .collect()
}

pub fn build_job(
    doc: &Document,
    selection: &[ObjectId],
    profile: &MachineProfile,
    settings: &JobSettings,
) -> Result<Job, String> {
    if settings.selected_only && selection.is_empty() {
        return Err("„Nur Auswahl“ ist aktiv, aber es ist nichts ausgewählt.".into());
    }
    let objects = output_objects(doc, selection, settings);
    if objects.is_empty() {
        return Err("Es gibt nichts auszugeben (keine Objekte auf aktiven Ebenen).".into());
    }
    let design_bounds = objects.iter().fold(Rect::EMPTY, |r, o| r.union(&o.bounds()));
    let (_, _, start) = placement(&design_bounds, profile, settings);
    let toolpath = generate(doc, &objects, settings, start)?;
    if toolpath.is_empty() {
        return Err(
            "Die ausgewählten Objekte erzeugen keinen Werkzeugweg (z. B. offene Pfade auf einer Füllebene).".into()
        );
    }
    let layer_names = doc.layers.iter().map(|l| l.name.clone()).collect();
    Ok(finish_job(toolpath, design_bounds, profile, settings, layer_names, None))
}

/// Ankerpunkt, Ausgabeabbildung und Startpunkt (Dokument) für einen Job mit diesen Umrissen.
pub fn placement(
    design_bounds: &Rect,
    profile: &MachineProfile,
    settings: &JobSettings,
) -> (Point, OutputMapping, Point) {
    let anchor = settings.anchor.point_in(design_bounds);
    let mapping = OutputMapping::new(profile, settings, anchor);
    let start = match settings.start_from {
        StartFrom::Absolute => profile.machine_to_doc(Point::ZERO),
        _ => anchor,
    };
    (anchor, mapping, start)
}

/// Bereichsprüfung, Zeitschätzung und G-Code für einen fertigen Werkzeugweg.
pub fn finish_job(
    toolpath: Toolpath,
    design_bounds: Rect,
    profile: &MachineProfile,
    settings: &JobSettings,
    layer_names: Vec<String>,
    title: Option<String>,
) -> Job {
    let (anchor, mapping, start) = placement(&design_bounds, profile, settings);
    let mut path_bounds = Rect::EMPTY;
    for b in &toolpath.blocks {
        for op in &b.ops {
            path_bounds.include(op.target());
        }
    }
    let mut warnings = Vec::new();
    if mapping.mode == CoordMode::Absolute {
        let mb = Rect::from_points(mapping.map(path_bounds.min), mapping.map(path_bounds.max));
        let eps = 1e-3;
        if mb.min.x < -eps
            || mb.min.y < -eps
            || mb.max.x > profile.bed_width + eps
            || mb.max.y > profile.bed_height + eps
        {
            warnings.push(format!(
                "Der Job (inkl. Overscan) liegt außerhalb der Arbeitsfläche: X {} … {}, Y {} … {} mm (Fläche {} × {} mm).",
                fmt_mm(mb.min.x),
                fmt_mm(mb.max.x),
                fmt_mm(mb.min.y),
                fmt_mm(mb.max.y),
                fmt_mm(profile.bed_width),
                fmt_mm(profile.bed_height)
            ));
        }
    }
    if !profile.air_assist_supported && toolpath.blocks.iter().any(|b| b.air) {
        warnings.push("Luftunterstützung ist eingestellt, aber im Maschinenprofil deaktiviert.".into());
    }

    let est = estimate(&toolpath, start, profile.rapid_rate, profile.acceleration);
    let return_to = profile.return_to_origin.then_some(match settings.start_from {
        StartFrom::UserOrigin => settings.user_origin,
        StartFrom::Absolute | StartFrom::CurrentPosition => Point::ZERO,
    });
    let mut comments = vec![format!("VectorLume {}", env!("CARGO_PKG_VERSION"))];
    comments.extend(title);
    comments.extend([
        format!("Maschine: {}", profile.name),
        format!("Start: {}", settings.start_from.label()),
        format!("Geschaetzte Dauer: {}", est.format_duration()),
    ]);
    let opts = GcodeOptions {
        s_max: profile.s_max,
        dynamic_power: profile.dynamic_power,
        air_on: profile.air_on.clone(),
        air_off: profile.air_off.clone(),
        air_supported: profile.air_assist_supported,
        coord_mode: mapping.mode,
        return_to,
        comments,
        layer_names,
    };
    let gcode = emit(&toolpath, &|p| mapping.map(p), &opts);
    Job { toolpath, design_bounds, path_bounds, anchor, start, mapping, warnings, estimate: est, gcode }
}

/// Erzeugt den Werkzeugweg aller Ebenen in Ausgabereihenfolge (Dokumentkoordinaten).
pub fn generate(doc: &Document, objects: &[&Object], settings: &JobSettings, start: Point) -> Result<Toolpath, String> {
    let mut tp = Toolpath::default();
    let mut cur = start;
    let mut layers = doc.used_layers();
    if settings.engrave_first {
        // Stabile Sortierung: innerhalb gleicher Art bleibt die Reihenfolge der Ebenenliste erhalten.
        layers.sort_by_key(|&l| {
            let has_images = objects.iter().any(|o| o.layer == l && o.is_image());
            match doc.layers[l].settings.mode {
                LayerMode::Line if !has_images => 1,
                _ => 0,
            }
        });
    }
    for layer in layers {
        let cs = &doc.layers[layer].settings;
        if !cs.output {
            continue;
        }
        let objs: Vec<&Object> = objects.iter().copied().filter(|o| o.layer == layer).collect();
        if objs.is_empty() {
            continue;
        }
        let power = (cs.power_max / 100.0).clamp(0.0, 1.0) as f32;

        let mut raster_blocks: Vec<(usize, Vec<Op>)> = Vec::new();
        let mut polys: Vec<Polyline> = Vec::new();
        for o in &objs {
            match &o.shape {
                Shape::Image(img) => {
                    let r = vl_raster::engrave_image(img, &o.transform, cs.interval, cs.dither, cs.negative)
                        .map_err(|e| format!("Bild „{}“: {e}", o.name))?;
                    let ops = raster_ops(&r, cs);
                    if let Some(last) = ops.last() {
                        cur = last.target();
                    }
                    raster_blocks.push((tp.rasters.len(), ops));
                    tp.rasters.push(RasterPreview { layer, result: r });
                }
                Shape::Path(p) => polys.extend(p.flatten(&o.transform, OUTPUT_TOLERANCE)),
            }
        }

        let (vector_kind, vector_ops) = match cs.mode {
            LayerMode::Line => {
                let ordered =
                    if settings.optimize_order { order_polylines(polys, cur, settings.inner_first) } else { polys };
                (BlockKind::Line, line_ops(&ordered, power))
            }
            LayerMode::Fill | LayerMode::Image => {
                let mut ops = hatch_ops(&polys, cs.interval, cs.fill_angle, cs.bidirectional, cs.overscan, power);
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
        if let Some(last) = vector_ops.last() {
            cur = last.target();
        }

        for pass in 0..cs.passes.max(1) {
            for (idx, ops) in &raster_blocks {
                tp.blocks.push(Block {
                    layer,
                    kind: BlockKind::Raster,
                    pass,
                    feed: cs.speed,
                    air: cs.air_assist,
                    ops: ops.clone(),
                    raster: Some(*idx),
                });
            }
            if !vector_ops.is_empty() {
                tp.blocks.push(Block {
                    layer,
                    kind: vector_kind,
                    pass,
                    feed: cs.speed,
                    air: cs.air_assist,
                    ops: vector_ops.clone(),
                    raster: None,
                });
            }
        }
    }
    Ok(tp)
}

/// G-Code zum Abfahren des Job-Umrisses (Laser aus oder mit geringer Leistung laut Profil).
pub fn frame_gcode(job: &Job, profile: &MachineProfile) -> Vec<String> {
    let r = job.design_bounds;
    let corners = [r.min, Point::new(r.max.x, r.min.y), r.max, Point::new(r.min.x, r.max.y)];
    let power = (profile.frame_power / 100.0).clamp(0.0, 0.1) as f32;
    // Zur ersten Ecke im Eilgang (Laser aus), sonst entstünde bei Rahmen-Leistung > 0 eine Linie vom Startpunkt.
    let mut ops = vec![Op::Rapid(corners[0])];
    ops.extend(corners[1..].iter().map(|&c| Op::Move { to: c, power }));
    ops.push(Op::Move { to: corners[0], power });
    let tp = Toolpath {
        blocks: vec![Block {
            layer: 0,
            kind: BlockKind::Line,
            pass: 0,
            feed: profile.frame_speed,
            air: false,
            ops,
            raster: None,
        }],
        rasters: vec![],
    };
    let return_to = match job.mapping.mode {
        CoordMode::Relative => Some(Point::ZERO),
        CoordMode::Absolute => None,
    };
    let opts = GcodeOptions {
        s_max: profile.s_max,
        dynamic_power: true,
        air_on: String::new(),
        air_off: String::new(),
        air_supported: false,
        coord_mode: job.mapping.mode,
        return_to,
        comments: vec!["VectorLume Rahmen".into()],
        layer_names: vec!["Rahmen".into()],
    };
    emit(&tp, &|p| job.mapping.map(p), &opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gcode::strip_for_streaming;
    use crate::machine::Anchor;
    use std::sync::Arc;
    use vl_core::{ImageAdjust, ImageObject, RasterImage, VectorPath};

    fn sample_doc() -> Document {
        let mut doc = Document::new();
        doc.add_path("Quadrat", 0, VectorPath::rect(10.0, 10.0, 20.0, 20.0, 0.0));
        let fill = doc.add_path("Kreis", 1, VectorPath::ellipse(Point::new(60.0, 20.0), 10.0, 10.0));
        let _ = fill;
        doc.layers[1].settings = vl_core::CutSettings::default_for(LayerMode::Fill);
        doc.add_image(
            "Bild",
            2,
            ImageObject {
                image: Arc::new(RasterImage::new(2, 2, vec![0, 255, 255, 0])),
                width_mm: 4.0,
                height_mm: 4.0,
                adjust: ImageAdjust::default(),
            },
            Point::new(100.0, 100.0),
        );
        doc
    }

    #[test]
    fn full_job_absolute() {
        let doc = sample_doc();
        let profile = MachineProfile::default();
        let job = build_job(&doc, &[], &profile, &JobSettings::default()).unwrap();
        assert!(job.warnings.is_empty(), "{:?}", job.warnings);
        let kinds: Vec<BlockKind> = job.toolpath.blocks.iter().map(|b| b.kind).collect();
        // Gravieren vor Schneiden: Linienebene kommt zuletzt.
        assert_eq!(kinds, vec![BlockKind::Fill, BlockKind::Raster, BlockKind::Line]);
        let in_list_order = JobSettings { engrave_first: false, ..Default::default() };
        let job2 = build_job(&doc, &[], &profile, &in_list_order).unwrap();
        let kinds2: Vec<BlockKind> = job2.toolpath.blocks.iter().map(|b| b.kind).collect();
        assert_eq!(kinds2, vec![BlockKind::Line, BlockKind::Fill, BlockKind::Raster]);
        let code = strip_for_streaming(&job.gcode);
        assert_eq!(&code[..3], &["G21", "G90", "M4 S0"]);
        assert!(code.iter().any(|l| l.contains("S300")));
        assert!(job.estimate.seconds > 0.0);
        // Quadrat oben links im Dokument (y=10) liegt bei Nullpunkt vorne links bei Y = 400-10 = 390.
        assert!(code.iter().any(|l| l.contains("Y390")));
    }

    #[test]
    fn out_of_bed_warns() {
        let mut doc = Document::new();
        doc.add_path("zu groß", 0, VectorPath::rect(-5.0, 10.0, 20.0, 20.0, 0.0));
        let job = build_job(&doc, &[], &MachineProfile::default(), &JobSettings::default()).unwrap();
        assert_eq!(job.warnings.len(), 1);
    }

    #[test]
    fn current_position_is_relative_to_anchor() {
        let mut doc = Document::new();
        doc.add_path("Quadrat", 0, VectorPath::rect(10.0, 10.0, 20.0, 20.0, 0.0));
        let settings =
            JobSettings { start_from: StartFrom::CurrentPosition, anchor: Anchor::Center, ..Default::default() };
        let job = build_job(&doc, &[], &MachineProfile::default(), &settings).unwrap();
        assert_eq!(job.mapping.map(Point::new(20.0, 20.0)), Point::ZERO);
        let code = strip_for_streaming(&job.gcode);
        assert_eq!(code[1], "G91");
        assert!(job.warnings.is_empty());
        let lit = MachineProfile { frame_power: 1.0, ..Default::default() };
        let frame = strip_for_streaming(&frame_gcode(&job, &lit));
        assert_eq!(frame[1], "G91");
        assert_eq!(frame.last().unwrap(), "G90");
        // Erste Bewegung ist ein Eilgang, erst danach wird mit geringer Leistung abgefahren.
        assert!(frame[3].starts_with("G0 "), "{frame:?}");
        assert!(frame[4].starts_with("G1 ") && frame[4].ends_with("S10"), "{frame:?}");
    }

    #[test]
    fn selected_only_requires_selection() {
        let doc = sample_doc();
        let settings = JobSettings { selected_only: true, ..Default::default() };
        assert!(build_job(&doc, &[], &MachineProfile::default(), &settings).is_err());
        let id = doc.objects[0].id;
        let job = build_job(&doc, &[id], &MachineProfile::default(), &settings).unwrap();
        assert_eq!(job.toolpath.blocks.len(), 1);
    }
}
