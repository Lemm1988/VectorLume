//! Aus einem Dokument einen fertigen Job erzeugen: Werkzeugweg, Prüfungen, Zeitschätzung und G-Code.

use crate::estimate::{Estimate, estimate};
use crate::gcode::{CoordMode, GcodeOptions, emit, fmt_mm};
use crate::machine::{JobSettings, MachineProfile, RotaryKind, StartFrom};
use crate::optimize::order_polylines;
use crate::toolpath::{
    Block, BlockKind, Leads, Op, RasterPreview, Toolpath, apply_ramp, cut_ops_with_leads, hatch_ops, hatch_ops_on_grid,
    line_ops, raster_ops,
};
use vl_core::{
    CutSettings, DitherMode, Document, FillGrouping, LayerMode, Object, ObjectId, Point, Polyline, Rect, Shape,
};

/// Toleranz beim Abflachen von Kurven für die Ausgabe (mm).
pub const OUTPUT_TOLERANCE: f64 = 0.01;

/// Umrechnung Dokument → Ausgabekoordinaten des G-Codes.
#[derive(Clone, Debug)]
pub struct OutputMapping {
    profile: MachineProfile,
    anchor_machine: Point,
    base: Point,
    pub mode: CoordMode,
    /// Drehachse: Y wird um `pivot` mit `scale` gestreckt (Oberfläche → Achsweg).
    rotary: Option<RotaryMap>,
}

#[derive(Clone, Copy, Debug)]
struct RotaryMap {
    pivot: f64,
    scale: f64,
    /// A-Achse: der Ankerpunkt liegt bei A = 0.
    a_axis: bool,
}

impl OutputMapping {
    pub fn new(profile: &MachineProfile, settings: &JobSettings, anchor_doc: Point) -> Self {
        let anchor_machine = profile.doc_to_machine(anchor_doc);
        let (anchor_machine, base, mode) = match settings.start_from {
            StartFrom::Absolute => (Point::ZERO, Point::ZERO, CoordMode::Absolute),
            StartFrom::UserOrigin => (anchor_machine, settings.user_origin, CoordMode::Absolute),
            StartFrom::CurrentPosition => (anchor_machine, Point::ZERO, CoordMode::Relative),
        };
        let mut m = Self { profile: profile.clone(), anchor_machine, base, mode, rotary: None };
        m.rotary = profile.rotary.active_scale().map(|scale| RotaryMap {
            pivot: m.map(anchor_doc).y,
            scale,
            a_axis: profile.rotary.uses_a_axis(),
        });
        m
    }

