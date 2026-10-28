//! Verzerren der Auswahl: Griffe auf der Arbeitsfläche ziehen (Perspektive oder Hülle), Vorschau, Anwenden.
//! Texte und Grundformen werden dabei zu Kurven; Bilder bleiben unverändert.

use crate::app::{ToastKind, VectorLumeApp};
use crate::canvas::{HANDLE, ViewTf};
use crate::i18n::{tr, trs};
use crate::theme;
use egui::{Align2, Button, CursorIcon, PointerButton, Pos2, RichText, Shape, Stroke, vec2};
use egui_phosphor::regular as icon;
use vl_core::warp::{Warp, WarpKind};
use vl_core::{Affine, ObjectId, Point, Polyline, Rect, VectorPath};

/// Höchste Abweichung der verzerrten Kurven (mm).
const TOLERANCE: f64 = 0.01;

pub struct WarpSession {
    pub warp: Warp,
    /// Pfade der beteiligten Objekte in Dokumentkoordinaten.
    sources: Vec<(ObjectId, VectorPath)>,
    selection: Vec<ObjectId>,
    revision: u64,
    dragging: Option<usize>,
    /// Vorschau (Polylinien) zu den Griffen, mit denen sie berechnet wurde.
    preview: Option<([Point; 8], WarpKind, Vec<Polyline>)>,
}

impl WarpSession {
    fn preview(&mut self) -> &[Polyline] {
        let key = (self.warp.handles, self.warp.kind);
        if self.preview.as_ref().is_none_or(|(h, k, _)| (*h, *k) != key) {
            let polys = self
                .sources
                .iter()
                .flat_map(|(_, p)| self.warp.apply(p, 0.05).flatten(&Affine::IDENTITY, 0.1))
                .collect();
            self.preview = Some((key.0, key.1, polys));
        }
        &self.preview.as_ref().expect("gerade berechnet").2
    }
}

impl VectorLumeApp {
    /// Startet das Verzerren der ausgewählten Formen und Texte.
    pub fn start_warp(&mut self, kind: WarpKind) {
        let mut sources = Vec::new();
        let mut images = 0;
        for o in self.selection.iter().filter_map(|id| self.doc.get(*id)) {
            match &o.shape {
                vl_core::Shape::Path(p) if o.transform.inverse().is_some() => {
                    sources.push((o.id, p.transformed(&o.transform)))
                }
                vl_core::Shape::Path(_) => {}
                vl_core::Shape::Image(_) => images += 1,
            }
        }
        let bounds = sources.iter().fold(Rect::EMPTY, |r, (_, p)| r.union(&p.bounds(&Affine::IDENTITY)));
        if sources.is_empty() {
            self.toast(ToastKind::Info, tr("Zum Verzerren Formen oder Texte auswählen."));
            return;
        }
        if bounds.width() < 0.01 || bounds.height() < 0.01 {
            self.toast(ToastKind::Info, tr("Die Auswahl ist zu flach zum Verzerren."));
            return;
        }
        if images > 0 {
            self.toast(ToastKind::Info, tr("Bilder werden nicht verzerrt und bleiben unverändert."));
        }
        self.tool = crate::app::Tool::Select;
        self.warp = Some(WarpSession {
            warp: Warp::new(bounds, kind),
            sources,
            selection: self.selection.clone(),
            revision: self.revision,
            dragging: None,
            preview: None,
        });
    }

    pub fn cancel_warp(&mut self) {
        self.warp = None;
    }

    /// Übernimmt die Verzerrung (ein Rückgängig-Schritt).
    pub fn apply_warp(&mut self) {
        let Some(session) = self.warp.take() else { return };
        if session.warp.is_identity() {
            return;
        }
        if !session.warp.is_valid() {
            self.toast(ToastKind::Error, tr("Die Griffe überkreuzen sich – so lässt sich nicht verzerren."));
            self.warp = Some(session);
            return;
        }
        self.checkpoint();
        let mut count = 0;
        for (id, path) in &session.sources {
            let Some(o) = self.doc.get_mut(*id) else { continue };
            let Some(inv) = o.transform.inverse() else { continue };
            o.shape = vl_core::Shape::Path(session.warp.apply(path, TOLERANCE).transformed(&inv));
            o.text = None;
            o.primitive = None;
            self.geom.invalidate(*id);
            count += 1;
        }
        self.mark_changed();
        self.toast(ToastKind::Success, trs(&format!("{count} Objekte verzerrt.")));
    }

    /// Beendet die Sitzung, wenn sich Auswahl oder Dokument anderweitig geändert haben.
    fn warp_still_valid(&mut self) -> bool {
        let valid = self.tool == crate::app::Tool::Select
            && self.warp.as_ref().is_some_and(|s| s.revision == self.revision && s.selection == self.selection);
        if !valid {
            self.warp = None;
        }
        valid
    }

