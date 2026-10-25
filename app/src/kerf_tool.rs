//! Schnittbreite ermitteln: Testkamm erzeugen, nach dem Schneiden den Messwert eingeben und die errechnete
//! Schnittbreite auf die Ebene übernehmen.

use crate::app::{ToastKind, VectorLumeApp};
use crate::i18n::{tr, trs};
use crate::theme;
use crate::units::{self, UnitField};
use egui::{Align2, Button, Color32, DragValue, RichText, Sense, Shape, Stroke, vec2};
use egui_phosphor::regular as icon;
use vl_core::kerftest::{KerfTest, Measured};
use vl_core::{Affine, Point};

/// Abstand zum Rand der Arbeitsfläche (mm).
const MARGIN: f64 = 5.0;

pub struct KerfTool {
    pub open: bool,
    pub spec: KerfTest,
    pub measured: Measured,
    /// Messwert in mm (0 = noch nicht gemessen).
    pub value: f64,
    /// Schnittbreite der Ebene, als der Kamm erzeugt wurde (mm).
    pub cut_kerf: f64,
}

impl Default for KerfTool {
    fn default() -> Self {
        Self { open: false, spec: KerfTest::default(), measured: Measured::Strips, value: 0.0, cut_kerf: 0.0 }
    }
}

impl VectorLumeApp {
    pub(crate) fn open_kerf_test(&mut self) {
        self.kerf.cut_kerf = self.doc.layers[self.current_layer].settings.kerf;
        self.kerf.open = true;
    }

    pub(crate) fn insert_kerf_test(&mut self) {
        let spec = self.kerf.spec;
        if let Err(e) = spec.check() {
            self.toast(ToastKind::Error, tr(e));
            return;
        }
        let bed = self.settings.profile.bed_rect();
        let at = bed.min + Point::new(MARGIN, MARGIN);
        self.checkpoint();
        let id = self.doc.add_path(
            tr("Schnittbreiten-Test"),
            self.current_layer,
            spec.build().transformed(&Affine::translate(at.x, at.y)),
        );
        self.kerf.cut_kerf = self.doc.layers[self.current_layer].settings.kerf;
        self.kerf.value = 0.0;
        self.selection = vec![id];
        self.mark_changed();
        self.toast(ToastKind::Success, tr("Testkamm erzeugt. Schneiden, Streifen herausnehmen und messen."));
    }

    pub(crate) fn apply_kerf(&mut self, kerf: f64) {
        self.checkpoint();
        let layer = &mut self.doc.layers[self.current_layer];
        layer.settings.kerf = kerf;
        let name = layer.name.clone();
        self.mark_changed();
        self.toast(
            ToastKind::Success,
            trs(&format!("Schnittbreite {} {} auf Ebene {name} übernommen.", units::num(kerf, 3), units::unit())),
        );
    }
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.kerf.open {
        return;
    }
    let mut open = true;
    let mut create = false;
    let mut apply = None;
    let layer_name = app.doc.layers[app.current_layer].name.clone();
    egui::Window::new(trs(&format!("{} Schnittbreite ermitteln", icon::RULER)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(400.0)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let k = &mut app.kerf;
            ui.label(RichText::new(tr("1. Testkamm schneiden")).strong());
            egui::Grid::new("kerf_comb").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                ui.label(tr("Streifen"));
                ui.horizontal(|ui| {
                    ui.add(DragValue::new(&mut k.spec.strips).range(2..=50));
                    ui.label("×");
                    ui.add(DragValue::new(&mut k.spec.strip_width).speed(0.1).range(2.0..=100.0).mm());
                });
                ui.end_row();
                ui.label(tr("Länge"));
                ui.add(DragValue::new(&mut k.spec.height).speed(0.5).range(5.0..=200.0).mm());
                ui.end_row();
            });
            let spec = k.spec;
            let check = spec.check();
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().max(360.0), 90.0), Sense::hover());
            ui.painter().rect_filled(rect, 6.0, pal.canvas);
            if check.is_ok() {
                let inner = rect.shrink(10.0);
                let scale = (inner.width() as f64 / spec.width()).min(inner.height() as f64 / spec.height);
                let off = inner.center() - vec2((spec.width() * scale) as f32 / 2.0, (spec.height * scale) as f32 / 2.0);
                for q in spec.build().flatten(&Affine::IDENTITY, 0.1) {
                    let mut pts: Vec<egui::Pos2> =
                        q.points.iter().map(|p| off + vec2((p.x * scale) as f32, (p.y * scale) as f32)).collect();
                    if q.closed {
                        pts.push(pts[0]);
                    }
                    ui.painter().add(Shape::line(pts, Stroke::new(1.2, pal.accent)));
                }
            }
            if let Err(e) = check {
                ui.label(RichText::new(tr(e)).color(pal.danger));
            }
            if ui.add_enabled(check.is_ok(), Button::new(trs(&format!("{} Testkamm erzeugen", icon::PLUS)))).clicked() {
                create = true;
            }
            ui.label(
                RichText::new(trs(&format!(
                    "Auf der aktuellen Ebene {layer_name} mit den Werten schneiden, die später verwendet werden – das Material muss ganz durchtrennt werden."
                )))
                .small()
                .color(pal.text_weak),
            );
            ui.separator();
            ui.label(RichText::new(tr("2. Messen")).strong());
            ui.horizontal(|ui| {
                ui.selectable_value(&mut k.measured, Measured::Strips, tr("Streifen zusammengeschoben"));
                ui.selectable_value(&mut k.measured, Measured::Gap, tr("Spalt im Rahmen"));
            });
            ui.label(
                RichText::new(match k.measured {
                    Measured::Strips => tr("Alle Streifen dicht aneinanderlegen und quer über die Streifen messen."),
                    Measured::Gap => tr("Alle Streifen im Rahmen an eine Seite schieben und den Spalt auf der anderen messen."),
                })
                .small()
                .color(pal.text_weak),
            );
            egui::Grid::new("kerf_measure").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                ui.label(tr("Messwert"));
                ui.add(DragValue::new(&mut k.value).speed(0.01).range(0.0..=5000.0).max_decimals(2).mm());
                ui.end_row();
                if k.measured == Measured::Strips {
                    ui.label(tr("Schnittbreite beim Test"));
                    ui.add(DragValue::new(&mut k.cut_kerf).speed(0.01).range(0.0..=2.0).max_decimals(3).mm())
                        .on_hover_text(tr("War auf der Ebene beim Schneiden des Tests schon eine Schnittbreite eingestellt, wird sie hier berücksichtigt."));
                    ui.end_row();
                }
            });
            ui.separator();
            match spec.kerf(k.measured, k.value, k.cut_kerf) {
                Ok(kerf) => {
                    ui.label(
                        RichText::new(trs(&format!("Schnittbreite: {} {}", units::num(kerf, 3), units::unit())))
                            .size(16.0)
                            .strong(),
                    );
                    if ui
                        .add(
                            Button::new(
                                RichText::new(trs(&format!("Auf Ebene {layer_name} übernehmen"))).color(Color32::WHITE),
                            )
                            .fill(pal.accent),
                        )
                        .clicked()
                    {
                        apply = Some(kerf);
                    }
                }
                Err(e) => {
                    let color = if k.value > 0.0 { pal.danger } else { pal.text_weak };
                    ui.label(RichText::new(tr(e)).color(color));
                }
            }
        });
    if create {
        app.insert_kerf_test();
    }
    if let Some(kerf) = apply {
        app.apply_kerf(kerf);
    }
    if !open {
        app.kerf.open = false;
    }
}
