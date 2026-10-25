//! Formoperationen auf der Auswahl: Boolesche Operationen, Verschweißen, Versatz und Reihen-Kopien.

use crate::app::{ToastKind, VectorLumeApp};
use crate::i18n::{tr, trs};
use crate::theme;
use egui::{Align2, Button, Color32, DragValue, RichText, vec2};
use egui_phosphor::regular as icon;
use vl_cam::OriginCorner;
use vl_core::corners::CornerStyle;
use vl_core::nest::NestOptions;
use vl_core::ops::{BoolOp, Corner};
use vl_core::{Affine, ImageAdjust, ImageObject, Object, ObjectId, Point, Polyline, Shape, VectorPath};
use vl_raster::RasterError;
use vl_raster::render::{RenderOptions, render_bounds, render_objects, render_size};

/// Toleranz beim Abflachen von Kurven für Formoperationen (mm).
const TOLERANCE: f64 = 0.01;

/// Richtung des Versatzes; beidseitig ergibt je eine Kontur außen und innen.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OffsetDirection {
    Outward,
    Inward,
    Both,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ArrayMode {
    Grid,
    Circle,
    /// Kopien entlang eines Pfades
    Path,
}

pub struct ShapeDialogs {
    pub offset_open: bool,
    pub offset_distance: f64,
    pub offset_direction: OffsetDirection,
    pub offset_corner: Corner,
    pub offset_keep_original: bool,
    pub array_open: bool,
    pub array_mode: ArrayMode,
    pub columns: u32,
    pub rows: u32,
    pub gap_x: f64,
    pub gap_y: f64,
    pub circle_count: u32,
    pub circle_radius: f64,
    pub circle_angle: f64,
    pub circle_rotate: bool,
    pub path_count: u32,
    pub path_rotate: bool,
    /// Vorlage nach dem Verteilen entfernen.
    pub path_remove_template: bool,
    /// Pfad = zuerst statt zuletzt gewählte Einheit.
    pub path_swap: bool,
    /// Höchster Abstand zwischen Enden, die beim Bereinigen verbunden werden (mm).
    pub join_tolerance: f64,
    pub bitmap_open: bool,
    pub bitmap: RenderOptions,
    pub bitmap_keep_original: bool,
    pub simplify_open: bool,
    /// Erlaubte Abweichung beim Vereinfachen (mm).
    pub simplify_tolerance: f64,
    /// 0 = eckig, 1 = rund (Kurven).
    pub simplify_smoothing: f64,
    pub nest_open: bool,
    pub nest: NestOptions,
    /// Die größte ausgewählte geschlossene Form ist die Platte (z. B. ein Reststück) statt der Arbeitsfläche.
    pub nest_on_shape: bool,
    pub nest_job: Option<NestJob>,
    pub slots_open: bool,
    /// Bisherige Schlitzbreite (mm), neue Breite und Toleranz.
    pub slots_from: f64,
    pub slots_to: f64,
    pub slots_tolerance: f64,
    pub corners_open: bool,
    pub corner_style: CornerStyle,
    /// Radius bzw. Fasenlänge (mm) und Mindestknick (Grad), ab dem ein Knoten als Ecke gilt.
    pub corner_size: f64,
    pub corner_min_angle: f64,
    /// Abstand, der beim Andocken frei bleibt (mm).
    pub dock_gap: f64,
}

/// Verschachteln im Hintergrund: Einheiten, Projektstand beim Start und das Ergebnis.
pub struct NestJob {
    units: Vec<Vec<ObjectId>>,
    revision: u64,
    result: std::sync::mpsc::Receiver<Vec<Option<Affine>>>,
}

impl Default for ShapeDialogs {
    fn default() -> Self {
        Self {
            offset_open: false,
            offset_distance: 2.0,
            offset_direction: OffsetDirection::Outward,
            offset_corner: Corner::Round,
            offset_keep_original: true,
            array_open: false,
            array_mode: ArrayMode::Grid,
            columns: 3,
            rows: 2,
            gap_x: 5.0,
            gap_y: 5.0,
            circle_count: 6,
            circle_radius: 30.0,
            circle_angle: 360.0,
            circle_rotate: true,
            path_count: 10,
            path_rotate: true,
            path_remove_template: true,
            join_tolerance: 0.1,
            bitmap_open: false,
            bitmap: RenderOptions::default(),
            bitmap_keep_original: false,
            simplify_open: false,
            simplify_tolerance: 0.05,
            simplify_smoothing: 0.0,
            nest_open: false,
            nest: NestOptions::default(),
            nest_on_shape: false,
            nest_job: None,
            slots_open: false,
            slots_from: 3.0,
            slots_to: 3.0,
            slots_tolerance: 0.1,
            corners_open: false,
            corner_style: CornerStyle::Round,
            corner_size: 2.0,
            corner_min_angle: 15.0,
            dock_gap: 0.0,
            path_swap: false,
        }
    }
}

fn polylines_to_path(polys: &[Polyline]) -> VectorPath {
    let mut path = VectorPath::default();
    for p in polys {
        path.append(VectorPath::polygon(&p.points, p.closed));
    }
    path
}

impl VectorLumeApp {
    /// Auswahl als Einheiten (Gruppen gemeinsam), die zuerst gewählte Einheit vorne.
    fn ordered_units(&self) -> Vec<Vec<ObjectId>> {
        let mut units = self.selection_units();
        // Eine einzelne Gruppe (z. B. ein Import) wird in ihre Teile zerlegt.
        if units.len() == 1 && units[0].len() > 1 {
            units = units[0].iter().map(|&id| vec![id]).collect();
        }
        if let Some(first) = self.selection.first()
            && let Some(i) = units.iter().position(|u| u.contains(first))
        {
            let u = units.remove(i);
            units.insert(0, u);
        }
        units
    }

    /// Geometrie einer Einheit (nur Vektorpfade) in Dokumentkoordinaten.
    fn unit_polylines(&self, unit: &[ObjectId]) -> Vec<Polyline> {
        self.doc
            .objects
            .iter()
            .filter(|o| unit.contains(&o.id) && matches!(o.shape, Shape::Path(_)))
            .flat_map(|o| o.polylines(TOLERANCE))
            .collect()
    }

    fn unit_layer(&self, unit: &[ObjectId]) -> usize {
        unit.first().and_then(|id| self.doc.get(*id)).map(|o| o.layer).unwrap_or(self.current_layer)
    }

    fn has_images_selected(&self) -> bool {
        self.doc.objects.iter().any(|o| self.selection.contains(&o.id) && o.is_image())
    }