    /// Maus auf der Arbeitsfläche während des Verzerrens; `true`, wenn die Eingabe verbraucht wurde.
    pub(crate) fn warp_interaction(
        &mut self,
        ctx: &egui::Context,
        resp: &egui::Response,
        tf: &ViewTf,
        space: bool,
    ) -> bool {
        if self.warp.is_none() || !self.warp_still_valid() {
            return false;
        }
        if space {
            return true;
        }
        let snapped = |app: &Self, p: Pos2| app.snap(tf.to_doc(p));
        let Some(session) = self.warp.as_ref() else { return true };
        let near = |p: Pos2| {
            (0..session.warp.handle_count())
                .map(|i| (i, tf.to_screen(session.warp.handles[i]).distance(p)))
                .filter(|(_, d)| *d <= HANDLE + 4.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
        };
        if let Some(hover) = resp.hover_pos()
            && (session.dragging.is_some() || near(hover).is_some())
        {
            ctx.set_cursor_icon(CursorIcon::Grab);
        }
        if resp.drag_started_by(PointerButton::Primary) {
            let press = ctx.input(|i| i.pointer.press_origin()).unwrap_or_default();
            let hit = near(press);
            if let Some(s) = self.warp.as_mut() {
                s.dragging = hit;
            }
        }
        if resp.dragged_by(PointerButton::Primary)
            && let Some(cur) = resp.interact_pointer_pos()
        {
            let p = snapped(self, cur);
            if let Some(s) = self.warp.as_mut()
                && let Some(i) = s.dragging
            {
                s.warp.handles[i] = p;
                ctx.set_cursor_icon(CursorIcon::Grabbing);
            }
        }
        if resp.drag_stopped()
            && let Some(s) = self.warp.as_mut()
        {
            s.dragging = None;
        }
        // Klicks und Ziehen außerhalb der Griffe sollen die Auswahl nicht ändern
        true
    }

    /// Vorschau, Umriss und Griffe.
    pub(crate) fn draw_warp(&mut self, painter: &egui::Painter, tf: &ViewTf, pal: &theme::Palette) {
        if self.warp.is_none() || !self.warp_still_valid() {
            return;
        }
        let Some(session) = self.warp.as_mut() else { return };
        let valid = session.warp.is_valid();
        let color = if valid { pal.warning } else { pal.danger };
        // Hilfsraster zeigt die Verformung
        let faint = Stroke::new(1.0, pal.selection.gamma_multiply(0.35));
        for k in 1..4 {
            let c = k as f64 / 4.0;
            let col: Vec<Pos2> = (0..=16).map(|i| tf.to_screen(session.warp.map_unit(c, i as f64 / 16.0))).collect();
            let row: Vec<Pos2> = (0..=16).map(|i| tf.to_screen(session.warp.map_unit(i as f64 / 16.0, c))).collect();
            painter.add(Shape::line(col, faint));
            painter.add(Shape::line(row, faint));
        }
        for poly in session.preview() {
            let pts: Vec<Pos2> = poly.points.iter().map(|p| tf.to_screen(*p)).collect();
            painter.add(crate::canvas::outline(pts, poly.closed, Stroke::new(1.2, color)));
        }
        let outline: Vec<Pos2> = session.warp.outline(24).iter().map(|p| tf.to_screen(*p)).collect();
        painter.extend(Shape::dashed_line(
            &outline.iter().chain(outline.first()).copied().collect::<Vec<_>>(),
            Stroke::new(1.0, pal.selection),
            5.0,
            4.0,
        ));
        for i in 0..session.warp.handle_count() {
            let p = tf.to_screen(session.warp.handles[i]);
            let active = session.dragging == Some(i);
            let fill = if active { pal.accent } else { pal.panel };
            if i < 4 {
                let r = egui::Rect::from_center_size(p, vec2(HANDLE * 2.0, HANDLE * 2.0));
                painter.rect_filled(r, 2.0, fill);
                painter.rect_stroke(r, 2.0, Stroke::new(1.5, pal.selection), egui::StrokeKind::Middle);
            } else {
                painter.circle_filled(p, HANDLE, fill);
                painter.circle_stroke(p, HANDLE, Stroke::new(1.5, pal.selection));
            }
        }
    }
}

/// Steuerfenster, solange verzerrt wird.
pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    let Some(session) = app.warp.as_mut() else { return };
    let mut open = true;
    let mut apply = false;
    let valid = session.warp.is_valid();
    egui::Window::new(trs(&format!("{} Verzerren", icon::PERSPECTIVE)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(300.0)
        .pivot(Align2::RIGHT_TOP)
        .default_pos(ctx.content_rect().right_top() + vec2(-400.0, 96.0))
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            ui.horizontal(|ui| {
                for kind in WarpKind::ALL {
                    if ui.selectable_label(session.warp.kind == kind, tr(kind.label())).clicked() {
                        session.warp.set_kind(kind);
                    }
                }
            });
            ui.label(
                RichText::new(match session.warp.kind {
                    WarpKind::Perspective => {
                        tr("Ecken ziehen: Geraden bleiben gerade, z. B. für schräg gesehene Flächen.")
                    }
                    WarpKind::Envelope => {
                        tr("Ecken und Kantenmitten ziehen: Kanten werden gebogen, z. B. für Wölbungen.")
                    }
                })
                .small()
                .color(pal.text_weak),
            );
            ui.label(
                RichText::new(tr("Texte und Grundformen werden dabei zu Kurven. Enter übernimmt, Esc bricht ab."))
                    .small()
                    .color(pal.text_weak),
            );
            if !valid {
                ui.colored_label(pal.danger, tr("Die Griffe überkreuzen sich – so lässt sich nicht verzerren."));
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button(trs(&format!("{} Zurücksetzen", icon::ARROW_COUNTER_CLOCKWISE))).clicked() {
                    session.warp.reset();
                }
                let ok = valid && !session.warp.is_identity();
                if ui.add_enabled(ok, Button::new(trs(&format!("{} Anwenden", icon::CHECK)))).clicked() {
                    apply = true;
                }
            });
        });
    if apply {
        app.apply_warp();
    } else if !open {
        app.cancel_warp();
    }
}
