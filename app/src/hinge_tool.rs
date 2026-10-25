//! Biegescharnier: die ausgewählten geschlossenen Formen mit versetzten Schnittreihen füllen, damit sich das
//! Material dort biegen lässt. Die Schnitte kommen als ein Objekt auf die aktuelle Ebene.

use crate::app::{ToastKind, VectorLumeApp};
use crate::i18n::{tr, trs};
use crate::theme;
use crate::units::{self, UnitField};
use egui::{Align2, Button, DragValue, RichText, Sense, Shape, Stroke, vec2};
use egui_phosphor::regular as icon;
use vl_core::hinge::HingeSpec;
use vl_core::{ObjectId, Polyline, Rect, Shape as DocShape, VectorPath};

pub struct HingeTool {
    pub open: bool,
    pub spec: HingeSpec,
    /// Bereichsformen nach dem Erzeugen entfernen.
    pub remove_region: bool,
}

impl Default for HingeTool {
    fn default() -> Self {
        Self { open: false, spec: HingeSpec::default(), remove_region: true }
    }
}

impl VectorLumeApp {
    /// Geschlossene Konturen der Auswahl (Dokumentkoordinaten) und die beteiligten Objekte.
    fn hinge_region(&self) -> (Vec<Polyline>, Vec<ObjectId>) {
        let mut polys = Vec::new();
        let mut ids = Vec::new();
        for id in self.doc.expand_groups(&self.selection) {
            let Some(o) = self.doc.get(id) else { continue };
            let DocShape::Path(p) = &o.shape else { continue };
            let closed: Vec<Polyline> = p.flatten(&o.transform, 0.02).into_iter().filter(|q| q.closed).collect();
            if !closed.is_empty() {
                polys.extend(closed);
                ids.push(id);
            }
        }
        (polys, ids)
    }

    pub(crate) fn insert_hinge(&mut self) {
        let spec = self.hinge.spec;
        let (region, ids) = self.hinge_region();
        let lines = spec.lines(&region);
        if lines.is_empty() {
            self.toast(ToastKind::Info, tr("Keine geschlossene Form ausgewählt oder Bereich zu klein."));
            return;
        }
        self.checkpoint();
        let mut path = VectorPath::default();
        for l in &lines {
            path.append(VectorPath::polygon(&l.points, false));
        }
        let id = self.doc.add_path(tr("Biegescharnier"), self.current_layer, path);
        if self.hinge.remove_region {
            self.doc.remove(&ids);
        }
        self.selection = vec![id];
        self.mark_changed();
        self.toast(ToastKind::Success, trs(&format!("Biegescharnier mit {} Schnitten erzeugt.", lines.len())));
    }
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.hinge.open {
        return;
    }
    let mut open = true;
    let mut create = false;
    let (region, _) = app.hinge_region();
    egui::Window::new(trs(&format!("{} Biegescharnier", icon::WAVE_SINE)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(400.0)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            ui.label(
                RichText::new(tr(
                    "Füllt die ausgewählten geschlossenen Formen mit versetzten Schnittreihen, damit sich z. B. Sperrholz dort biegen lässt.",
                ))
                .small()
                .color(pal.text_weak),
            );
            let h = &mut app.hinge;
            egui::Grid::new("hinge").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                ui.label(tr("Schnittlänge"));
                ui.add(DragValue::new(&mut h.spec.cut).speed(0.1).range(1.0..=500.0).mm());
                ui.end_row();
                ui.label(tr("Steg"));
                ui.add(DragValue::new(&mut h.spec.bridge).speed(0.05).range(0.5..=50.0).mm())
                    .on_hover_text(tr("Material zwischen zwei Schnitten einer Reihe – kürzer = biegsamer, länger = stabiler"));
                ui.end_row();
                ui.label(tr("Linienabstand"));
                ui.add(DragValue::new(&mut h.spec.spacing).speed(0.05).range(0.3..=20.0).mm())
                    .on_hover_text(tr("Abstand der Schnittreihen – kleiner = biegsamer"));
                ui.end_row();
                ui.label(tr("Richtung"));
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut h.spec.angle, 0.0, tr("senkrecht"));
                    ui.selectable_value(&mut h.spec.angle, 90.0, tr("waagrecht"));
                    ui.add(DragValue::new(&mut h.spec.angle).speed(1.0).range(-90.0..=90.0).suffix("°"));
                });
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut h.remove_region, tr("Bereichsform danach entfernen"));
                ui.end_row();
            });
            let spec = h.spec;
            let lines = if spec.check().is_ok() { spec.lines(&region) } else { Vec::new() };
            // Vorschau
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().max(360.0), 170.0), Sense::hover());
            ui.painter().rect_filled(rect, 6.0, pal.canvas);
            let all = region.iter().map(|p| p.bounds()).fold(Rect::EMPTY, |a, b| a.union(&b));
            if !all.is_empty() {
                let inner = rect.shrink(10.0);
                let scale = (inner.width() as f64 / all.width()).min(inner.height() as f64 / all.height());
                let to_screen = |p: &vl_core::Point| {
                    inner.center()
                        + vec2(
                            ((p.x - all.center().x) * scale) as f32,
                            ((p.y - all.center().y) * scale) as f32,
                        )
                };
                for p in &region {
                    let mut pts: Vec<egui::Pos2> = p.points.iter().map(to_screen).collect();
                    pts.push(pts[0]);
                    ui.painter().add(Shape::line(pts, Stroke::new(1.0, pal.text_weak)));
                }
                for l in &lines {
                    let pts: Vec<egui::Pos2> = l.points.iter().map(to_screen).collect();
                    ui.painter().add(Shape::line(pts, Stroke::new(1.0, pal.accent)));
                }
            }
            match spec.check() {
                Err(e) => {
                    ui.label(RichText::new(tr(e)).color(pal.danger));
                }
                Ok(()) if region.is_empty() => {
                    ui.label(RichText::new(tr("Bitte zuerst eine geschlossene Form als Bereich auswählen.")).color(pal.warning));
                }
                Ok(()) => {
                    let total: f64 = lines.iter().map(|l| l.length()).sum();
                    ui.label(
                        RichText::new(trs(&format!("{} Schnitte, zusammen {}", lines.len(), units::len(total, 0))))
                            .small()
                            .color(pal.text_weak),
                    );
                }
            }
            ui.add_space(4.0);
            if ui.add_enabled(!lines.is_empty(), Button::new(trs(&format!("{} Scharnier erzeugen", icon::WAVE_SINE)))).clicked()
            {
                create = true;
            }
        });
    if create {
        app.insert_hinge();
        app.hinge.open = false;
    }
    if !open {
        app.hinge.open = false;
    }
}
