//! Aus einem Dokument einen fertigen Job erzeugen: Werkzeugweg, Prüfungen, Zeitschätzung und G-Code.

use crate::estimate::{Estimate, estimate};
use crate::gcode::{CoordMode, GcodeOptions, emit, fmt_mm};
use crate::machine::{JobSettings, MachineProfile, RotaryKind, StartFrom};
use crate::optimize::{order_polylines, order_polylines_at};
use crate::toolpath::{
    Block, BlockKind, Leads, Op, RasterPreview, Toolpath, apply_ramp, cut_ops_with_leads, hatch_ops, hatch_ops_on_grid,
    line_ops, raster_ops,
};
use vl_core::{
    CutSettings, DitherMode, Document, FillGrouping, LayerMode, LineBuf, Object, ObjectId, Point, Polyline, Rect, Shape,
};

/// Toleranz beim Abflachen von Kurven für die Ausgabe (mm).
pub const OUTPUT_TOLERANCE: f64 = 0.01;
/// Größter Abstand eines Objekts vom Ursprung in mm (100 m). Weiter draußen liegt nur Unsinn aus beschädigten
/// Dateien, der in Füllungen und Perforationen Milliarden Schritte ergäbe.
const MAX_EXTENT: f64 = 100_000.0;

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

    /// Ausgabe → Dokument (ohne Drehachse).
    fn unmap(&self, q: Point) -> Point {
        self.profile.machine_to_doc(q + self.anchor_machine - self.base)
    }

    /// Ausgabe → Dokument; `None` bei Drehachse oder relativen Koordinaten (dort fehlt der feste Bezug).
    pub fn to_doc(&self, q: Point) -> Option<Point> {
        (self.rotary.is_none() && self.mode == CoordMode::Absolute).then(|| self.unmap(q))
    }
}

/// Umrandung des gebrannten Wegs (Dokument).
fn burn_bounds(toolpath: &Toolpath) -> Rect {
    let mut r = Rect::EMPTY;
    let mut prev: Option<Point> = None;
    for op in toolpath.blocks.iter().flat_map(|b| &b.ops) {
        if let Op::Move { to, power } = *op
            && power > 0.0
        {
            if let Some(p) = prev {
                r.include(p);
            }
            r.include(to);
        }
        prev = Some(op.target());
    }
    r
}

/// Der Anlauf (Overscan) beim Gravieren ist nötig, damit der Kopf beim Brennen schon seine Geschwindigkeit hat. Liegt
/// das Motiv direkt am Rand, reichte er aber über den Rahmen hinaus: der Kopf führe gegen den Endschalter und der Job
/// bräche ab. Leerfahrten werden deshalb auf die Arbeitsfläche begrenzt – nur dort, wo sie hinausreichen, und nur,
/// wenn der gebrannte Weg selbst auf der Fläche liegt. Ergebnis: um wie viel mm höchstens gekürzt wurde.
fn keep_travel_on_bed(toolpath: &mut Toolpath, mapping: &OutputMapping, profile: &MachineProfile) -> f64 {
    if mapping.mode != CoordMode::Absolute || mapping.rotary.is_some() {
        return 0.0;
    }
    let bed =
        Rect::from_points(mapping.unmap(Point::ZERO), mapping.unmap(Point::new(profile.bed_width, profile.bed_height)));
    let burns = burn_bounds(toolpath);
    if !burns.is_empty() && !bed.expanded(1e-3).contains_rect(&burns) {
        return 0.0;
    }
    let mut trimmed = 0.0_f64;
    let mut clamp = |p: &mut Point| {
        let c = Point::new(p.x.clamp(bed.min.x, bed.max.x), p.y.clamp(bed.min.y, bed.max.y));
        trimmed = trimmed.max((p.x - c.x).abs()).max((p.y - c.y).abs());
        *p = c;
    };
    for op in toolpath.blocks.iter_mut().flat_map(|b| &mut b.ops) {
        match op {
            Op::Rapid(p) => clamp(p),
            Op::Move { to, power } if *power <= 0.0 => clamp(to),
            Op::Move { .. } => {}
        }
    }
    trimmed
}