    /// Dokument → Ausgabe. Bei einer Drehachse ist y der Achsweg der Drehachse (Y oder A).
    pub fn map(&self, p: Point) -> Point {
        let mut q = self.profile.doc_to_machine(p) - self.anchor_machine + self.base;
        if let Some(r) = self.rotary {
            let base = if r.a_axis { 0.0 } else { r.pivot };
            q.y = base + (q.y - r.pivot) * r.scale;
        }
        q
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
    let toolpath = generate(doc, &objects, profile, settings, start)?;
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
    let rotary = profile.rotary.enabled.then_some(&profile.rotary);
    if mapping.mode == CoordMode::Absolute {
        let mb = Rect::from_points(mapping.map(path_bounds.min), mapping.map(path_bounds.max));
        let eps = 1e-3;
        // Mit Drehachse ist y kein Weg auf der Arbeitsfläche, nur X wird geprüft.
        let y_out = rotary.is_none() && (mb.min.y < -eps || mb.max.y > profile.bed_height + eps);
        if mb.min.x < -eps || mb.max.x > profile.bed_width + eps || y_out {
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
    if let Some(r) = rotary {
        let u = r.circumference();
        if design_bounds.height() > u + 1e-3 {
            warnings.push(format!(
                "Drehachse: Das Motiv ist {} mm hoch, der Umfang des Werkstücks beträgt nur {} mm – es überlappt sich.",
                fmt_mm(design_bounds.height()),
                fmt_mm(u)
            ));
        }
        if settings.start_from == StartFrom::Absolute {
            warnings.push(
                "Drehachse: Mit „Absolute Koordinaten“ dreht sich das Werkstück bis zur Position des Motivs. \
                 Empfohlen ist „Aktuelle Position“."
                    .into(),
            );
        }
    }
    if !profile.air_assist_supported && toolpath.blocks.iter().any(|b| b.air) {
        warnings.push("Luftunterstützung ist eingestellt, aber im Maschinenprofil deaktiviert.".into());
    }
    if !profile.z_axis && toolpath.blocks.iter().any(|b| b.z.is_some()) {
        warnings.push("Z-Versatz ist eingestellt, aber im Maschinenprofil ist keine Z-Achse aktiviert.".into());
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
    if let Some(r) = rotary {
        let kind = match r.kind {
            RotaryKind::Roller => format!("Rollen {} mm", fmt_mm(r.roller_diameter)),
            RotaryKind::Chuck => "Futter".into(),
        };
        comments.push(format!(
            "Drehachse {}: {kind}, Werkstueck {} mm, {} je Umdrehung",
            r.axis.letter(),
            fmt_mm(r.object_diameter),
            fmt_mm(r.travel_per_rotation)
        ));
    }
    let opts = GcodeOptions {
        firmware: profile.firmware,
        s_max: profile.s_max,
        dynamic_power: profile.dynamic_power,
        air_on: profile.air_on.clone(),
        air_off: profile.air_off.clone(),
        air_supported: profile.air_assist_supported,
        z_axis: profile.z_axis,
        rotary_a: profile.rotary.uses_a_axis(),
        coord_mode: mapping.mode,
        return_to,
        comments,
        layer_names,
    };
    let gcode = emit(&toolpath, &|p| mapping.map(p), &opts);
    Job { toolpath, design_bounds, path_bounds, anchor, start, mapping, warnings, estimate: est, gcode }
}

/// Erzeugt den Werkzeugweg aller Ebenen in Ausgabereihenfolge (Dokumentkoordinaten).
pub fn generate(
    doc: &Document,
    objects: &[&Object],
    profile: &MachineProfile,
    settings: &JobSettings,
    start: Point,
) -> Result<Toolpath, String> {
    let mut tp = Toolpath::default();
    let mut cur = start;
    // Bearbeitungen (Ebene, Haupt- bzw. Unterebene); der Ausgabe-Schalter der Ebene gilt für alle
    let mut work: Vec<(usize, usize)> = Vec::new();
    for l in doc.used_layers() {
        let layer = &doc.layers[l];
        if layer.settings.output {
            work.extend(layer.operations().enumerate().filter(|(_, cs)| cs.output).map(|(i, _)| (l, i)));
        }
    }
    let has_images = |l: usize| objects.iter().any(|o| o.layer == l && o.is_image());
    if settings.engrave_first {
        // Stabile Sortierung: innerhalb gleicher Art bleibt die Reihenfolge der Ebenenliste erhalten.
        work.sort_by_key(|&(l, i)| {
            let layer = &doc.layers[l];
            let images = layer.image_operation() == i && has_images(l);
            match layer.operation(i).map(|cs| cs.mode) {
                Some(LayerMode::Line) if !images => 1,
                _ => 0,
            }
        });
    }
    for unit in job_units(objects, settings, start) {
        for &(layer, i) in &work {
            unit_blocks(&mut tp, &mut cur, doc, &unit, layer, i, profile, settings)?;
        }
    }
    Ok(tp)
}

/// Objektgruppen, die nacheinander vollständig bearbeitet werden: ohne „nach Gruppe“ alles gemeinsam, sonst je
/// Gruppe eine Einheit (Ungruppiertes zusammen); bei optimierter Reihenfolge jeweils die nächstgelegene zuerst.
fn job_units<'a>(objects: &[&'a Object], settings: &JobSettings, start: Point) -> Vec<Vec<&'a Object>> {
    if !settings.order_by_group {
        return vec![objects.to_vec()];
    }
    let mut units: Vec<(Option<vl_core::GroupId>, Vec<&Object>)> = Vec::new();
    for o in objects {
        match units.iter_mut().find(|u| u.0 == o.group) {
            Some(u) => u.1.push(o),
            None => units.push((o.group, vec![o])),
        }
    }
    let mut units: Vec<Vec<&Object>> = units.into_iter().map(|u| u.1).collect();
    if !settings.optimize_order {
        return units;
    }
    let bounds = |u: &[&Object]| u.iter().fold(Rect::EMPTY, |r, o| r.union(&o.bounds()));
    let mut ordered = Vec::with_capacity(units.len());
    let mut at = start;
    while !units.is_empty() {
        let dist = |u: &Vec<&Object>| {
            let b = bounds(u);
            let dx = (b.min.x - at.x).max(at.x - b.max.x).max(0.0);
            let dy = (b.min.y - at.y).max(at.y - b.max.y).max(0.0);
            dx * dx + dy * dy
        };
        let next = (0..units.len()).min_by(|&a, &b| dist(&units[a]).total_cmp(&dist(&units[b]))).unwrap_or(0);
        let u = units.remove(next);
        at = bounds(&u).center();
        ordered.push(u);
    }
    ordered
}

/// Eine Bearbeitung (`layer`, Unterebene `i`) für die Objekte einer Einheit.
#[allow(clippy::too_many_arguments)]
fn unit_blocks(
    tp: &mut Toolpath,
    cur: &mut Point,
    doc: &Document,
    unit: &[&Object],
    layer: usize,
    i: usize,
    profile: &MachineProfile,
    settings: &JobSettings,
) -> Result<(), String> {
    let l = &doc.layers[layer];
    let Some(cs) = l.operation(i) else { return Ok(()) };
    let (images, vectors) = (l.image_operation() == i, l.handles_vectors(i));
    let objs: Vec<&Object> =
        unit.iter().copied().filter(|o| o.layer == layer && if o.is_image() { images } else { vectors }).collect();
    // Objekte mit eigener Leistung (z. B. Materialtest) getrennt ausgeben, in der Reihenfolge des Dokuments
    let mut scales: Vec<f64> = Vec::new();
    for o in &objs {
        if !scales.contains(&o.power_scale) {
            scales.push(o.power_scale);
        }
    }
    for scale in scales {
        let part: Vec<&Object> = objs.iter().copied().filter(|o| o.power_scale == scale).collect();
        if scale == 100.0 {
            layer_blocks(tp, cur, layer, cs, &part, profile, settings)?;
        } else {
            let mut scaled = cs.clone();
            scaled.power_max *= scale / 100.0;
            scaled.power_min *= scale / 100.0;
            layer_blocks(tp, cur, layer, &scaled, &part, profile, settings)?;
        }
    }
    Ok(())
}

/// Füllt die Teile nacheinander (jeweils das nächstgelegene zuerst) auf einem gemeinsamen Zeilenraster.
fn fill_parts(parts: &[Vec<Polyline>], cs: &CutSettings, shift: f64, power: f32, from: Point) -> Vec<Op> {
    let hatch = |polys: &[Polyline], angle: f64| {
        hatch_ops_on_grid(polys, cs.interval, angle, cs.bidirectional, cs.overscan, shift, power, Some(0.0))
    };
    let mut pending: Vec<Vec<Op>> = parts
        .iter()
        .map(|p| {
            let mut ops = hatch(p, cs.fill_angle);
            if cs.crosshatch {
                ops.extend(hatch(p, cs.fill_angle + 90.0));
            }
            ops
        })
        .filter(|ops| !ops.is_empty())
        .collect();
    let mut out = Vec::new();
    let mut at = from;
    while !pending.is_empty() {
        let next = (0..pending.len())
            .min_by(|&a, &b| at.distance(pending[a][0].target()).total_cmp(&at.distance(pending[b][0].target())))
            .unwrap_or(0);
        let mut ops = pending.swap_remove(next);
        // Der erste Zug eines Teils führt mit abgeschaltetem Laser dorthin – als Eilgang
        if let Some(Op::Move { to, .. }) = ops.first().copied() {
            ops[0] = Op::Rapid(to);
        }
        at = ops.last().map(|o| o.target()).unwrap_or(at);
        out.extend(ops);
    }
    out
}

/// Bahnen einer Ebene (bzw. ihrer Objekte mit gleicher Leistung) an den Werkzeugweg anhängen.
fn layer_blocks(
    tp: &mut Toolpath,
    cur: &mut Point,
    layer: usize,
    cs: &CutSettings,
    objs: &[&Object],
    profile: &MachineProfile,
    settings: &JobSettings,
) -> Result<(), String> {
    let power = (cs.power_max / 100.0).clamp(0.0, 1.0) as f32;
    let shift = profile.scan_shift(cs.speed);

    // Je Bild: Bahnen für alle Durchgänge gemeinsam oder (3D-Schichten) je Durchgang eigene
    let mut raster_blocks: Vec<(usize, Vec<Vec<Op>>)> = Vec::new();
    let mut polys: Vec<Polyline> = Vec::new();
    // Konturen je Gruppe (ungruppierte Objekte einzeln) für „Gruppen einzeln füllen“
    let mut units: Vec<(Option<u64>, Vec<Polyline>)> = Vec::new();
    for o in objs {
        match &o.shape {
            Shape::Image(img) => {
                let r =
                    vl_raster::engrave_image(img, &o.transform, cs).map_err(|e| format!("Bild „{}“: {e}", o.name))?;
                let per_pass: Vec<Vec<Op>> = if cs.dither == DitherMode::Slices3D {
                    let n = cs.passes.max(1);
                    (0..n).map(|k| raster_ops(&r.slice(k as f32 / n as f32), cs, shift)).collect()
                } else {
                    vec![raster_ops(&r, cs, shift)]
                };
                if let Some(last) = per_pass.iter().rev().find_map(|ops| ops.last()) {
                    *cur = last.target();
                }
                raster_blocks.push((tp.rasters.len(), per_pass));
                tp.rasters.push(RasterPreview { layer, result: r });
            }
            Shape::Path(p) => {
                let flat = p.flatten(&o.transform, OUTPUT_TOLERANCE);
                if cs.mode == LayerMode::Fill && cs.fill_grouping == FillGrouping::Groups {
                    match units.iter_mut().find(|u| o.group.is_some() && u.0 == o.group) {
                        Some(u) => u.1.extend(flat.iter().cloned()),
                        None => units.push((o.group, flat.clone())),
                    }
                }
                polys.extend(flat);
            }
        }
    }

    let (vector_kind, vector_ops) = match cs.mode {
        LayerMode::Line => {
            if cs.kerf.abs() > 1e-6 {
                polys = apply_kerf(polys, cs.kerf);
            }
            let mut ordered =
                if settings.optimize_order { order_polylines(polys, *cur, settings.inner_first) } else { polys };
            if cs.tab_count > 0 && cs.tab_width > 0.0 {
                ordered = apply_tabs(ordered, cs.tab_count as usize, cs.tab_width, cs.tabs_outer_only);
            }
            let leads = (cs.lead_in > 0.0 || cs.lead_out > 0.0).then_some(Leads {
                lead_in: cs.lead_in,
                lead_out: cs.lead_out,
                angle_deg: cs.lead_angle,
            });
            (BlockKind::Line, cut_ops_with_leads(&ordered, power, cs.overcut, cs.perforation(), leads))
        }
        LayerMode::OffsetFill => {
            let rings = vl_core::ops::offset_fill(&polys, cs.interval, MAX_OFFSET_RINGS);
            let ordered = if settings.optimize_order { order_polylines(rings, *cur, false) } else { rings };
            (BlockKind::Fill, line_ops(&ordered, power))
        }
        LayerMode::Fill | LayerMode::Image => {
            let parts = match cs.fill_grouping {
                FillGrouping::All => Vec::new(),
                FillGrouping::Groups => units.into_iter().map(|u| u.1).collect(),
                FillGrouping::Shapes => vl_core::ops::islands(&polys),
            };
            let ops = if parts.is_empty() {
                let mut ops =
                    hatch_ops(&polys, cs.interval, cs.fill_angle, cs.bidirectional, cs.overscan, shift, power);
                if cs.crosshatch {
                    let angle = cs.fill_angle + 90.0;
                    ops.extend(hatch_ops(&polys, cs.interval, angle, cs.bidirectional, cs.overscan, shift, power));
                }
                ops
            } else {
                fill_parts(&parts, cs, shift, power, *cur)
            };
            let ops = if cs.mode == LayerMode::Fill && cs.ramp_length > 0.0 {
                let edges = if cs.ramp_outer_only { vl_core::ops::outermost(&polys) } else { polys.clone() };
                let min = (cs.power_min / 100.0).clamp(0.0, 1.0) as f32;
                apply_ramp(ops, &edges, cs.ramp_length, min.min(power), power)
            } else {
                ops
            };
            (BlockKind::Fill, ops)
        }
    };
    if let Some(last) = vector_ops.last() {
        *cur = last.target();
    }

    for pass in 0..cs.passes.max(1) {
        for (idx, per_pass) in &raster_blocks {
            let ops = &per_pass[(pass as usize).min(per_pass.len() - 1)];
            if ops.is_empty() {
                continue;
            }
            tp.blocks.push(Block {
                layer,
                kind: BlockKind::Raster,
                pass,
                feed: cs.speed,
                air: cs.air_assist,
                ops: ops.clone(),
                raster: Some(*idx),
                z: cs.z_for_pass(pass),
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
                z: cs.z_for_pass(pass),
            });
        }
    }
    Ok(())
}

/// Obergrenze für konzentrische Linien beim Versatz-Füllen (schützt vor riesigen Jobs bei kleinem Abstand).
const MAX_OFFSET_RINGS: usize = 5000;

/// Schnittbreitenausgleich: geschlossene Konturen um die halbe Schnittbreite versetzen (Außenkonturen nach
/// außen, Löcher nach innen), offene Pfade bleiben unverändert.
fn apply_kerf(polys: Vec<Polyline>, kerf: f64) -> Vec<Polyline> {
    let (closed, open): (Vec<Polyline>, Vec<Polyline>) = polys.into_iter().partition(|p| p.closed);
    let mut out = vl_core::ops::offset(&closed, kerf / 2.0, vl_core::ops::Corner::Sharp);
    out.extend(open);
    out
}

/// Unterbricht geschlossene Konturen für Laschen; die Reihenfolge der Konturen bleibt erhalten.
fn apply_tabs(polys: Vec<Polyline>, count: usize, width: f64, outer_only: bool) -> Vec<Polyline> {
    let bounds: Vec<Rect> = polys.iter().map(|p| p.bounds()).collect();
    let is_inner = |i: usize| {
        let p = &polys[i];
        let Some(&probe) = p.points.first() else { return false };
        (0..polys.len()).any(|j| {
            j != i
                && polys[j].closed
                && bounds[j].contains_rect(&bounds[i])
                && bounds[j] != bounds[i]
                && vl_core::path::point_in_polylines(probe, std::slice::from_ref(&polys[j]))
        })
    };
    let mut out = Vec::with_capacity(polys.len() * (count + 1));
    for (i, p) in polys.iter().enumerate() {
        if p.closed && !(outer_only && is_inner(i)) {
            out.extend(vl_core::ops::split_for_tabs(p, count, width));
        } else {
            out.push(p.clone());
        }
    }
    out
}

/// G-Code zum Abfahren des Job-Umrisses (Laser aus oder mit geringer Leistung laut Profil): das umgebende
/// Rechteck oder mit `hull` die konvexe Hülle aller gelaserten Strecken, die sich runden und schrägen Formen anpasst.
pub fn frame_gcode(job: &Job, profile: &MachineProfile, hull: bool) -> Vec<String> {
    let r = job.design_bounds;
    let mut corners = vec![r.min, Point::new(r.max.x, r.min.y), r.max, Point::new(r.min.x, r.max.y)];
    if hull {
        let outline = vl_core::ops::convex_hull(&burn_points(&job.toolpath));
        if outline.len() >= 3 {
            corners = outline;
        }
    }
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
            z: None,
        }],
        rasters: vec![],
    };
    let return_to = match job.mapping.mode {
        CoordMode::Relative => Some(Point::ZERO),
        CoordMode::Absolute => None,
    };
    let opts = GcodeOptions {
        firmware: profile.firmware,
        s_max: profile.s_max,
        dynamic_power: true,
        air_on: String::new(),
        air_off: String::new(),
        air_supported: false,
        z_axis: false,
        rotary_a: profile.rotary.uses_a_axis(),
        coord_mode: job.mapping.mode,
        return_to,
        comments: vec!["VectorLume Rahmen".into()],
        layer_names: vec!["Rahmen".into()],
    };
    emit(&tp, &|p| job.mapping.map(p), &opts)
}

