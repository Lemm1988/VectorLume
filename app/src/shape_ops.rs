//! Formoperationen auf der Auswahl: Boolesche Operationen, Verschweißen, Versatz und Reihen-Kopien.

use crate::app::{ToastKind, VectorLumeApp};
use crate::i18n::{tr, trs};
use crate::theme;
use egui::{Align2, Button, Color32, DragValue, RichText, vec2};
use egui_phosphor::regular as icon;
use vl_core::ops::{BoolOp, Corner};
use vl_core::{Affine, ObjectId, Point, Polyline, Shape, VectorPath};

/// Toleranz beim Abflachen von Kurven für Formoperationen (mm).
const TOLERANCE: f64 = 0.01;

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
    pub offset_inward: bool,
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
}

impl Default for ShapeDialogs {
    fn default() -> Self {
        Self {
            offset_open: false,
            offset_distance: 2.0,
            offset_inward: false,
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
        let distance = if d.offset_inward { -d.offset_distance } else { d.offset_distance };
        let (corner, keep) = (d.offset_corner, d.offset_keep_original);
        let units = self.ordered_units();
        if units.is_empty() {
            self.toast(ToastKind::Info, tr("Bitte zuerst Objekte auswählen."));
            return;
        }
        let results: Vec<(usize, Vec<Polyline>)> = units
            .iter()
            .map(|u| (self.unit_layer(u), vl_core::ops::offset(&self.unit_polylines(u), distance, corner)))
            .filter(|(_, p)| !p.is_empty())
            .collect();
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
                    ui.radio_value(&mut d.offset_inward, false, tr("Nach außen"));
                    ui.radio_value(&mut d.offset_inward, true, tr("Nach innen"));
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