/// Warnungen, wenn der Kopf über die Arbeitsfläche hinausfahren würde: wo und wie weit, warum, und was dann passiert.
/// `mb` ist die Umrandung aller Bewegungen in Ausgabekoordinaten. Jede Zeile enthält „Arbeitsfläche“.
fn bed_warnings(
    toolpath: &Toolpath,
    mapping: &OutputMapping,
    profile: &MachineProfile,
    mb: Rect,
    check_y: bool,
) -> Vec<String> {
    let (w, h) = (profile.bed_width, profile.bed_height);
    let eps = 1e-3;
    let (mx, my) = (profile.origin.mirror_x(), profile.origin.mirror_y());
    let mut out = Vec::new();
    // Seite aus Sicht der Arbeitsfläche (vorne = beim Benutzer)
    let mut edges = Vec::new();
    if mb.min.x < -eps {
        edges.push((if mx { "rechts" } else { "links" }, -mb.min.x, mb.min.x));
    }
    if mb.max.x > w + eps {
        edges.push((if mx { "links" } else { "rechts" }, mb.max.x - w, mb.max.x));
    }
    if check_y && mb.min.y < -eps {
        edges.push((if my { "vorne" } else { "hinten" }, -mb.min.y, mb.min.y));
    }
    if check_y && mb.max.y > h + eps {
        edges.push((if my { "hinten" } else { "vorne" }, mb.max.y - h, mb.max.y));
    }
    if edges.is_empty() {
        return out;
    }
    for (edge, over, to) in edges {
        let (a, b) = (fmt_mm(over), fmt_mm(to));
        out.push(match edge {
            "links" => format!("Der Laserkopf würde links {a} mm über die Arbeitsfläche hinausfahren (bis X {b} mm)."),
            "rechts" => {
                format!("Der Laserkopf würde rechts {a} mm über die Arbeitsfläche hinausfahren (bis X {b} mm).")
            }
            "vorne" => format!("Der Laserkopf würde vorne {a} mm über die Arbeitsfläche hinausfahren (bis Y {b} mm)."),
            _ => format!("Der Laserkopf würde hinten {a} mm über die Arbeitsfläche hinausfahren (bis Y {b} mm)."),
        });
    }
    let bb = burn_bounds(toolpath);
    let bm = Rect::from_points(mapping.map(bb.min), mapping.map(bb.max));
    let motif_out = bm.min.x < -eps || bm.max.x > w + eps || (check_y && (bm.min.y < -eps || bm.max.y > h + eps));
    out.push(if motif_out {
        format!(
            "Teile des Motivs liegen außerhalb der Arbeitsfläche ({} × {} mm) – das Motiv ganz auf die Fläche verschieben.",
            fmt_mm(w),
            fmt_mm(h)
        )
    } else {
        "Das Motiv liegt auf der Arbeitsfläche, nur der Anlauf (Overscan) reicht darüber hinaus. Der Anlauf ist beim \
         Gravieren normal – der Kopf braucht ihn, um beim Brennen schon seine Geschwindigkeit zu haben –, hier ist aber \
         der Rahmen zu Ende: das Motiv etwas weiter vom Rand weg legen."
            .into()
    });
    out.push(
        "Am Rand der Arbeitsfläche stößt der Kopf an den Endschalter oder den Rahmen: Der Laser stoppt mit „Alarm 1“ und \
         der Job bricht ab. Ohne Endschalter fährt er gegen den Rahmen – Riemen können überspringen und das Gerät kann \
         beschädigt werden."
            .into(),
    );
    out
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
    /// Der Kopf würde über die Arbeitsfläche hinausfahren (Warnungen erklären, wo und warum).
    pub leaves_bed: bool,
    /// Hinweise ohne Rückfrage, z. B. ein am Rand gekürzter Anlauf.
    pub notes: Vec<String>,
    pub estimate: Estimate,
    pub gcode: LineBuf,
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
    mut toolpath: Toolpath,
    design_bounds: Rect,
    profile: &MachineProfile,
    settings: &JobSettings,
    layer_names: Vec<String>,
    title: Option<String>,
) -> Job {
    let (anchor, mapping, start) = placement(&design_bounds, profile, settings);
    let trimmed = keep_travel_on_bed(&mut toolpath, &mapping, profile);
    let mut notes = Vec::new();
    if trimmed > 0.05 {
        let t = fmt_mm(trimmed);
        notes.push(format!(
            "Der Anlauf (Overscan) würde am Rand bis {t} mm über die Arbeitsfläche hinausreichen und wird dort gekürzt, \
             damit der Kopf nicht gegen den Endschalter fährt. Die Kante kann dort etwas dunkler werden; für den vollen \
             Anlauf das Motiv {t} mm weiter vom Rand weg legen."
        ));
    }
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
        // Mit Drehachse ist y kein Weg auf der Arbeitsfläche, nur X wird geprüft.
        warnings = bed_warnings(&toolpath, &mapping, profile, mb, rotary.is_none());
    }
    let leaves_bed = !warnings.is_empty();
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
    Job {
        toolpath,
        design_bounds,
        path_bounds,
        anchor,
        start,
        mapping,
        warnings,
        leaves_bed,
        notes,
        estimate: est,
        gcode,
    }
}