    /// Ersetzt die Auswahl durch ein neues Objekt mit der Geometrie `polys`.
    fn replace_selection(&mut self, name: &str, layer: usize, polys: &[Polyline]) {
        self.checkpoint();
        let old = self.selection.clone();
        self.doc.remove(&old);
        let id = self.doc.add_path(name, layer, polylines_to_path(polys));
        self.selection = vec![id];
        self.mark_changed();
    }

    pub fn boolean_op(&mut self, op: BoolOp) {
        let units = self.ordered_units();
        if units.len() < 2 {
            self.toast(ToastKind::Info, tr("Bitte mindestens zwei Objekte (oder Gruppen) auswählen."));
            return;
        }
        if self.has_images_selected() {
            self.toast(ToastKind::Info, tr("Bilder werden bei Formoperationen nicht berücksichtigt."));
        }
        let subject = self.unit_polylines(&units[0]);
        let others: Vec<Vec<Polyline>> = units[1..].iter().map(|u| self.unit_polylines(u)).collect();
        let result = match op {
            BoolOp::Union => {
                let mut all = vec![subject];
                all.extend(others);
                vl_core::ops::weld(&all)
            }
            BoolOp::Intersection => others.iter().fold(subject, |acc, o| vl_core::ops::boolean(&acc, o, op)),
            BoolOp::Difference | BoolOp::Xor => {
                // Die übrigen Objekte zuerst vereinigen, sonst höben sich Überlappungen gegenseitig auf.
                let clip = vl_core::ops::weld(&others);
                vl_core::ops::boolean(&subject, &clip, op)
            }
        };
        if result.is_empty() {
            self.toast(
                ToastKind::Error,
                trs(&format!("{}: Das Ergebnis ist leer (nur geschlossene Formen zählen).", tr(op.label()))),
            );
            return;
        }
        let layer = self.unit_layer(&units[0]);
        self.replace_selection(tr(op.label()), layer, &result);
    }

    /// Zerlegt Pfade und Texte der Auswahl in ein Objekt je Teilpfad (Löcher werden eigene Formen).
    pub fn break_apart(&mut self) {
        let targets: Vec<ObjectId> = self
            .selection
            .iter()
            .copied()
            .filter(|id| self.doc.get(*id).is_some_and(|o| matches!(&o.shape, Shape::Path(p) if p.subpaths.len() > 1)))
            .collect();
        if targets.is_empty() {
            self.toast(ToastKind::Info, tr("Nichts zu zerlegen – die Auswahl besteht schon aus einzelnen Formen."));
            return;
        }
        self.checkpoint();
        let mut selection: Vec<ObjectId> = self.selection.iter().copied().filter(|id| !targets.contains(id)).collect();
        for id in targets {
            let Some(i) = self.doc.objects.iter().position(|o| o.id == id) else { continue };
            let mut base = self.doc.objects.remove(i);
            let Shape::Path(path) = std::mem::replace(&mut base.shape, Shape::Path(VectorPath::default())) else {
                continue;
            };
            base.text = None;
            for (k, sub) in path.subpaths.into_iter().enumerate() {
                let mut part = base.clone();
                part.id = self.doc.alloc_id();
                part.shape = Shape::Path(VectorPath { subpaths: vec![sub] });
                selection.push(part.id);
                self.doc.objects.insert(i + k, part);
            }
        }
        self.selection = selection;
        self.mark_changed();
    }

    /// Bringt jede ausgewählte Einheit (Gruppen als Ganzes) auf die Breite bzw. Höhe der zuerst gewählten.
    pub fn make_same_size(&mut self, width: bool) {
        let units = self.selection_units();
        let Some(first) = self.selection.first() else { return };
        let Some(reference) = units.iter().find(|u| u.contains(first)) else { return };
        let r = self.doc.bounds_of(reference);
        let target = if width { r.width() } else { r.height() };
        if units.len() < 2 || target <= 1e-9 {
            return;
        }
        self.checkpoint();
        for u in &units {
            let b = self.doc.bounds_of(u);
            let size = if width { b.width() } else { b.height() };
            if size <= 1e-9 {
                continue;
            }
            let k = target / size;
            let (sx, sy) = if width { (k, 1.0) } else { (1.0, k) };
            self.doc.transform_objects(u, &Affine::scale_about(sx, sy, b.center()));
        }
        self.mark_changed();
    }

