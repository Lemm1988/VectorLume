//! Knotenwerkzeug: Knoten und Kurvengriffe eines Pfades direkt auf der Arbeitsfläche bearbeiten.

use crate::app::VectorLumeApp;
use crate::canvas::{Drag, HANDLE, ViewTf};
use crate::i18n::tr;
use crate::snapping::{SnapKind, SnapTargets};
use crate::theme;
use crate::units::UnitField;
use egui::{CursorIcon, PointerButton, Pos2, RichText, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;
use vl_core::nodes::{self, HandleRef, NodeRef};
use vl_core::{Affine, ObjectId, Point, Shape as DocShape, VectorPath};

#[derive(Default)]
pub struct NodeEdit {
    /// Pfad, dessen Knoten gerade bearbeitet werden.
    pub target: Option<ObjectId>,
    pub selected: Option<NodeRef>,
}

#[derive(Clone, Copy)]
enum Hit {
    Node(NodeRef),
    Handle(HandleRef),
}

impl VectorLumeApp {
    fn node_target(&self) -> Option<(&VectorPath, Affine)> {
        let obj = self.doc.get(self.node_edit.target?)?;
        match &obj.shape {
            DocShape::Path(p) => Some((p, obj.transform)),
            DocShape::Image(_) => None,
        }
    }

    /// Ändert den Zielpfad (lokale Koordinaten); `f` erhält den Pfad und die Umkehrung der Objekttransformation.
    fn edit_target(&mut self, f: impl FnOnce(&mut VectorPath, &Affine)) -> bool {
        let Some(id) = self.node_edit.target else { return false };
        let Some(obj) = self.doc.get_mut(id) else { return false };
        let Some(inv) = obj.transform.inverse() else { return false };
        let DocShape::Path(path) = &mut obj.shape else { return false };
        f(path, &inv);
        // Knoten eines Textes bearbeiten macht ihn zu Kurven (sonst ginge die Änderung beim Neuerzeugen verloren)
        obj.text = None;
        self.geom.invalidate(id);
        true
    }

    /// Hält Ziel und gewählten Knoten gültig (nach Löschen, Rückgängig, Ebenensperre …).
    pub(crate) fn node_sync(&mut self) {
        let valid = self
            .node_edit
            .target
            .and_then(|id| self.doc.get(id))
            .is_some_and(|o| !o.is_image() && self.doc.layers[o.layer].selectable());
        if !valid {
            self.node_edit = NodeEdit::default();
        }
        if self.node_edit.target.is_none()
            && let [id] = self.selection[..]
            && self.doc.get(id).is_some_and(|o| !o.is_image())
        {
            self.node_edit.target = Some(id);
        }
        if let Some(n) = self.node_edit.selected
            && !self.node_target().is_some_and(|(p, _)| nodes::nodes(p).iter().any(|(r, _)| *r == n))
        {
            self.node_edit.selected = None;
        }
    }

    fn node_hit(&self, screen: Pos2, tf: &ViewTf) -> Option<Hit> {
        let (path, transform) = self.node_target()?;
        let full = transform.then(&tf.affine());
        let near = |p: Point| {
            let s = full.apply(p);
            Pos2::new(s.x as f32, s.y as f32).distance(screen) <= HANDLE + 3.0
        };
        // Griffe haben Vorrang, da sie oft auf einem Knoten liegen, der sonst nicht mehr greifbar wäre
        if let Some(sel) = self.node_edit.selected
            && let Some((h, _, _)) = nodes::handles_near(path, sel).into_iter().find(|(_, p, _)| near(*p))
        {
            return Some(Hit::Handle(h));
        }
        nodes::nodes(path).into_iter().find(|(_, p)| near(*p)).map(|(n, _)| Hit::Node(n))
    }

    pub(crate) fn draw_nodes(&self, painter: &egui::Painter, tf: &ViewTf, pal: &theme::Palette) {
        let Some((path, transform)) = self.node_target() else { return };
        let full = transform.then(&tf.affine());
        let scr = |p: Point| {
            let s = full.apply(p);
            Pos2::new(s.x as f32, s.y as f32)
        };
        let clip = painter.clip_rect().expand(HANDLE * 2.0);
        if let Some(sel) = self.node_edit.selected {
            for (_, h, anchor) in nodes::handles_near(path, sel) {
                let (h, a) = (scr(h), scr(anchor));
                painter.line_segment([a, h], Stroke::new(1.0, pal.accent));
                painter.circle_filled(h, HANDLE - 1.0, pal.panel);
                painter.circle_stroke(h, HANDLE - 1.0, Stroke::new(1.5, pal.accent));
            }
        }
        for (n, p) in nodes::nodes(path) {
            let s = scr(p);
            if !clip.contains(s) {
                continue;
            }
            let r = egui::Rect::from_center_size(s, vec2(HANDLE * 1.6, HANDLE * 1.6));
            let selected = self.node_edit.selected == Some(n);
            painter.rect_filled(r, 1.0, if selected { pal.accent } else { pal.panel });
            painter.rect_stroke(r, 1.0, Stroke::new(1.3, pal.selection), egui::StrokeKind::Middle);
        }
    }

    /// Fangziele beim Ziehen eines Knotens: andere Objekte und die übrigen Knoten des eigenen Pfades.
    fn node_snap_targets(&self, dragged: Option<NodeRef>) -> SnapTargets {
        let Some(id) = self.node_edit.target else { return SnapTargets::default() };
        let mut targets = self.snap_targets_except(&[id]);
        if self.settings.snap_to_objects
            && let Some((path, transform)) = self.node_target()
        {
            for (n, p) in nodes::nodes(path) {
                if Some(n) != dragged {
                    targets.add_point(transform.apply(p), SnapKind::Node);
                }
            }
        }
        targets
    }

    fn node_pick(&mut self, p: Point, tol: f64) {
        let hit = self.hit_test(p, tol).filter(|id| self.doc.get(*id).is_some_and(|o| !o.is_image()));
        self.node_edit = NodeEdit { target: hit, selected: None };
        match hit {
            Some(id) => {
                self.selection = vec![id];
                if let Some(o) = self.doc.get(id) {
                    self.current_layer = o.layer;
                }
            }
            None => self.selection.clear(),
        }
    }

    pub(crate) fn node_interaction(
        &mut self,
        ctx: &egui::Context,
        resp: &egui::Response,
        tf: &ViewTf,
        space: bool,
        tol: f64,
        snap_tol: f64,
    ) {
        if self.preview.active {
            return;
        }
        if resp.hovered() && !space && self.drag.is_idle() {
            let over = resp.hover_pos().and_then(|p| self.node_hit(p, tf)).is_some();
            ctx.set_cursor_icon(if over { CursorIcon::PointingHand } else { CursorIcon::Default });
        }

        if resp.drag_started_by(PointerButton::Primary) && !space {
            let press = ctx.input(|i| i.pointer.press_origin()).unwrap_or_default();
            match self.node_hit(press, tf) {
                Some(Hit::Node(n)) => {
                    self.node_edit.selected = Some(n);
                    let targets = self.node_snap_targets(Some(n));
                    self.checkpoint();
                    self.drag = Drag::NodeMove { node: n, targets };
                }
                Some(Hit::Handle(h)) => {
                    let targets = self.node_snap_targets(None);
                    self.checkpoint();
                    self.drag = Drag::NodeHandle { handle: h, targets };
                }
                None => self.node_pick(tf.to_doc(press), tol),
            }
        }

        if resp.dragged_by(PointerButton::Primary)
            && !space
            && let Some(cur) = resp.interact_pointer_pos()
        {
            let raw = tf.to_doc(cur);
            match std::mem::replace(&mut self.drag, Drag::None) {
                Drag::NodeMove { node, targets } => {
                    let p = self.snap_to(raw, &targets, snap_tol);
                    self.edit_target(|path, inv| nodes::move_node(path, node, inv.apply(p)));
                    ctx.set_cursor_icon(CursorIcon::Grabbing);
                    self.drag = Drag::NodeMove { node, targets };
                }
                Drag::NodeHandle { handle, targets } => {
                    let p = self.snap_to(raw, &targets, snap_tol);
                    self.edit_target(|path, inv| nodes::move_handle(path, handle, inv.apply(p)));
                    ctx.set_cursor_icon(CursorIcon::Grabbing);
                    self.drag = Drag::NodeHandle { handle, targets };
                }
                other => self.drag = other,
            }
        }

        if resp.drag_stopped() && matches!(self.drag, Drag::NodeMove { .. } | Drag::NodeHandle { .. }) {
            self.drag = Drag::None;
            self.history.discard_if_unchanged(&self.doc);
            self.mark_changed();
        }

        if let Some(pos) = resp.interact_pointer_pos() {
            if resp.double_clicked() {
                self.insert_node_at(tf.to_doc(pos), tol);
            } else if resp.clicked() {
                match self.node_hit(pos, tf) {
                    Some(Hit::Node(n)) => self.node_edit.selected = Some(n),
                    Some(Hit::Handle(_)) => {}
                    None => {
                        // Klick auf den Zielpfad selbst behält die Bearbeitung bei
                        let p = tf.to_doc(pos);
                        if self.hit_test(p, tol) != self.node_edit.target || self.node_edit.target.is_none() {
                            self.node_pick(p, tol);
                        } else {
                            self.node_edit.selected = None;
                        }
                    }
                }
            }
        }

        resp.context_menu(|ui| {
            node_buttons(self, ui, true);
        });
    }

    fn insert_node_at(&mut self, p: Point, tol: f64) {
        let Some((path, transform)) = self.node_target() else { return };
        let Some(inv) = transform.inverse() else { return };
        let local_tol = tol / transform.mean_scale().max(1e-9);
        let Some((sub, seg, t, d)) = nodes::nearest_on_path(path, inv.apply(p)) else { return };
        if d > local_tol {
            return;
        }
        self.checkpoint();
        let mut inserted = None;
        self.edit_target(|path, _| inserted = nodes::insert_node(path, sub, seg, t));
        self.node_edit.selected = inserted;
        self.mark_changed();
    }

    /// Wendet eine Knotenoperation auf den gewählten Knoten an (mit Rückgängig-Schritt).
    fn node_op(&mut self, f: impl FnOnce(&mut VectorPath, NodeRef)) {
        let Some(n) = self.node_edit.selected else { return };
        self.checkpoint();
        self.edit_target(|path, _| f(path, n));
        self.history.discard_if_unchanged(&self.doc);
        self.mark_changed();
    }

    /// Rundet bzw. fast die Ecke am gewählten Knoten ab (Werte aus „Ecken abrunden“).
    fn round_selected_node(&mut self, style: vl_core::corners::CornerStyle) {
        let Some(n) = self.node_edit.selected else { return };
        let size = self.shape_dialogs.corner_size;
        let Some((path, t)) = self.node_target() else { return };
        let Some(inv) = t.inverse() else { return };
        let (out, count) = vl_core::corners::round_corners(&path.transformed(&t), style, size, 0.5, Some(&[n]));
        if count == 0 {
            self.toast(crate::app::ToastKind::Info, tr("Hier liegt keine Ecke zwischen zwei geraden Linien."));
            return;
        }
        let out = out.transformed(&inv);
        self.checkpoint();
        self.edit_target(|p, _| *p = out);
        // Die Knotennummern haben sich verschoben
        self.node_edit.selected = None;
        self.mark_changed();
    }

    /// Trennt den Pfad am gewählten Knoten auf.
    fn break_selected_node(&mut self) {
        let Some(n) = self.node_edit.selected else { return };
        let mut result = None;
        self.checkpoint();
        self.edit_target(|p, _| result = nodes::break_at_node(p, n));
        if result.is_none() {
            self.history.discard_if_unchanged(&self.doc);
            self.toast(crate::app::ToastKind::Info, tr("Am Ende eines offenen Pfades gibt es nichts aufzutrennen."));
            return;
        }
        self.node_edit.selected = result;
        self.mark_changed();
    }

    /// Macht den gewählten Knoten glatt (Griffe auf einer Geraden).
    fn smooth_selected_node(&mut self) {
        let Some(n) = self.node_edit.selected else { return };
        let mut done = false;
        self.checkpoint();
        self.edit_target(|p, _| done = nodes::smooth_node(p, n));
        if !done {
            self.history.discard_if_unchanged(&self.doc);
            self.toast(crate::app::ToastKind::Info, tr("Endknoten offener Pfade lassen sich nicht glätten."));
            return;
        }
        self.mark_changed();
    }

    /// Ist der gewählte Knoten ein offenes Pfadende (nur dort lässt sich verlängern)?
    fn selected_is_open_end(&self) -> bool {
        let Some(n) = self.node_edit.selected else { return false };
        self.node_target().is_some_and(|(p, _)| nodes::is_open_end(p, n))
    }

    /// Verlängert das gewählte offene Ende bis zur nächsten sichtbaren Linie (auch des eigenen Pfades).
    fn extend_selected_node(&mut self) {
        let (Some(n), Some(id)) = (self.node_edit.selected, self.node_edit.target) else { return };
        let Some((path, t)) = self.node_target() else { return };
        let Some(inv) = t.inverse() else { return };
        let mut out = path.clone();
        // Hindernisse im lokalen System des Pfades: Geraden bleiben unter affinen Abbildungen Geraden
        let mut obstacles = Vec::new();
        for obj in &self.doc.objects {
            if !self.doc.layers[obj.layer].visible {
                continue;
            }
            let polys = if obj.id == id {
                out.flatten(&vl_core::Affine::IDENTITY, 0.02 / t.mean_scale().max(1e-9))
            } else {
                let Some((polys, _)) = self.geom.local(obj) else { continue };
                let to_local = obj.transform.then(&inv);
                polys
                    .iter()
                    .map(|p| vl_core::Polyline::new(p.points.iter().map(|q| to_local.apply(*q)).collect(), p.closed))
                    .collect()
            };
            obstacles.extend(polys);
        }
        let Some(end) = nodes::extend_end(&mut out, n, &obstacles) else {
            self.toast(crate::app::ToastKind::Info, tr("In Verlängerung des Endes liegt keine Linie."));
            return;
        };
        self.checkpoint();
        self.edit_target(|p, _| *p = out);
        self.node_edit.selected = Some(end);
        self.mark_changed();
    }

    pub(crate) fn delete_selected_node(&mut self) {
        let Some(n) = self.node_edit.selected else { return };
        self.node_op(nodes::delete_node);
        // Der Nachfolger rückt auf denselben Index nach; bleibt er gültig, bleibt er gewählt.
        self.node_edit.selected = Some(n);
        let path_empty = self.node_target().is_none_or(|(p, _)| p.subpaths.is_empty());
        if path_empty && let Some(id) = self.node_edit.target {
            // Ohne Knoten bleibt kein sinnvolles Objekt übrig
            self.doc.objects.retain(|o| o.id != id);
            self.selection.retain(|s| *s != id);
            self.node_edit = NodeEdit::default();
        }
        self.node_sync();
    }
}

fn node_buttons(app: &mut VectorLumeApp, ui: &mut Ui, close: bool) {
    let sel = app.node_edit.selected.is_some();
    // In der Werkzeugleiste nur Symbole (Name und Erklärung im Tooltip), im Kontextmenü mit Text
    let button = |ui: &mut Ui, enabled: bool, glyph: &str, text: &str, tip: &str, why_not: &str| {
        let label = if close { format!("{glyph} {text}") } else { glyph.to_string() };
        let r = ui
            .add_enabled(enabled, egui::Button::new(label))
            .on_hover_text(format!(
                "{text}
{tip}"
            ))
            .on_disabled_hover_text(format!(
                "{text}
{why_not}"
            ));
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, text));
        let clicked = r.clicked();
        if clicked && close {
            ui.close();
        }
        clicked
    };
    let pick = tr("Zuerst einen Knoten anklicken");
    let clicked = |ui: &mut Ui, glyph: &str, text: &str, tip: &str| button(ui, sel, glyph, text, tip, pick);
    if clicked(ui, icon::TRASH, tr("Knoten löschen"), tr("Gewählten Knoten entfernen (Entf)")) {
        app.delete_selected_node();
    }
    if clicked(ui, icon::LINE_SEGMENT, tr("Zur Linie"), tr("Segment vor dem Knoten in eine gerade Linie umwandeln")) {
        app.node_op(|p, n| nodes::convert_segment(p, n, false));
    }
    if clicked(ui, icon::BEZIER_CURVE, tr("Zur Kurve"), tr("Segment vor dem Knoten in eine Bézierkurve umwandeln")) {
        app.node_op(|p, n| nodes::convert_segment(p, n, true));
    }
    if clicked(ui, icon::LINK_BREAK, tr("Öffnen/Schließen"), tr("Teilpfad des Knotens öffnen bzw. schließen")) {
        app.node_op(|p, n| nodes::toggle_closed(p, n.sub));
    }
    if clicked(
        ui,
        icon::SCISSORS,
        tr("Auftrennen"),
        tr("Pfad am Knoten auftrennen: geschlossene Formen werden dort geöffnet, offene in zwei Stücke geteilt"),
    ) {
        app.break_selected_node();
    }
    if clicked(ui, icon::WAVE_SINE, tr("Glätten"), tr("Knoten glätten: beide Kurvengriffe auf eine Gerade legen")) {
        app.smooth_selected_node();
    }
    if button(
        ui,
        app.selected_is_open_end(),
        icon::ARROW_LINE_RIGHT,
        tr("Verlängern"),
        tr("Offenes Pfadende geradlinig bis zur nächsten Linie verlängern"),
        tr("Ein Ende eines offenen Pfades wählen"),
    ) {
        app.extend_selected_node();
    }
    if clicked(ui, icon::FRAME_CORNERS, tr("Abrunden"), tr("Ecke am Knoten mit dem eingestellten Radius abrunden")) {
        app.round_selected_node(vl_core::corners::CornerStyle::Round);
    }
    if clicked(ui, icon::POLYGON, tr("Abschrägen"), tr("Ecke am Knoten mit der eingestellten Länge abschrägen")) {
        app.round_selected_node(vl_core::corners::CornerStyle::Chamfer);
    }
}

/// Werkzeugoptionen in der oberen Leiste.
pub fn options(app: &mut VectorLumeApp, ui: &mut Ui) {
    node_buttons(app, ui, false);
    ui.add(
        egui::DragValue::new(&mut app.shape_dialogs.corner_size)
            .speed(0.05)
            .range(0.01..=500.0)
            .max_decimals(2)
            .prefix("r ")
            .mm(),
    )
    .on_hover_text(tr("Radius bzw. Fasenlänge für „Abrunden“ und „Abschrägen“"));
    let hint = if app.node_edit.target.is_none() {
        tr("Pfad anklicken, um seine Knoten zu bearbeiten")
    } else {
        tr("Ziehen = verschieben · Doppelklick auf Linie = Knoten einfügen · Entf = Knoten löschen")
    };
    ui.label(RichText::new(hint).small().color(theme::of(ui).text_weak));
}