/// Erzeugt den Werkzeugweg aller Ebenen in Ausgabereihenfolge (Dokumentkoordinaten).
pub fn generate(
    doc: &Document,
    objects: &[&Object],
    profile: &MachineProfile,
    settings: &JobSettings,
    start: Point,
) -> Result<Toolpath, String> {
    if let Some(o) = objects.iter().find(|o| !o.within(MAX_EXTENT)) {
        return Err(format!(
            "„{}“ reicht über 100 m hinaus – das kann kein Laser ausgeben (beschädigte Datei?).",
            o.name
        ));
    }
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
    // Von Hand gesetzte Laschen, auf die Kontur ihres Objekts gelegt
    let mut manual_tabs: Vec<Point> = Vec::new();
    for o in objs {
        match &o.shape {
            Shape::Image(img) => {
                let r =
                    vl_raster::engrave_image(img, &o.transform, cs).map_err(|e| format!("Bild „{}“: {e}", o.name))?;
                let per_pass: Vec<Vec<Op>> = if cs.dither == DitherMode::Slices3D {
                    let n = cs.passes.max(1);
                    (0..n)
                        .map(|k| raster_ops(&r.slice(k as f32 / n as f32), cs, shift, profile.fast_whitespace))
                        .collect()
                } else {
                    vec![raster_ops(&r, cs, shift, profile.fast_whitespace)]
                };
                if let Some(last) = per_pass.iter().rev().find_map(|ops| ops.last()) {
                    *cur = last.target();
                }
                raster_blocks.push((tp.rasters.len(), per_pass));
                tp.rasters.push(RasterPreview { layer, result: r });
            }
            Shape::Path(p) => {
                let flat = p.flatten(&o.transform, OUTPUT_TOLERANCE);
                manual_tabs.extend(o.tabs.iter().filter_map(|t| on_outline(&flat, o.transform.apply(*t))));
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
            if settings.remove_overlaps {
                polys = crate::optimize::remove_overlaps(polys, settings.overlap_tolerance);
            }
            let mut ordered = if settings.optimize_order {
                order_polylines_at(polys, *cur, settings.inner_first, settings.start_at_corners)
            } else {
                polys
            };
            if cs.tab_width > 0.0 && (cs.tab_count > 0 || !manual_tabs.is_empty()) {
                ordered = apply_tabs(ordered, cs, &manual_tabs);
            }
            let leads = (cs.lead_in > 0.0 || cs.lead_out > 0.0).then_some(Leads {
                lead_in: cs.lead_in,
                lead_out: cs.lead_out,
                angle_deg: cs.lead_angle,
            });
            (BlockKind::Line, vec![cut_ops_with_leads(&ordered, power, cs.overcut, cs.perforation(), leads)])
        }
        LayerMode::OffsetFill => {
            let rings = vl_core::ops::offset_fill(&polys, cs.interval, MAX_OFFSET_RINGS);
            let ordered = if settings.optimize_order { order_polylines(rings, *cur, false) } else { rings };
            (BlockKind::Fill, vec![line_ops(&ordered, power)])
        }
        LayerMode::Fill | LayerMode::Image => {
            let parts = match cs.fill_grouping {
                FillGrouping::All => Vec::new(),
                FillGrouping::Groups => units.into_iter().map(|u| u.1).collect(),
                FillGrouping::Shapes => vl_core::ops::islands(&polys),
            };
            let fill = |cs: &CutSettings| {
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
                if cs.mode == LayerMode::Fill && cs.ramp_length > 0.0 {
                    let edges = if cs.ramp_outer_only { vl_core::ops::outermost(&polys) } else { polys.clone() };
                    let min = (cs.power_min / 100.0).clamp(0.0, 1.0) as f32;
                    apply_ramp(ops, &edges, cs.ramp_length, min.min(power), power)
                } else {
                    ops
                }
            };
            // Winkelschritt: jeder Durchgang mit eigenem, weitergedrehtem Füllwinkel
            let per_pass = if cs.mode == LayerMode::Fill && cs.fill_angle_step != 0.0 && cs.passes > 1 {
                (0..cs.passes)
                    .map(|k| {
                        fill(&CutSettings { fill_angle: cs.fill_angle + k as f64 * cs.fill_angle_step, ..cs.clone() })
                    })
                    .collect()
            } else {
                vec![fill(cs)]
            };
            (BlockKind::Fill, per_pass)
        }
    };
    if let Some(last) = vector_ops.iter().rev().find_map(|ops| ops.last()) {
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
                constant_power: cs.constant_power,
                ops: ops.clone(),
                raster: Some(*idx),
                z: cs.z_for_pass(pass),
            });
        }
        let ops = &vector_ops[(pass as usize).min(vector_ops.len() - 1)];
        if !ops.is_empty() {
            tp.blocks.push(Block {
                layer,
                kind: vector_kind,
                pass,
                feed: cs.speed,
                air: cs.air_assist,
                constant_power: cs.constant_power,
                ops: ops.clone(),
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

/// Nächster Punkt auf einer geschlossenen Kontur.
fn on_outline(polys: &[Polyline], p: Point) -> Option<Point> {
    polys
        .iter()
        .filter(|q| q.closed)
        .filter_map(|q| q.closest(p).map(|c| c.0))
        .min_by(|a, b| a.distance(p).total_cmp(&b.distance(p)))
}

/// So weit darf eine von Hand gesetzte Lasche (zusätzlich zur halben Schnittbreite) neben der Kontur liegen.
const MANUAL_TAB_REACH: f64 = 0.5;

/// Unterbricht geschlossene Konturen für Laschen; die Reihenfolge der Konturen bleibt erhalten. Von Hand gesetzte
/// Laschen gehören zur nächstgelegenen Kontur und ersetzen dort die automatisch verteilten.
fn apply_tabs(polys: Vec<Polyline>, cs: &CutSettings, manual: &[Point]) -> Vec<Polyline> {
    let (count, width) = (cs.tab_count as usize, cs.tab_width);
    let bounds: Vec<Rect> = polys.iter().map(|p| p.bounds()).collect();
    let reach = MANUAL_TAB_REACH + cs.kerf.abs() / 2.0;
    let mut at: Vec<Vec<f64>> = vec![Vec::new(); polys.len()];
    for &t in manual {
        let best = polys
            .iter()
            .enumerate()
            .filter(|(i, p)| p.closed && bounds[*i].expanded(reach).contains(t))
            .filter_map(|(i, p)| p.closest(t).map(|(q, s)| (i, q.distance(t), s)))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, d, s)) = best
            && d <= reach
        {
            at[i].push(s);
        }
    }
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
        if !at[i].is_empty() {
            out.extend(vl_core::ops::split_at_tabs(p, &at[i], width));
        } else if p.closed && count > 0 && !(cs.tabs_outer_only && is_inner(i)) {
            out.extend(vl_core::ops::split_for_tabs(p, count, width));
        } else {
            out.push(p.clone());
        }
    }
    out
}