    /// Startet das Verschachteln der Auswahl (Gruppen bleiben zusammen) auf der Arbeitsfläche oder der größten
    /// ausgewählten geschlossenen Form; gepackt wird an der Nullpunkt-Ecke.
    pub fn start_nesting(&mut self) {
        let mut units = self.selection_units();
        let doc = &self.doc;
        let outline = |unit: &[ObjectId]| -> Vec<Polyline> {
            unit.iter().filter_map(|id| doc.get(*id)).flat_map(|o| o.polylines(0.05)).collect()
        };
        let sheet: Option<Vec<Polyline>> = if self.shape_dialogs.nest_on_shape {
            // Fläche des Umrisses der Einheit, nur wenn sie eine geschlossene Form enthält
            let area = |u: &[ObjectId]| {
                let polys: Vec<Polyline> = outline(u).into_iter().filter(|p| p.closed).collect();
                let b = vl_core::path::bounds_of(&polys);
                if polys.is_empty() { 0.0 } else { b.width() * b.height() }
            };
            (0..units.len())
                .filter(|&i| area(&units[i]) > 0.0)
                .max_by(|&a, &b| area(&units[a]).total_cmp(&area(&units[b])))
                .map(|i| outline(&units.remove(i)).into_iter().filter(|p| p.closed).collect())
        } else {
            let p = &self.settings.profile;
            Some(vl_core::nest::sheet_rect(vl_core::Rect::from_xywh(0.0, 0.0, p.bed_width, p.bed_height)))
        };
        let parts: Vec<Vec<Polyline>> = units.iter().map(|u| outline(u)).collect();
        let Some(sheet) = sheet else {
            self.toast(ToastKind::Info, tr("Keine geschlossene Form als Platte ausgewählt."));
            return;
        };
        if parts.is_empty() {
            self.toast(ToastKind::Info, tr("Nichts zum Verschachteln ausgewählt."));
            return;
        }
        let origin = self.settings.profile.origin;
        let opts = NestOptions {
            right: matches!(origin, OriginCorner::FrontRight | OriginCorner::RearRight),
            bottom: matches!(origin, OriginCorner::FrontLeft | OriginCorner::FrontRight),
            ..self.shape_dialogs.nest
        };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(vl_core::nest::nest(&parts, &sheet, &opts));
        });
        self.shape_dialogs.nest_job = Some(NestJob { units, revision: self.revision, result: rx });
    }

    /// Übernimmt ein fertiges Verschachteln, sofern sich das Projekt inzwischen nicht geändert hat.
    fn poll_nesting(&mut self) {
        let Some(job) = &self.shape_dialogs.nest_job else { return };
        let result = match job.result.try_recv() {
            Ok(r) => r,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Vec::new(),
        };
        let Some(job) = self.shape_dialogs.nest_job.take() else { return };
        if job.revision != self.revision {
            self.toast(ToastKind::Info, tr("Das Projekt wurde inzwischen geändert – Verschachteln verworfen."));
            return;
        }
        let placed = result.iter().filter(|t| t.is_some()).count();
        if placed == 0 {
            self.toast(ToastKind::Error, tr("Kein Teil passt auf die Platte."));
            return;
        }
        self.checkpoint();
        for (unit, t) in job.units.iter().zip(&result) {
            if let Some(t) = t {
                self.doc.transform_objects(unit, t);
            }
        }
        self.mark_changed();
        let missing = job.units.len() - placed;
        if missing == 0 {
            self.toast(ToastKind::Success, trs(&format!("{placed} Teile verschachtelt.")));
        } else {
            self.toast(
                ToastKind::Info,
                trs(&format!("{placed} Teile verschachtelt, {missing} passen nicht und bleiben, wo sie sind.")),
            );
        }
    }

    /// Bringt Steckschlitze der ausgewählten Pfade auf eine neue Breite (z. B. andere Materialstärke).
    pub fn resize_slots_selection(&mut self) {
        let d = &self.shape_dialogs;
        let (from, to, tol) = (d.slots_from, d.slots_to, d.slots_tolerance);
        let mut changed: Vec<(ObjectId, VectorPath)> = Vec::new();
        let mut count = 0;
        for o in self.selection.iter().filter_map(|id| self.doc.get(*id)).filter(|o| o.text.is_none()) {
            if let Shape::Path(p) = &o.shape {
                let mut p = p.clone();
                let n = vl_core::slots::resize_slots(&mut p, &o.transform, from, to, tol);
                if n > 0 {
                    count += n;
                    changed.push((o.id, p));
                }
            }
        }
        if changed.is_empty() {
            self.toast(ToastKind::Info, trs(&format!("Keine Schlitze mit {from} mm Breite gefunden.")));
            return;
        }
        self.checkpoint();
        for (id, p) in changed {
            if let Some(o) = self.doc.get_mut(id) {
                o.shape = Shape::Path(p);
                self.geom.invalidate(id);
            }
        }
        self.mark_changed();
        self.toast(ToastKind::Success, trs(&format!("{count} Schlitze angepasst.")));
    }

    /// Schiebt die Auswahl in Richtung `dir` (Einheitsvektor, Y nach unten), bis ihre Kontur eine andere sichtbare
    /// Form berührt – sonst bis an den Rand der Arbeitsfläche.
    pub fn dock_selection(&mut self, dir: Point) {
        if self.selection.is_empty() {
            return;
        }
        let moving: Vec<Polyline> =
            self.selection.iter().filter_map(|id| self.doc.get(*id)).flat_map(|o| o.polylines(TOLERANCE)).collect();
        let obstacles: Vec<Polyline> = self
            .doc
            .objects
            .iter()
            .filter(|o| !self.selection.contains(&o.id) && self.doc.layers.get(o.layer).is_some_and(|l| l.visible))
            .flat_map(|o| o.polylines(TOLERANCE))
            .collect();
        let b = self.selection_bounds();
        let p = &self.settings.profile;
        let edge = if dir.x < 0.0 {
            b.min.x
        } else if dir.x > 0.0 {
            p.bed_width - b.max.x
        } else if dir.y < 0.0 {
            b.min.y
        } else {
            p.bed_height - b.max.y
        };
        let stop = match vl_core::dock::contact_distance(&moving, &obstacles, dir) {
            Some(t) if t <= edge.max(0.0) => t,
            _ => edge,
        };
        let dist = (stop - self.shape_dialogs.dock_gap).max(0.0);
        if dist < 1e-6 {
            self.toast(ToastKind::Info, tr("Die Auswahl liegt in dieser Richtung bereits an."));
            return;
        }
        self.transform_selection(&Affine::translate(dir.x * dist, dir.y * dist));
    }

    /// Rundet oder fast die Ecken der ausgewählten Pfade ab; gerechnet in Dokument-Millimetern, damit der
    /// Radius auch bei verzerrt skalierten Objekten stimmt.
    pub fn round_corners_selection(&mut self) {
        let d = &self.shape_dialogs;
        let (style, size, min_angle) = (d.corner_style, d.corner_size, d.corner_min_angle);
        let mut changed: Vec<(ObjectId, VectorPath)> = Vec::new();
        let mut count = 0;
        for o in self.selection.iter().filter_map(|id| self.doc.get(*id)).filter(|o| o.text.is_none()) {
            let (Shape::Path(p), Some(inv)) = (&o.shape, o.transform.inverse()) else { continue };
            let (rounded, n) =
                vl_core::corners::round_corners(&p.transformed(&o.transform), style, size, min_angle, None);
            if n > 0 {
                count += n;
                changed.push((o.id, rounded.transformed(&inv)));
            }
        }
        if changed.is_empty() {
            self.toast(
                ToastKind::Info,
                tr("Keine Ecken zwischen geraden Linien gefunden (Texte zuerst in Kurven umwandeln)."),
            );
            return;
        }
        self.checkpoint();
        for (id, p) in changed {
            if let Some(o) = self.doc.get_mut(id) {
                o.shape = Shape::Path(p);
                self.geom.invalidate(id);
            }
        }
        self.mark_changed();
        self.toast(ToastKind::Success, trs(&format!("{count} Ecken bearbeitet.")));
    }

    /// Reduziert die Knoten der ausgewählten Pfade (Douglas-Peucker), wahlweise mit Glättung zu Kurven.
    pub fn simplify_selection(&mut self) {
        let (tol, smoothing) = (self.shape_dialogs.simplify_tolerance, self.shape_dialogs.simplify_smoothing);
        let targets: Vec<(ObjectId, usize, VectorPath)> = self
            .selection
            .iter()
            .filter_map(|id| self.doc.get(*id))
            .filter(|o| o.text.is_none())
            .filter_map(|o| match &o.shape {
                Shape::Path(p) => {
                    let mut out = VectorPath::default();
                    for poly in p.flatten(&o.transform, TOLERANCE) {
                        let pts = vl_core::ops::simplify_points(&poly.points, poly.closed, tol);
                        out.append(vl_core::ops::smooth_points(&pts, poly.closed, smoothing, 65.0));
                    }
                    Some((o.id, p.segment_count(), out))
                }
                Shape::Image(_) => None,
            })
            .collect();
        if targets.is_empty() {
            self.toast(ToastKind::Info, tr("Keine Pfade ausgewählt (Texte zuerst in Kurven umwandeln)."));
            return;
        }
        self.checkpoint();
        let (mut before, mut after) = (0, 0);
        for (id, count, path) in targets {
            before += count;
            after += path.segment_count();
            if let Some(o) = self.doc.get_mut(id) {
                o.shape = Shape::Path(path);
                o.transform = Affine::IDENTITY;
                self.geom.invalidate(id);
            }
        }
        self.mark_changed();
        self.toast(ToastKind::Success, trs(&format!("{before} → {after} Knoten.")));
    }

    /// Rechnet die Auswahl in ein Graustufenbild um, z. B. um Text und Foto gemeinsam zu rastern.
    pub fn convert_to_bitmap(&mut self) {
        let opts = self.shape_dialogs.bitmap;
        let objs: Vec<&Object> = self.selection.iter().filter_map(|id| self.doc.get(*id)).collect();
        let (image, rect) = match render_objects(&objs, &opts) {
            Ok(r) => r,
            Err(RasterError::Empty) => {
                self.toast(ToastKind::Info, tr("Nichts zum Umwandeln ausgewählt."));
                return;
            }
            Err(RasterError::TooLarge { cols, rows }) => {
                self.toast(
                    ToastKind::Error,
                    trs(&format!("Das Ergebnis wäre {cols} × {rows} Pixel – bitte die Auflösung verringern.")),
                );
                return;
            }
        };
        self.checkpoint();
        if !self.shape_dialogs.bitmap_keep_original {
            let ids = std::mem::take(&mut self.selection);
            self.doc.remove(&ids);
        }
        let layer = vl_io::image_layer(&self.doc);
        let img = ImageObject {
            image: std::sync::Arc::new(image),
            width_mm: rect.width(),
            height_mm: rect.height(),
            adjust: ImageAdjust::default(),
            clip: Vec::new(),
        };
        let id = self.doc.add_image(tr("Bitmap"), layer, img, rect.min);
        self.selection = vec![id];
        self.current_layer = layer;
        self.mark_changed();
    }

    /// Legt eine straff gespannte Hülle (konvexe Hülle) um die Auswahl, z. B. als Schnittkontur um Aufkleber.
    pub fn rubber_band_outline(&mut self) {
        let points: Vec<Point> = self
            .selection
            .iter()
            .filter_map(|id| self.doc.get(*id))
            .flat_map(|o| o.polylines(0.05))
            .flat_map(|p| p.points)
            .collect();
        let hull = vl_core::ops::convex_hull(&points);
        if hull.len() < 3 {
            self.toast(ToastKind::Info, tr("Für einen Umriss sind mindestens drei Punkte nötig."));
            return;
        }
        self.checkpoint();
        let id = self.doc.add_path(tr("Gummiband-Umriss"), self.current_layer, VectorPath::polygon(&hull, true));
        self.selection = vec![id];
        self.mark_changed();
    }

    /// Reine Pfadobjekte der Auswahl (ohne Bilder und Texte), bei leerer Auswahl alle sichtbaren.
    fn cleanup_targets(&self) -> Vec<ObjectId> {
        let all = self.selection.is_empty();
        self.doc
            .objects
            .iter()
            .filter(|o| o.text.is_none() && matches!(o.shape, Shape::Path(_)))
            .filter(|o| if all { self.doc.layers[o.layer].visible } else { self.selection.contains(&o.id) })
            .map(|o| o.id)
            .collect()
    }

    /// Verbindet offene Pfadstücke (je Ebene), deren Enden höchstens die Toleranz auseinanderliegen, und schließt
    /// entstandene Ringe. Jeder Linienzug wird ein eigenes Objekt.
    pub fn join_open_paths(&mut self) {
        let tol = self.shape_dialogs.join_tolerance;
        let mut by_layer: std::collections::BTreeMap<usize, (Vec<ObjectId>, Vec<vl_core::SubPath>)> =
            Default::default();
        for id in self.cleanup_targets() {
            let Some(o) = self.doc.get(id) else { continue };
            let Shape::Path(p) = &o.shape else { continue };
            if p.subpaths.iter().all(|s| s.closed) {
                continue;
            }
            let entry = by_layer.entry(o.layer).or_default();
            entry.0.push(id);
            entry.1.extend(p.transformed(&o.transform).subpaths);
        }
        let results: Vec<(usize, Vec<ObjectId>, vl_core::cleanup::JoinResult)> = by_layer
            .into_iter()
            .map(|(layer, (ids, subs))| (layer, ids, vl_core::cleanup::join_paths(subs, tol)))
            .collect();
        let (joined, closed): (usize, usize) =
            results.iter().fold((0, 0), |(j, c), (_, _, r)| (j + r.joined, c + r.closed));
        if joined == 0 && closed == 0 {
            self.toast(ToastKind::Info, trs(&format!("Keine offenen Enden näher als {tol} mm gefunden.")));
            return;
        }
        self.checkpoint();
        let mut selection: Vec<ObjectId> = Vec::new();
        for (layer, ids, r) in results {
            self.doc.remove(&ids);
            for s in r.subpaths {
                selection.push(self.doc.add_path(tr("Pfad"), layer, VectorPath { subpaths: vec![s] }));
            }
        }
        self.selection = selection;
        self.mark_changed();
        self.toast(ToastKind::Success, trs(&format!("{joined} Verbindungen, {closed} Pfade geschlossen.")));
    }

    /// Entfernt doppelte Linien (auch gegenläufige und solche in verschiedenen Objekten), damit nichts zweimal
    /// geschnitten wird. Objekte ohne verbleibende Linien werden gelöscht.
    pub fn delete_duplicate_lines(&mut self) {
        let ids = self.cleanup_targets();
        let paths: Vec<VectorPath> = ids
            .iter()
            .filter_map(|id| self.doc.get(*id))
            .filter_map(|o| match &o.shape {
                Shape::Path(p) => Some(p.transformed(&o.transform)),
                Shape::Image(_) => None,
            })
            .collect();
        let (cleaned, removed) = vl_core::cleanup::remove_duplicate_segments(&paths, 0.01);
        if removed == 0 {
            self.toast(ToastKind::Info, tr("Keine doppelten Linien gefunden."));
            return;
        }
        self.checkpoint();
        let mut empty = Vec::new();
        for ((id, before), after) in ids.iter().zip(&paths).zip(cleaned) {
            if after.is_empty() {
                empty.push(*id);
            } else if after != *before
                && let Some(o) = self.doc.get_mut(*id)
            {
                o.shape = Shape::Path(after);
                o.transform = Affine::IDENTITY;
                self.geom.invalidate(*id);
            }
        }
        self.doc.remove(&empty);
        self.selection.retain(|id| !empty.contains(id));
        self.mark_changed();
        self.toast(ToastKind::Success, trs(&format!("{removed} doppelte Linien entfernt.")));
    }

    /// Vereinigt alle ausgewählten Objekte – auch überlappende Buchstaben – zu einer Kontur.
    pub fn weld_selection(&mut self) {
        let units = self.ordered_units();
        if units.is_empty() {
            return;
        }
        let objects: Vec<Vec<Polyline>> = self
            .doc
            .objects
            .iter()
            .filter(|o| self.selection.contains(&o.id) && matches!(o.shape, Shape::Path(_)))
            .map(|o| o.polylines(TOLERANCE))
            .collect();
        let result = vl_core::ops::weld(&objects);
        if result.is_empty() {
            self.toast(ToastKind::Error, tr("Verschweißen: Es gibt keine geschlossenen Formen."));
            return;
        }
        let layer = self.unit_layer(&units[0]);
        self.replace_selection(tr("Verschweißt"), layer, &result);
    }

    fn apply_offset(&mut self) {
        let d = &self.shape_dialogs;
        let (distance, direction) = (d.offset_distance, d.offset_direction);
        let (corner, keep) = (d.offset_corner, d.offset_keep_original);
        let units = self.ordered_units();
        if units.is_empty() {
            self.toast(ToastKind::Info, tr("Bitte zuerst Objekte auswählen."));
            return;
        }
        let mut results: Vec<(usize, Vec<Polyline>)> = Vec::new();
        for u in &units {
            let (layer, polys) = (self.unit_layer(u), self.unit_polylines(u));
            if direction != OffsetDirection::Inward {
                results.push((layer, vl_core::ops::offset(&polys, distance, corner)));
            }
            if direction != OffsetDirection::Outward {
                // Offene Pfade werden nur einmal umrandet (beim Versatz nach außen)
                let closed: Vec<Polyline> = if direction == OffsetDirection::Both {
                    polys.into_iter().filter(|p| p.closed).collect()
                } else {
                    polys
                };
                results.push((layer, vl_core::ops::offset(&closed, -distance, corner)));
            }
        }
        results.retain(|(_, p)| !p.is_empty());
        if results.is_empty() {
            self.toast(ToastKind::Error, tr("Versatz: Das Ergebnis ist leer (zu weit nach innen?)."));
            return;
        }
        self.checkpoint();
        if !keep {
            let old = self.selection.clone();
            self.doc.remove(&old);
        }
        let mut new_ids = Vec::new();
        for (layer, polys) in results {
            new_ids.push(self.doc.add_path(tr("Versatz"), layer, polylines_to_path(&polys)));
        }
        self.selection = new_ids;
        self.mark_changed();
    }

    /// Für „Entlang Pfad“: (Pfad-Einheit, Vorlage-Einheiten). Der Pfad ist standardmäßig die zuletzt gewählte
    /// Einheit.
    fn path_and_templates(&self) -> Option<(Vec<ObjectId>, Vec<Vec<ObjectId>>)> {
        let mut units = self.ordered_units();
        if units.len() < 2 {
            return None;
        }
        // ordered_units stellt die zuerst gewählte Einheit nach vorn; die zuletzt gewählte suchen
        let last = self.selection.last()?;
        let last_idx = units.iter().position(|u| u.contains(last))?;
        let path_idx = if self.shape_dialogs.path_swap { 0 } else { last_idx };
        let path = units.remove(path_idx);
        Some((path, units))
    }

    fn unit_name(&self, unit: &[ObjectId]) -> String {
        let name = unit.first().and_then(|id| self.doc.get(*id)).map(|o| o.name.clone()).unwrap_or_default();
        let name = trs(&name);
        if unit.len() > 1 { format!("{name} (+{})", unit.len() - 1) } else { name }
    }

    fn apply_along_path(&mut self) {
        let Some((path_unit, templates)) = self.path_and_templates() else {
            self.toast(ToastKind::Info, tr("Bitte die Vorlage und danach den Pfad auswählen (Umschalt+Klick)."));
            return;
        };
        let Some(line) = self.unit_polylines(&path_unit).into_iter().max_by(|a, b| a.length().total_cmp(&b.length()))
        else {
            self.toast(ToastKind::Error, tr("Der Pfad enthält keine Linien."));
            return;
        };
        let d = &self.shape_dialogs;
        let spots = vl_core::ops::points_along(&line, d.path_count.max(1) as usize);
        let (rotate, remove) = (d.path_rotate, d.path_remove_template);
        let template: Vec<ObjectId> = templates.concat();
        let center = self.doc.bounds_of(&template).center();
        let originals: Vec<vl_core::Object> =
            self.doc.objects.iter().filter(|o| template.contains(&o.id)).cloned().collect();
        self.checkpoint();
        let mut created = Vec::new();
        for (p, angle) in &spots {
            let ids = self.doc.duplicate(&originals, Point::ZERO);
            let mut t = Affine::translate(-center.x, -center.y);
            if rotate {
                t = t.then(&Affine::rotate(*angle));
            }
            t = t.then(&Affine::translate(p.x, p.y));
            self.doc.transform_objects(&ids, &t);
            created.extend(ids);
        }
        if remove {
            self.doc.remove(&template);
        }
        self.selection = created;
        self.mark_changed();
        self.toast(ToastKind::Success, trs(&format!("{} Exemplare entlang des Pfades angelegt.", spots.len())));
    }

    fn apply_array(&mut self) {
        if self.shape_dialogs.array_mode == ArrayMode::Path {
            self.apply_along_path();
            return;
        }
        if self.selection.is_empty() {
            self.toast(ToastKind::Info, tr("Bitte zuerst Objekte auswählen."));
            return;
        }
        let d = &self.shape_dialogs;
        let b = self.selection_bounds();
        let originals = self.selected_objects();
        let mut copies: Vec<Affine> = Vec::new();
        match d.array_mode {
            ArrayMode::Grid => {
                for r in 0..d.rows.max(1) {
                    for c in 0..d.columns.max(1) {
                        if r == 0 && c == 0 {
                            continue;
                        }
                        let dx = c as f64 * (b.width() + d.gap_x);
                        let dy = r as f64 * (b.height() + d.gap_y);
                        copies.push(Affine::translate(dx, dy));
                    }
                }
            }
            ArrayMode::Circle => {
                let n = d.circle_count.max(2);
                // Original oben auf dem Kreis, Mittelpunkt darunter (Dokument: Y nach unten).
                let center = b.center() + Point::new(0.0, d.circle_radius);
                let full = (d.circle_angle - 360.0).abs() < 1e-6;
                let step = d.circle_angle.to_radians() / if full { n as f64 } else { (n - 1) as f64 };
                for i in 1..n {
                    let a = step * i as f64;
                    let t = if d.circle_rotate {
                        Affine::rotate_about(a, center)
                    } else {
                        let moved = (b.center() - center).rotated(a) + center;
                        Affine::translate(moved.x - b.center().x, moved.y - b.center().y)
                    };
                    copies.push(t);
                }
            }
            ArrayMode::Path => {}
        }
        self.checkpoint();
        let mut all = self.selection.clone();
        for t in copies {
            let ids = self.doc.duplicate(&originals, Point::ZERO);
            self.doc.transform_objects(&ids, &t);
            all.extend(ids);
        }
        let n = all.len() / originals.len().max(1);
        self.selection = all;
        self.mark_changed();
        self.toast(ToastKind::Success, trs(&format!("{n} Exemplare angelegt.")));
    }
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    offset_window(app, ctx);
    array_window(app, ctx);
    bitmap_window(app, ctx);
    simplify_window(app, ctx);
    app.poll_nesting();
    nest_window(app, ctx);
    slots_window(app, ctx);
    corners_window(app, ctx);
}

