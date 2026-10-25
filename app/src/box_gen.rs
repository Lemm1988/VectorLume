//! Box-Generator: Box mit Fingerzinken aus Maßen, Materialstärke und Zinkenbreite erzeugen. Die Platten werden
//! nebeneinander auf der Arbeitsfläche ausgelegt und als Gruppe auf die aktuelle Ebene gelegt.

use crate::app::{ToastKind, VectorLumeApp};
use crate::i18n::{tr, trs};
use crate::theme;
use crate::units::{self, UnitField};
use egui::{Align2, Button, DragValue, RichText, Sense, Shape, Stroke, vec2};
use egui_phosphor::regular as icon;
use vl_core::boxgen::{BoxSpec, Panel};
use vl_core::{Point, Polyline, Rect, VectorPath};

/// Abstand der Platten beim Auslegen (mm).
const GAP: f64 = 4.0;
/// Abstand zum Rand der Arbeitsfläche (mm).
const MARGIN: f64 = 5.0;

#[derive(Default)]
pub struct BoxGen {
    pub open: bool,
    pub spec: BoxSpec,
}

/// Platten zeilenweise (bis zur Breite `max_w`) auslegen; liefert die Umrisse an ihrer Stelle, links oben bei 0/0.
fn laid_out(spec: &BoxSpec, max_w: f64) -> Vec<(Panel, Polyline)> {
    let (mut x, mut y, mut row_h) = (0.0, 0.0, 0.0f64);
    let mut out = Vec::new();
    for (panel, poly) in spec.panels() {
        let b = poly.bounds();
        if x > 0.0 && x + b.width() > max_w {
            (x, y, row_h) = (0.0, y + row_h + GAP, 0.0);
        }
        let at = Point::new(x, y) - b.min;
        out.push((panel, Polyline::new(poly.points.iter().map(|p| *p + at).collect(), true)));
        x += b.width() + GAP;
        row_h = row_h.max(b.height());
    }
    out
}

impl VectorLumeApp {
    pub(crate) fn insert_box(&mut self) {
        let spec = self.box_gen.spec;
        if let Err(e) = spec.check() {
            self.toast(ToastKind::Error, tr(e));
            return;
        }
        let bed = self.settings.profile.bed_rect();
        let panels = laid_out(&spec, bed.width() - 2.0 * MARGIN);
        let offset = bed.min + Point::new(MARGIN, MARGIN);
        self.checkpoint();
        let mut ids = Vec::new();
        for (panel, poly) in &panels {
            let pts: Vec<Point> = poly.points.iter().map(|p| *p + offset).collect();
            ids.push(self.doc.add_path(tr(panel.label()), self.current_layer, VectorPath::polygon(&pts, true)));
        }
        self.doc.group(&ids);
        self.selection = ids;
        self.mark_changed();
        self.toast(
            ToastKind::Success,
            trs(&format!(
                "Box aus {} Platten erzeugt. Für einen strammen Sitz die Schnittbreite der Ebene einstellen.",
                panels.len()
            )),
        );
    }
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.box_gen.open {
        return;
    }
    let mut open = true;
    let mut create = false;
    let bed_w = app.settings.profile.bed_rect().width();
    egui::Window::new(trs(&format!("{} Box-Generator", icon::PACKAGE)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(420.0)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let s = &mut app.box_gen.spec;
            egui::Grid::new("box_gen").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                ui.label(tr("Maße gelten"));
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut s.inner, false, tr("außen"));
                    ui.selectable_value(&mut s.inner, true, tr("innen"));
                });
                ui.end_row();
                for (label, v) in [(tr("Breite"), &mut s.width), (tr("Tiefe"), &mut s.depth), (tr("Höhe"), &mut s.height)] {
                    ui.label(label);
                    ui.add(DragValue::new(v).speed(0.5).range(5.0..=2000.0).mm());
                    ui.end_row();
                }
                ui.label(tr("Materialstärke"));
                ui.add(DragValue::new(&mut s.thickness).speed(0.05).range(0.5..=30.0).mm());
                ui.end_row();
                ui.label(tr("Zinkenbreite"));
                ui.add(DragValue::new(&mut s.finger).speed(0.1).range(1.0..=200.0).mm()).on_hover_text(tr(
                    "Gewünschte Breite; je Kante wird auf eine ungerade Anzahl gleich breiter Zinken gerundet, damit beide Enden gleich aussehen.",
                ));
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut s.open_top, tr("Ohne Deckel"));
                ui.end_row();
            });
            let spec = *s;
            let (x, y, z) = spec.outer();
            let num = |v: f64| units::num(v, 1);
            ui.label(
                RichText::new(trs(&format!(
                    "Außenmaß {} × {} × {} {} · Zinken je Kante {} / {} / {}",
                    num(x),
                    num(y),
                    num(z),
                    units::unit(),
                    spec.sections(x),
                    spec.sections(y),
                    spec.sections(z)
                )))
                .small()
                .color(pal.text_weak),
            );
            let check = spec.check();
            // Vorschau der ausgelegten Platten
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().max(380.0), 190.0), Sense::hover());
            ui.painter().rect_filled(rect, 6.0, pal.canvas);
            if check.is_ok() {
                let panels = laid_out(&spec, bed_w - 2.0 * MARGIN);
                let mut all = Rect::EMPTY;
                for (_, p) in &panels {
                    all = all.union(&p.bounds());
                }
                let inner = rect.shrink(10.0);
                let scale = (inner.width() as f64 / all.width()).min(inner.height() as f64 / all.height());
                let to_screen = |p: &Point| {
                    inner.min + vec2(((p.x - all.min.x) * scale) as f32, ((p.y - all.min.y) * scale) as f32)
                };
                for (_, p) in &panels {
                    let mut pts: Vec<egui::Pos2> = p.points.iter().map(to_screen).collect();
                    pts.push(pts[0]);
                    ui.painter().add(Shape::line(pts, Stroke::new(1.2, pal.accent)));
                }
            }
            if let Err(e) = check {
                ui.label(RichText::new(tr(e)).color(pal.danger));
            }
            ui.label(
                RichText::new(tr(
                    "Tipp: Schnittbreite der Ebene einstellen, damit die Zinken stramm sitzen – am besten erst ein Probestück schneiden.",
                ))
                .small()
                .color(pal.text_weak),
            );
            ui.add_space(4.0);
            if ui
                .add_enabled(check.is_ok(), Button::new(trs(&format!("{} Box erzeugen", icon::PACKAGE))))
                .clicked()
            {
                create = true;
            }
        });
    if create {
        app.insert_box();
        app.box_gen.open = false;
    }
    if !open {
        app.box_gen.open = false;
    }
}