/// G-Code zum Abfahren des Job-Umrisses (Laser aus oder mit geringer Leistung laut Profil): das umgebende
/// Rechteck oder mit `hull` die konvexe Hülle aller gelaserten Strecken, die sich runden und schrägen Formen anpasst.
pub fn frame_gcode(job: &Job, profile: &MachineProfile, hull: bool) -> LineBuf {
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
            constant_power: false,
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
        let g1 = |lines: LineBuf| {
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
    fn fill_angle_turns_each_pass() {
        let mut doc = Document::new();
        doc.add_path("Fläche", 0, VectorPath::rect(0.0, 0.0, 10.0, 10.0, 0.0));
        let mut cs = vl_core::CutSettings::default_for(LayerMode::Fill);
        cs.passes = 3;
        cs.fill_angle_step = 90.0;
        cs.interval = 1.0;
        doc.layers[0].settings = cs;
        let objs: Vec<&Object> = doc.objects.iter().collect();
        let tp = generate(&doc, &objs, &MachineProfile::default(), &JobSettings::default(), Point::ZERO).unwrap();
        assert_eq!(tp.blocks.len(), 3);
        // Richtung der ersten Brennstrecke je Durchgang: waagerecht, senkrecht, waagerecht
        let horizontal = |b: &Block| {
            let mut prev = Point::ZERO;
            for op in &b.ops {
                if let Op::Move { to, power } = *op
                    && power > 0.0
                {
                    return (to.y - prev.y).abs() < 1e-9;
                }
                prev = op.target();
            }
            panic!("keine Brennstrecke");
        };
        assert_eq!(tp.blocks.iter().map(horizontal).collect::<Vec<_>>(), [true, false, true]);
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

    /// Zeitmessung: Foto 400 × 400 mm mit 0,1 mm Zeilenabstand (nur von Hand, `--release -- --ignored --nocapture`).
    #[test]
    #[ignore = "Zeitmessung"]
    fn perf_large_photo() {
        let n = 4000u32;
        let pixels: Vec<u8> = (0..n * n).map(|i| ((i % n + i / n) / 32 % 256) as u8).collect();
        for dither in [DitherMode::Jarvis, DitherMode::Grayscale] {
            let mut doc = Document::new();
            let img = ImageObject {
                image: Arc::new(RasterImage::new(n, n, pixels.clone())),
                width_mm: 400.0,
                height_mm: 400.0,
                adjust: ImageAdjust::default(),
                clip: Vec::new(),
            };
            doc.add_image("Foto", 0, img, Point::ZERO);
            let mut cs = vl_core::CutSettings::default_for(LayerMode::Image);
            cs.dither = dither;
            cs.interval = 0.1;
            doc.layers[0].settings = cs;
            let profile = MachineProfile { bed_width: 410.0, bed_height: 410.0, ..Default::default() };
            let t = std::time::Instant::now();
            let job = build_job(&doc, &[], &profile, &JobSettings::default()).unwrap();
            let lines = job.gcode.len();
            println!("{dither:?}: {:.2} s, {lines} Zeilen G-Code", t.elapsed().as_secs_f64());
        }
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
        let code = strip_for_streaming(&job.gcode).to_vec();
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
        let code = strip_for_streaming(&job.gcode).to_vec();
        assert!(code.iter().any(|l| l.contains("Y10") || l.contains("Y-10")), "{code:?}");
        assert!(!code.iter().any(|l| l.contains("Y20") || l.contains("Y-20")), "{code:?}");

        // A-Achse: keine Y-Bewegungen, Anker bei A = 0 auch absolut
        profile.rotary.axis = RotaryAxis::A;
        let abs = build_job(&doc, &[], &profile, &JobSettings::default()).unwrap();
        assert!(abs.mapping.map(abs.anchor).y.abs() < 1e-9);
        assert!(abs.warnings.iter().any(|w| w.contains("Aktuelle Position")), "{:?}", abs.warnings);
        let code = strip_for_streaming(&abs.gcode).to_vec();
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
        assert!(job.leaves_bed);
        // wo, warum, was dann passiert
        assert_eq!(job.warnings.len(), 3, "{:?}", job.warnings);
        assert!(job.warnings[0].starts_with("Der Laserkopf würde links 5 mm"), "{:?}", job.warnings);
        assert!(job.warnings[1].starts_with("Teile des Motivs"), "{:?}", job.warnings);
        assert!(job.warnings[2].contains("Alarm 1"), "{:?}", job.warnings);
    }

    /// Eine Füllung direkt am Rand: der Overscan darf nicht über die Arbeitsfläche hinaus (Endschalter).
    #[test]
    fn overscan_stays_on_bed() {
        let mut doc = Document::new();
        doc.add_path("Bierdeckel", 0, VectorPath::rect(0.4, 305.6, 94.0, 94.0, 0.0));
        doc.layers[0].settings = CutSettings { overscan: 2.5, ..CutSettings::default_for(LayerMode::Fill) };
        let profile = MachineProfile { bed_width: 400.0, bed_height: 400.0, ..Default::default() };
        let job = build_job(&doc, &[], &profile, &JobSettings::default()).unwrap();
        assert!(job.warnings.is_empty() && !job.leaves_bed, "{:?}", job.warnings);
        assert!(job.path_bounds.min.x >= 0.0 && job.path_bounds.max.y <= 400.0, "{:?}", job.path_bounds);
        // gekürzt wird nur, was hinausreicht – mit Hinweis
        assert_eq!(job.notes.len(), 1);
        assert!(job.notes[0].contains("bis 2.1 mm"), "{:?}", job.notes);
        // auf der anderen Seite bleibt der Overscan vollständig
        assert!((job.path_bounds.max.x - 96.9).abs() < 0.05, "{:?}", job.path_bounds);
        // Gebranntes außerhalb bleibt eine Warnung
        let mut doc = Document::new();
        doc.add_path("zu weit links", 0, VectorPath::rect(-1.0, 10.0, 20.0, 20.0, 0.0));
        doc.layers[0].settings = CutSettings::default_for(LayerMode::Fill);
        let job = build_job(&doc, &[], &profile, &JobSettings::default()).unwrap();
        assert!(job.leaves_bed && job.notes.is_empty());
        assert!(job.warnings.iter().any(|w| w.starts_with("Teile des Motivs")), "{:?}", job.warnings);
    }

    #[test]
    fn current_position_is_relative_to_anchor() {
        let mut doc = Document::new();
        doc.add_path("Quadrat", 0, VectorPath::rect(10.0, 10.0, 20.0, 20.0, 0.0));
        let settings =
            JobSettings { start_from: StartFrom::CurrentPosition, anchor: Anchor::Center, ..Default::default() };
        let job = build_job(&doc, &[], &MachineProfile::default(), &settings).unwrap();
        assert_eq!(job.mapping.map(Point::new(20.0, 20.0)), Point::ZERO);
        let code = strip_for_streaming(&job.gcode).to_vec();
        assert_eq!(code[1], "G91");
        assert!(job.warnings.is_empty());
        let lit = MachineProfile { frame_power: 1.0, ..Default::default() };
        let frame = strip_for_streaming(&frame_gcode(&job, &lit, false)).to_vec();
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
    fn shared_edges_are_cut_once() {
        let mut doc = Document::new();
        doc.add_path("A", 0, VectorPath::rect(10.0, 10.0, 20.0, 20.0, 0.0));
        doc.add_path("B", 0, VectorPath::rect(30.0, 10.0, 20.0, 20.0, 0.0));
        let profile = MachineProfile::default();
        let plain = build_job(&doc, &[], &profile, &JobSettings::default()).unwrap();
        assert!((cut_lengths(&plain) - 160.0).abs() < 1e-6);
        let settings = JobSettings { remove_overlaps: true, ..Default::default() };
        let once = build_job(&doc, &[], &profile, &settings).unwrap();
        assert!((cut_lengths(&once) - 140.0).abs() < 1e-6, "{}", cut_lengths(&once));
    }

    #[test]
    fn manual_tabs_replace_automatic_ones() {
        let mut doc = Document::new();
        let plate = doc.add_path("Platte", 0, VectorPath::rect(0.0, 0.0, 40.0, 40.0, 0.0));
        doc.add_path("Loch", 0, VectorPath::rect(20.0, 20.0, 10.0, 10.0, 0.0));
        let o = doc.get_mut(plate).unwrap();
        o.transform = vl_core::Affine::translate(10.0, 10.0);
        // Objektkoordinaten; die zweite liegt etwas neben der Kante
        o.tabs = vec![Point::new(20.0, 0.0), Point::new(40.3, 20.0)];
        doc.layers[0].settings.tab_count = 4;
        doc.layers[0].settings.tab_width = 1.0;
        let profile = MachineProfile::default();
        let job = build_job(&doc, &[], &profile, &JobSettings::default()).unwrap();
        // statt 4 automatischer nur die 2 gesetzten
        assert!((cut_lengths(&job) - 198.0).abs() < 1e-6, "{}", cut_lengths(&job));
        // auch ohne automatische Laschen und mit Schnittbreite
        doc.layers[0].settings.tab_count = 0;
        doc.layers[0].settings.kerf = 0.2;
        let job = build_job(&doc, &[], &profile, &JobSettings::default()).unwrap();
        assert!((cut_lengths(&job) - 198.0).abs() < 1e-6, "{}", cut_lengths(&job));
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