fn corners_window(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.shape_dialogs.corners_open {
        return;
    }
    let mut open = true;
    let mut apply = false;
    let has = !app.selection.is_empty();
    egui::Window::new(trs(&format!("{} Ecken abrunden", icon::FRAME_CORNERS)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let d = &mut app.shape_dialogs;
            ui.horizontal(|ui| {
                ui.radio_value(&mut d.corner_style, CornerStyle::Round, tr("Abrunden"));
                ui.radio_value(&mut d.corner_style, CornerStyle::Chamfer, tr("Abschrägen (Fase)"));
            });
            egui::Grid::new("corners").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label(if d.corner_style == CornerStyle::Round { tr("Radius") } else { tr("Fasenlänge") });
                ui.add(
                    DragValue::new(&mut d.corner_size).speed(0.05).range(0.01..=500.0).max_decimals(2).suffix(" mm"),
                )
                .on_hover_text(tr("Bei Fasen: Abstand von der Ecke entlang jeder Kante"));
                ui.end_row();
                ui.label(tr("Ab Knickwinkel"));
                ui.add(
                    DragValue::new(&mut d.corner_min_angle).speed(0.5).range(1.0..=179.0).max_decimals(0).suffix("°"),
                )
                .on_hover_text(tr("Flachere Knicke (z. B. in abgeflachten Rundungen) bleiben unverändert"));
                ui.end_row();
            });
            ui.label(
                RichText::new(tr(
                    "Nur Ecken zwischen zwei geraden Linien werden bearbeitet. Ist eine Kante zu kurz, wird der \
                     Radius verkleinert. Einzelne Ecken: im Knotenwerkzeug einen Knoten wählen.",
                ))
                .small()
                .color(pal.text_weak),
            );
            ui.add_space(6.0);
            if ui
                .add_enabled(has, Button::new(RichText::new(tr("Anwenden")).color(Color32::WHITE)).fill(pal.accent))
                .clicked()
            {
                apply = true;
            }
            if !has {
                ui.label(RichText::new(tr("Nichts ausgewählt")).small().color(pal.text_weak));
            }
        });
    if apply {
        app.round_corners_selection();
    }
    if !open {
        app.shape_dialogs.corners_open = false;
    }
}