/// Anfangs- und Endpunkte aller Bewegungen mit Laser an.
fn burn_points(tp: &Toolpath) -> Vec<Point> {
    let mut pts = Vec::new();
    let mut cur: Option<Point> = None;
    for op in tp.blocks.iter().flat_map(|b| &b.ops) {
        if let Op::Move { to, power } = *op
            && power > 0.0
        {
            pts.extend(cur);
            pts.push(to);
        }
        cur = Some(op.target());
    }
    pts
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
                clip: Vec::new(),
            },
            Point::new(100.0, 100.0),
        );
        doc
    }

    #[test]
    fn hull_frame_follows_round_shapes() {
        let mut doc = Document::new();
        doc.add_path("Kreis", 0, VectorPath::ellipse(Point::new(50.0, 50.0), 20.0, 20.0));
        let profile = MachineProfile::default();
        let job = build_job(&doc, &[], &profile, &JobSettings::default()).unwrap();
        // Vorschubbewegungen (G1 ist modal, Folgezeilen beginnen mit der Koordinate)
        let g1 = |lines: Vec<String>| {
            lines.iter().filter(|l| l.starts_with("G1") || l.starts_with('X') || l.starts_with('Y')).count()
        };
        assert_eq!(g1(frame_gcode(&job, &profile, false)), 4);
        let hull = frame_gcode(&job, &profile, true);
        assert!(g1(hull.clone()) > 16, "{hull:?}");
        // alle Ecken der Hülle liegen auf dem Kreis
        let pts = vl_core::ops::convex_hull(&burn_points(&job.toolpath));
        assert!(pts.iter().all(|p| (p.distance(Point::new(50.0, 50.0)) - 20.0).abs() < 0.1));
    }

    #[test]
    fn order_by_group_finishes_each_group() {
        let mut doc = Document::new();
        doc.layers[1].settings = vl_core::CutSettings::default_for(LayerMode::Fill);
        // Zwei Untersetzer: je Füllung (Ebene 1) und Kontur (Ebene 0), der zweite näher am Ursprung
        for (g, x) in [(1u64, 100.0), (2, 0.0)] {
            let fill = doc.add_path("Motiv", 1, VectorPath::rect(x + 5.0, 5.0, 10.0, 10.0, 0.0));
            let cut = doc.add_path("Kontur", 0, VectorPath::rect(x, 0.0, 20.0, 20.0, 0.0));
            for o in doc.objects.iter_mut().filter(|o| o.id == fill || o.id == cut) {
                o.group = Some(g);
            }
        }
        let objs: Vec<&Object> = doc.objects.iter().collect();
        let profile = MachineProfile::default();
        let run = |settings: &JobSettings| generate(&doc, &objs, &profile, settings, Point::ZERO).unwrap();
        let layers = |tp: &Toolpath| tp.blocks.iter().map(|b| b.layer).collect::<Vec<_>>();
        assert_eq!(layers(&run(&JobSettings::default())), [1, 0]);
        let by_group = run(&JobSettings { order_by_group: true, ..Default::default() });
        assert_eq!(layers(&by_group), [1, 0, 1, 0]);
        // Die nähere Gruppe (x = 0) zuerst, je Gruppe Gravur vor Schnitt
        let max_x = |b: &Block| b.ops.iter().map(|o| o.target().x).fold(f64::MIN, f64::max);
        assert!(max_x(&by_group.blocks[0]) < 50.0 && max_x(&by_group.blocks[1]) < 50.0);
        assert!(max_x(&by_group.blocks[2]) > 50.0 && max_x(&by_group.blocks[3]) > 50.0);
    }

    #[test]
    fn slices_engrave_darker_areas_more_often() {
        let mut doc = Document::new();
        // Dunkelheit 1,0 / 0,61 / 0,22 / 0 in vier 1-mm-Zellen
        let img = ImageObject {
            image: Arc::new(RasterImage::new(4, 1, vec![0, 100, 200, 255])),
            width_mm: 4.0,
            height_mm: 1.0,
            adjust: ImageAdjust::default(),
            clip: Vec::new(),
        };
        doc.add_image("Relief", 0, img, Point::new(10.0, 10.0));
        let mut cs = vl_core::CutSettings::default_for(LayerMode::Image);
        cs.dither = DitherMode::Slices3D;
        cs.interval = 1.0;
        cs.passes = 4;
        cs.overscan = 0.0;
        doc.layers[0].settings = cs;
        let objs: Vec<&Object> = doc.objects.iter().collect();
        let tp = generate(&doc, &objs, &MachineProfile::default(), &JobSettings::default(), Point::ZERO).unwrap();
        let burned: Vec<f64> = tp
            .blocks
            .iter()
            .map(|b| {
                let mut cur = Point::ZERO;
                let mut len = 0.0;
                for op in &b.ops {
                    if let Op::Move { to, power } = *op
                        && power > 0.0
                    {
                        len += cur.distance(to);
                    }
                    cur = op.target();
                }
                len
            })
            .collect();
        assert_eq!(tp.blocks.iter().map(|b| b.pass).collect::<Vec<_>>(), [0, 1, 2, 3]);
        let rounded: Vec<f64> = burned.iter().map(|l| l.round()).collect();
        assert_eq!(rounded, [3.0, 2.0, 2.0, 1.0], "{burned:?}");
    }

    #[test]
    fn sublayers_add_operations() {
        let mut doc = Document::new();
        doc.add_path("Quadrat", 0, VectorPath::rect(0.0, 0.0, 10.0, 10.0, 0.0));
        doc.add_path("Linie", 1, VectorPath::line(Point::new(0.0, 20.0), Point::new(10.0, 20.0)));
        doc.layers[0].settings = vl_core::CutSettings::default_for(LayerMode::Fill);
        doc.layers[0].sub.push(vl_core::CutSettings::default_for(LayerMode::Line));
        let profile = MachineProfile::default();
        let kinds = |doc: &Document, settings: &JobSettings| -> Vec<(usize, BlockKind)> {
            let objs: Vec<&Object> = doc.objects.iter().collect();
            let tp = generate(doc, &objs, &profile, settings, Point::ZERO).unwrap();
            tp.blocks.iter().map(|b| (b.layer, b.kind)).collect()
        };
        let in_order = JobSettings { engrave_first: false, ..JobSettings::default() };
        assert_eq!(kinds(&doc, &in_order), [(0, BlockKind::Fill), (0, BlockKind::Line), (1, BlockKind::Line)]);
        // Gravieren vor Schneiden gilt auch für Unterebenen
        let first = JobSettings { engrave_first: true, ..JobSettings::default() };
        assert_eq!(kinds(&doc, &first)[0], (0, BlockKind::Fill));
        // eigener Ausgabe-Schalter; Ebene aus = alle Bearbeitungen aus
        doc.layers[0].sub[0].output = false;
        assert_eq!(kinds(&doc, &in_order), [(0, BlockKind::Fill), (1, BlockKind::Line)]);
        doc.layers[0].sub[0].output = true;
        doc.layers[0].settings.output = false;
        assert_eq!(kinds(&doc, &in_order), [(1, BlockKind::Line)]);
        // Unterebene im Modus Bild bearbeitet nur Bilder
        doc.layers[0].settings.output = true;
        doc.layers[0].sub[0] = vl_core::CutSettings::default_for(LayerMode::Image);
        assert_eq!(kinds(&doc, &in_order), [(0, BlockKind::Fill), (1, BlockKind::Line)]);
        assert_eq!(doc.layers[0].mode_labels(), ["Füllen", "Bild"]);
    }

    #[test]
    fn fill_shapes_individually_saves_travel() {
        let mut doc = Document::new();
        // zwei weit auseinanderliegende Quadrate, das rechte mit Loch
        doc.add_path("links", 0, VectorPath::rect(0.0, 0.0, 10.0, 10.0, 0.0));
        let mut right = VectorPath::rect(200.0, 0.0, 10.0, 10.0, 0.0);
        right.append(VectorPath::rect(203.0, 3.0, 4.0, 4.0, 0.0));
        doc.add_path("rechts", 0, right);
        doc.layers[0].settings = vl_core::CutSettings::default_for(LayerMode::Fill);
        doc.layers[0].settings.interval = 0.5;
        let profile = MachineProfile::default();
        let run = |doc: &Document| {
            let objs: Vec<&Object> = doc.objects.iter().collect();
            let tp = generate(doc, &objs, &profile, &JobSettings::default(), Point::ZERO).unwrap();
            let (mut burn, mut travel, mut at) = (0.0, 0.0, Point::ZERO);
            for op in tp.blocks.iter().flat_map(|b| &b.ops) {
                let d = at.distance(op.target());
                match op {
                    Op::Move { power, .. } if *power > 0.0 => burn += d,
                    _ => travel += d,
                }
                at = op.target();
            }
            (burn, travel)
        };
        let (burn_all, travel_all) = run(&doc);
        for g in [FillGrouping::Shapes, FillGrouping::Groups] {
            doc.layers[0].settings.fill_grouping = g;
            let (burn, travel) = run(&doc);
            assert!((burn - burn_all).abs() < 1e-6, "{g:?}: gleiche Fläche {burn} {burn_all}");
            assert!(travel * 5.0 < travel_all, "{g:?}: {travel} gegen {travel_all}");
        }
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
    fn rotary_maps_surface_to_axis_travel() {
        use crate::machine::{Rotary, RotaryAxis, RotaryKind};
        let mut doc = Document::new();
        doc.add_path("Quadrat", 0, VectorPath::rect(10.0, 10.0, 20.0, 20.0, 0.0));
        // Futter, Umfang 100 mm, 50 mm Achsweg je Umdrehung → Faktor 0,5
        let rotary = Rotary {
            enabled: true,
            kind: RotaryKind::Chuck,
            travel_per_rotation: 50.0,
            object_diameter: 100.0 / std::f64::consts::PI,
            ..Rotary::default()
        };
        let mut profile = MachineProfile { rotary, ..Default::default() };
        let settings = JobSettings { start_from: StartFrom::CurrentPosition, ..Default::default() };
        let job = build_job(&doc, &[], &profile, &settings).unwrap();
        assert!(job.warnings.is_empty(), "{:?}", job.warnings);
        let (a, b) = (job.mapping.map(Point::new(10.0, 10.0)), job.mapping.map(Point::new(30.0, 30.0)));
        assert!(((a.y - b.y).abs() - 10.0).abs() < 1e-9 && ((a.x - b.x).abs() - 20.0).abs() < 1e-9);
        assert!(job.mapping.map(job.anchor).distance(Point::ZERO) < 1e-9);
        let code = strip_for_streaming(&job.gcode);
        assert!(code.iter().any(|l| l.contains("Y10") || l.contains("Y-10")), "{code:?}");
        assert!(!code.iter().any(|l| l.contains("Y20") || l.contains("Y-20")), "{code:?}");

        // A-Achse: keine Y-Bewegungen, Anker bei A = 0 auch absolut
        profile.rotary.axis = RotaryAxis::A;
        let abs = build_job(&doc, &[], &profile, &JobSettings::default()).unwrap();
        assert!(abs.mapping.map(abs.anchor).y.abs() < 1e-9);
        assert!(abs.warnings.iter().any(|w| w.contains("Aktuelle Position")), "{:?}", abs.warnings);
        let code = strip_for_streaming(&abs.gcode);
        assert!(code.iter().any(|l| l.contains(" A") || l.starts_with('A')), "{code:?}");
        assert!(!code.iter().any(|l| l.contains('Y')), "{code:?}");

        // Motiv höher als der Umfang
        profile.rotary.object_diameter = 5.0;
        let small = build_job(&doc, &[], &profile, &settings).unwrap();
        assert!(small.warnings.iter().any(|w| w.contains("Umfang")), "{:?}", small.warnings);
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
        let frame = strip_for_streaming(&frame_gcode(&job, &lit, false));
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

    fn cut_lengths(job: &Job) -> f64 {
        let mut cur = job.start;
        let mut len = 0.0;
        for b in &job.toolpath.blocks {
            for op in &b.ops {
                if let Op::Move { to, power } = *op
                    && power > 0.0
                {
                    len += cur.distance(to);
                }
                cur = op.target();
            }
        }
        len
    }

    #[test]
    fn kerf_and_tabs_change_the_cut() {
        let mut doc = Document::new();
        doc.add_path("Platte", 0, VectorPath::rect(10.0, 10.0, 40.0, 40.0, 0.0));
        doc.add_path("Loch", 0, VectorPath::rect(20.0, 20.0, 10.0, 10.0, 0.0));
        let profile = MachineProfile::default();
        let plain = build_job(&doc, &[], &profile, &JobSettings::default()).unwrap();
        assert!((cut_lengths(&plain) - 200.0).abs() < 1e-6);

        doc.layers[0].settings.kerf = 0.2;
        let kerf = build_job(&doc, &[], &profile, &JobSettings::default()).unwrap();
        // außen +0,1 je Seite (Umfang +0,8), Loch −0,1 je Seite (Umfang −0,8)
        assert!((cut_lengths(&kerf) - 200.0).abs() < 1e-6);
        assert!((kerf.design_bounds.width() - 40.0).abs() < 1e-9, "Entwurfsumriss bleibt gleich");
        assert!((kerf.path_bounds.width() - 40.2).abs() < 1e-6, "{:?}", kerf.path_bounds);

        doc.layers[0].settings.kerf = 0.0;
        doc.layers[0].settings.tab_count = 4;
        doc.layers[0].settings.tab_width = 1.0;
        let tabs = build_job(&doc, &[], &profile, &JobSettings::default()).unwrap();
        // nur die Außenkontur bekommt Laschen: 4 × 1 mm weniger
        assert!((cut_lengths(&tabs) - 196.0).abs() < 1e-6, "{}", cut_lengths(&tabs));
    }

    #[test]
    fn power_scale_per_object() {
        let mut doc = Document::new();
        doc.layers[0].settings.power_max = 80.0;
        doc.add_path("voll", 0, VectorPath::rect(0.0, 0.0, 10.0, 10.0, 0.0));
        let half = doc.add_path("halb", 0, VectorPath::rect(20.0, 0.0, 10.0, 10.0, 0.0));
        doc.get_mut(half).unwrap().power_scale = 50.0;
        let objs: Vec<&Object> = doc.objects.iter().collect();
        let tp = generate(&doc, &objs, &MachineProfile::default(), &JobSettings::default(), Point::ZERO).unwrap();
        let max_power = |b: &Block| {
            b.ops.iter().map(|o| if let Op::Move { power, .. } = o { *power } else { 0.0 }).fold(0.0, f32::max)
        };
        let powers: Vec<f32> = tp.blocks.iter().map(max_power).collect();
        assert_eq!(powers, vec![0.8, 0.4]);
    }

    #[test]
    fn offset_fill_engraves_rings() {
        let mut doc = Document::new();
        doc.add_path("Quadrat", 0, VectorPath::rect(10.0, 10.0, 10.0, 10.0, 0.0));
        doc.layers[0].settings = vl_core::CutSettings::default_for(LayerMode::OffsetFill);
        doc.layers[0].settings.interval = 1.0;
        let job = build_job(&doc, &[], &MachineProfile::default(), &JobSettings::default()).unwrap();
        assert_eq!(job.toolpath.blocks[0].kind, BlockKind::Fill);
        // Ringe mit 9, 7, 5, 3, 1 mm Kantenlänge
        assert!((cut_lengths(&job) - 4.0 * 25.0).abs() < 1e-3, "{}", cut_lengths(&job));
    }
}