fn slots_window(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.shape_dialogs.slots_open {
        return;
    }
    let mut open = true;
    let mut apply = false;
    let has = !app.selection.is_empty();
    egui::Window::new(trs(&format!("{} Schlitze anpassen", icon::ARROWS_OUT_LINE_HORIZONTAL)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let d = &mut app.shape_dialogs;
            egui::Grid::new("slots").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label(tr("Breite jetzt"));
                ui.add(DragValue::new(&mut d.slots_from).speed(0.05).range(0.1..=100.0).max_decimals(2).suffix(" mm"))
                    .on_hover_text(tr("Breite der vorhandenen Schlitze, meist die alte Materialstärke"));
                ui.end_row();
                ui.label(tr("Neue Breite"));
                ui.add(DragValue::new(&mut d.slots_to).speed(0.05).range(0.1..=100.0).max_decimals(2).suffix(" mm"))
                    .on_hover_text(tr("Gemessene Stärke des neuen Materials, ggf. plus etwas Spiel"));
                ui.end_row();
                ui.label(tr("Toleranz"));
                ui.add(
                    DragValue::new(&mut d.slots_tolerance).speed(0.01).range(0.0..=2.0).max_decimals(2).suffix(" mm"),
                );
                ui.end_row();
            });
            ui.label(
                RichText::new(tr(
                    "Kerben und Langlöcher dieser Breite werden symmetrisch verbreitert oder verengt, Tiefe und \
                     Lage bleiben. Gleich breite Zapfen bleiben unverändert.",
                ))
                .small()
                .color(pal.text_weak),
            );
            ui.add_space(6.0);
            if ui
                .add_enabled(has, Button::new(RichText::new(tr("Anpassen")).color(Color32::WHITE)).fill(pal.accent))
                .clicked()
            {
                apply = true;
            }
            if !has {
                ui.label(RichText::new(tr("Nichts ausgewählt")).small().color(pal.text_weak));
            }
        });
    if apply {
        app.resize_slots_selection();
    }
    if !open {
        app.shape_dialogs.slots_open = false;
    }
}

fn nest_window(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.shape_dialogs.nest_open {
        return;
    }
    let mut open = true;
    let mut start = false;
    let units = app.selection_units().len();
    let running = app.shape_dialogs.nest_job.is_some();
    let corner = tr(app.settings.profile.origin.label());
    egui::Window::new(trs(&format!("{} Verschachteln", icon::PUZZLE_PIECE)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let d = &mut app.shape_dialogs;
            egui::Grid::new("nest").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label(tr("Abstand"));
                ui.add(DragValue::new(&mut d.nest.spacing).speed(0.1).range(0.0..=50.0).suffix(" mm"))
                    .on_hover_text(tr("Mindestabstand zwischen den Teilen (Schnittbreite und Stege bedenken)"));
                ui.end_row();
                ui.label(tr("Rand"));
                ui.add(DragValue::new(&mut d.nest.margin).speed(0.1).range(0.0..=50.0).suffix(" mm"))
                    .on_hover_text(tr("Abstand zum Rand der Platte"));
                ui.end_row();
                ui.label(tr("Drehen"));
                ui.horizontal(|ui| {
                    for (n, label) in [(1, tr("nein")), (2, "180°"), (4, "90°"), (8, "45°")] {
                        ui.selectable_value(&mut d.nest.rotations, n, label);
                    }
                })
                .response
                .on_hover_text(tr("Mehr Drehungen passen oft dichter, rechnen aber länger"));
                ui.end_row();
            });
            ui.checkbox(&mut d.nest.use_holes, tr("Kleine Teile in Löcher setzen"));
            ui.checkbox(&mut d.nest_on_shape, tr("Größte ausgewählte Form ist die Platte")).on_hover_text(tr(
                "Zum Beispiel den Umriss eines Reststücks zeichnen und mit auswählen; sonst wird die ganze Arbeitsfläche genutzt",
            ));
            ui.label(
                RichText::new(trs(&format!(
                    "Gruppen bleiben zusammen. Gepackt wird an der Nullpunkt-Ecke ({corner}), größte Teile zuerst."
                )))
                .small()
                .color(pal.text_weak),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let enabled = units >= 1 && !running;
                if ui
                    .add_enabled(
                        enabled,
                        Button::new(RichText::new(tr("Verschachteln")).color(Color32::WHITE)).fill(pal.accent),
                    )
                    .clicked()
                {
                    start = true;
                }
                if running {
                    ui.spinner();
                    ui.label(RichText::new(tr("rechnet…")).small().color(pal.text_weak));
                } else {
                    ui.label(RichText::new(trs(&format!("{units} Teile ausgewählt"))).small().color(pal.text_weak));
                }
            });
        });
    if running {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    if start {
        app.start_nesting();
    }
    if !open {
        app.shape_dialogs.nest_open = false;
    }
}

fn simplify_window(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.shape_dialogs.simplify_open {
        return;
    }
    let mut open = true;
    let mut apply = false;
    let has = !app.selection.is_empty();
    egui::Window::new(trs(&format!("{} Formen vereinfachen", icon::BEZIER_CURVE)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let d = &mut app.shape_dialogs;
            egui::Grid::new("simplify").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label(tr("Toleranz"));
                ui.add(
                    DragValue::new(&mut d.simplify_tolerance)
                        .speed(0.005)
                        .range(0.001..=5.0)
                        .max_decimals(3)
                        .suffix(" mm"),
                )
                .on_hover_text(tr("So weit dürfen die neuen Linien von der alten Form abweichen"));
                ui.end_row();
                ui.label(tr("Glätten"));
                ui.add(egui::Slider::new(&mut d.simplify_smoothing, 0.0..=1.0).fixed_decimals(2))
                    .on_hover_text(tr("0 = gerade Linien, höher = flache Knicke werden zu Kurven, Ecken bleiben"));
                ui.end_row();
            });
            ui.label(
                RichText::new(tr(
                    "Für nachgezeichnete oder importierte Formen mit sehr vielen Knoten: weniger Knoten laufen \
                     ruhiger und schneller. Texte vorher in Kurven umwandeln.",
                ))
                .small()
                .color(pal.text_weak),
            );
            ui.add_space(6.0);
            if ui
                .add_enabled(has, Button::new(RichText::new(tr("Anwenden")).color(Color32::WHITE)).fill(pal.accent))
                .clicked()
            {
                apply = true;
            }
            if !has {
                ui.label(RichText::new(tr("Nichts ausgewählt")).small().color(pal.text_weak));
            }
        });
    if apply {
        app.simplify_selection();
    }
    if !open {
        app.shape_dialogs.simplify_open = false;
    }
}

fn bitmap_window(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.shape_dialogs.bitmap_open {
        return;
    }
    let mut open = true;
    let mut apply = false;
    let objs: Vec<&Object> = app.selection.iter().filter_map(|id| app.doc.get(*id)).collect();
    let bounds = render_bounds(&objs, &app.shape_dialogs.bitmap);
    let size = (!bounds.is_empty()).then(|| render_size(&bounds, &app.shape_dialogs.bitmap));
    egui::Window::new(trs(&format!("{} In Bitmap umwandeln", icon::IMAGE)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let d = &mut app.shape_dialogs;
            let o = &mut d.bitmap;
            egui::Grid::new("bitmap").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label(tr("Auflösung"));
                ui.horizontal(|ui| {
                    ui.add(DragValue::new(&mut o.dpi).speed(1.0).range(25.0..=1200.0).max_decimals(0).suffix(" dpi"));
                    ui.label(
                        RichText::new(trs(&format!("{:.3} mm je Pixel", o.pixel_mm()))).small().color(pal.text_weak),
                    );
                });
                ui.end_row();
                ui.label(tr("Formen"));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut o.outline, false, tr("Gefüllt"));
                    ui.radio_value(&mut o.outline, true, tr("Nur Konturen"));
                });
                ui.end_row();
                ui.label(tr("Strichbreite"));
                ui.add(DragValue::new(&mut o.line_width).speed(0.01).range(0.01..=10.0).suffix(" mm"))
                    .on_hover_text(tr("Für Konturen und offene Pfade"));
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut d.bitmap_keep_original, tr("Originale behalten"));
                ui.end_row();
            });
            ui.label(
                RichText::new(tr(
                    "Bilder in der Auswahl werden mit eingerechnet, die dunklere Stelle gewinnt. Das Ergebnis \
                     kommt auf eine Bildebene und lässt sich wie ein Foto rastern oder dithern.",
                ))
                .small()
                .color(pal.text_weak),
            );
            ui.add_space(4.0);
            match size {
                Some((c, r)) => {
                    let too_large = c.saturating_mul(r) > vl_raster::render::MAX_PIXELS;
                    let color = if too_large { pal.danger } else { pal.text };
                    ui.label(RichText::new(trs(&format!("Ergebnis: {c} × {r} Pixel"))).color(color));
                }
                None => {
                    ui.label(RichText::new(tr("Nichts ausgewählt")).small().color(pal.text_weak));
                }
            }
            ui.add_space(6.0);
            if ui
                .add_enabled(
                    size.is_some(),
                    Button::new(RichText::new(tr("Umwandeln")).color(Color32::WHITE)).fill(pal.accent),
                )
                .clicked()
            {
                apply = true;
            }
        });
    if apply {
        app.convert_to_bitmap();
    }
    if !open {
        app.shape_dialogs.bitmap_open = false;
    }
}

fn offset_window(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.shape_dialogs.offset_open {
        return;
    }
    let mut open = true;
    let mut apply = false;
    let has = !app.selection.is_empty();
    egui::Window::new(trs(&format!("{} Versatz", icon::SQUARES_FOUR)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let d = &mut app.shape_dialogs;
            egui::Grid::new("offset").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label(tr("Abstand"));
                ui.add(DragValue::new(&mut d.offset_distance).speed(0.05).range(0.01..=200.0).suffix(" mm"));
                ui.end_row();
                ui.label(tr("Richtung"));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut d.offset_direction, OffsetDirection::Outward, tr("Nach außen"));
                    ui.radio_value(&mut d.offset_direction, OffsetDirection::Inward, tr("Nach innen"));
                    ui.radio_value(&mut d.offset_direction, OffsetDirection::Both, tr("Beidseitig"));
                });
                ui.end_row();
                ui.label(tr("Ecken"));
                ui.horizontal(|ui| {
                    for c in Corner::ALL {
                        ui.radio_value(&mut d.offset_corner, c, tr(c.label()));
                    }
                });
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut d.offset_keep_original, tr("Original behalten"));
                ui.end_row();
            });
            ui.label(RichText::new(tr("Offene Pfade werden mit dem Abstand umrandet.")).small().color(pal.text_weak));
            ui.add_space(6.0);
            if ui
                .add_enabled(has, Button::new(RichText::new(tr("Anwenden")).color(Color32::WHITE)).fill(pal.accent))
                .clicked()
            {
                apply = true;
            }
            if !has {
                ui.label(RichText::new(tr("Nichts ausgewählt")).small().color(pal.text_weak));
            }
        });
    if apply {
        app.apply_offset();
    }
    if !open {
        app.shape_dialogs.offset_open = false;
    }
}

fn array_window(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.shape_dialogs.array_open {
        return;
    }
    let mut open = true;
    let mut apply = false;
    let (path_label, template_label, has) = match app.shape_dialogs.array_mode {
        ArrayMode::Path => match app.path_and_templates() {
            Some((p, t)) => {
                let names: Vec<String> = t.iter().map(|u| app.unit_name(u)).collect();
                (app.unit_name(&p), names.join(", "), true)
            }
            None => ("–".to_string(), tr("Vorlage und Pfad auswählen").to_string(), false),
        },
        _ => (String::new(), String::new(), !app.selection.is_empty()),
    };
    egui::Window::new(trs(&format!("{} Reihen-Kopien", icon::GRID_NINE)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let d = &mut app.shape_dialogs;
            ui.horizontal(|ui| {
                ui.selectable_value(&mut d.array_mode, ArrayMode::Grid, trs(&format!("{} Raster", icon::GRID_FOUR)));
                ui.selectable_value(
                    &mut d.array_mode,
                    ArrayMode::Circle,
                    trs(&format!("{} Kreis", icon::CIRCLE_DASHED)),
                );
                ui.selectable_value(&mut d.array_mode, ArrayMode::Path, trs(&format!("{} Entlang Pfad", icon::PATH)));
            });
            ui.add_space(6.0);
            egui::Grid::new("array").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| match d.array_mode {
                ArrayMode::Grid => {
                    ui.label(tr("Spalten × Zeilen"));
                    ui.horizontal(|ui| {
                        ui.add(DragValue::new(&mut d.columns).range(1..=100));
                        ui.label("×");
                        ui.add(DragValue::new(&mut d.rows).range(1..=100));
                    });
                    ui.end_row();
                    ui.label(tr("Abstand X / Y"));
                    ui.horizontal(|ui| {
                        ui.add(DragValue::new(&mut d.gap_x).speed(0.1).range(-500.0..=500.0).suffix(" mm"));
                        ui.add(DragValue::new(&mut d.gap_y).speed(0.1).range(-500.0..=500.0).suffix(" mm"));
                    });
                    ui.end_row();
                }
                ArrayMode::Circle => {
                    ui.label(tr("Anzahl"));
                    ui.add(DragValue::new(&mut d.circle_count).range(2..=360));
                    ui.end_row();
                    ui.label(tr("Radius"));
                    ui.add(DragValue::new(&mut d.circle_radius).speed(0.2).range(0.1..=2000.0).suffix(" mm"));
                    ui.end_row();
                    ui.label(tr("Winkelbereich"));
                    ui.add(DragValue::new(&mut d.circle_angle).speed(1.0).range(1.0..=360.0).suffix("°"));
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut d.circle_rotate, tr("Kopien mitdrehen"));
                    ui.end_row();
                }
                ArrayMode::Path => {
                    ui.label(tr("Pfad"));
                    ui.label(RichText::new(&path_label).strong());
                    ui.end_row();
                    ui.label(tr("Vorlage"));
                    ui.label(&template_label);
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut d.path_swap, tr("Pfad und Vorlage tauschen"));
                    ui.end_row();
                    ui.label(tr("Anzahl"));
                    ui.add(DragValue::new(&mut d.path_count).range(1..=1000));
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut d.path_rotate, tr("An Pfadrichtung ausrichten"));
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut d.path_remove_template, tr("Vorlage entfernen"));
                    ui.end_row();
                }
            });
            ui.label(
                RichText::new(match d.array_mode {
                    ArrayMode::Grid => tr("Abstand = Lücke zwischen den Kopien."),
                    ArrayMode::Circle => tr("Das Original liegt oben, der Kreismittelpunkt um den Radius darunter."),
                    ArrayMode::Path => {
                        tr("Vorlage auswählen, dann mit Umschalt+Klick den Pfad. Die Kopien werden gleichmäßig \
                         verteilt (offene Pfade von Anfang bis Ende).")
                    }
                })
                .small()
                .color(pal.text_weak),
            );
            ui.add_space(6.0);
            if ui
                .add_enabled(
                    has,
                    Button::new(RichText::new(tr("Kopien anlegen")).color(Color32::WHITE)).fill(pal.accent),
                )
                .clicked()
            {
                apply = true;
            }
        });
    if apply {
        app.apply_array();
    }
    if !open {
        app.shape_dialogs.array_open = false;
    }
}
