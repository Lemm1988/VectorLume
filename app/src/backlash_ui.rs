//! Umkehrspiel messen: Nonius-Muster lasern, ablesen, Ausgleich übernehmen (siehe `vl_cam::backlash_test`).

use crate::app::{ToastKind, VectorLumeApp};
use crate::i18n::{tr, trs};
use crate::theme;
use crate::units::UnitField;
use egui::{Align2, Button, Color32, DragValue, RichText, Sense, Stroke, vec2};
use egui_phosphor::regular as icon;
use std::sync::Arc;
use vl_cam::backlash_test::{STEPS, corrected};
use vl_cam::{BacklashTest, Job, JobSettings, MachineProfile, Op};

/// Ablesung einer Achse: Stelle, an der sich die Striche decken.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
    /// Auf der „+“-Seite (mehr Ausgleich) statt der „−“-Seite.
    pub plus: bool,
    /// Striche von der Mitte aus (0 = Mittelstriche decken sich).
    pub steps: u32,
}

impl Reading {
    fn apply(self, current: f64) -> f64 {
        corrected(current, self.plus, self.steps)
    }
}

#[derive(Default)]
pub struct BacklashUi {
    pub open: bool,
    pub spec: BacklashTest,
    pub x: Reading,
    pub y: Reading,
    job: Option<Arc<Job>>,
    error: Option<String>,
    built: Option<(BacklashTest, MachineProfile, JobSettings)>,
}

impl VectorLumeApp {
    pub(crate) fn open_backlash_test(&mut self) {
        self.backlash.open = true;
        self.backlash.built = None;
    }

    fn refresh_backlash_job(&mut self) {
        let key = (self.backlash.spec.clone(), self.settings.profile.clone(), self.settings.job.clone());
        if self.backlash.built.as_ref() == Some(&key) {
            return;
        }
        match key.0.build(&key.1, &key.2) {
            Ok(job) => {
                self.backlash.job = Some(Arc::new(job));
                self.backlash.error = None;
            }
            Err(e) => {
                self.backlash.job = None;
                self.backlash.error = Some(e);
            }
        }
        self.backlash.built = Some(key);
    }

    /// Ablesung in das Geräteprofil übernehmen.
    pub(crate) fn apply_backlash_reading(&mut self) {
        let (x, y) = (self.backlash.x, self.backlash.y);
        let p = &mut self.settings.profile;
        p.backlash_x = x.apply(p.backlash_x);
        p.backlash_y = y.apply(p.backlash_y);
        let msg = trs(&format!(
            "Umkehrspiel übernommen: X {} mm, Y {} mm. Zur Kontrolle das Muster noch einmal lasern – dann decken sich die \
             Mittelstriche.",
            crate::units::trim(p.backlash_x, 3),
            crate::units::trim(p.backlash_y, 3)
        ));
        self.settings.save();
        self.revision += 1;
        self.backlash.x = Reading::default();
        self.backlash.y = Reading::default();
        self.toast(ToastKind::Success, msg);
    }
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.backlash.open {
        return;
    }
    app.refresh_backlash_job();
    let mut open = true;
    let mut spec = app.backlash.spec.clone();
    let (mut rx, mut ry) = (app.backlash.x, app.backlash.y);
    let mut action: Option<&str> = None;
    let (bx, by) = (app.settings.profile.backlash_x, app.settings.profile.backlash_y);
    egui::Window::new(trs(&format!("{} Umkehrspiel messen", icon::ARROWS_LEFT_RIGHT)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(560.0)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            ui.set_max_width(560.0);
            ui.label(RichText::new(tr(
                "Riemen und Spindeln haben etwas Spiel: Wechselt eine Achse die Richtung, bleibt der Kopf um diesen Betrag \
                 zurück. Schlitze werden dann breiter oder schmaler als gezeichnet und Kreise unrund. Das Muster misst das \
                 Spiel wie der Nonius eines Messschiebers auf 0,05 mm genau.",
            ))
            .color(pal.text_weak));
            ui.add_space(6.0);
            egui::Grid::new("backlash_grid").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label(tr("Geschwindigkeit"));
                ui.add(DragValue::new(&mut spec.speed).speed(10.0).range(10.0..=50000.0).mm_per_min())
                    .on_hover_text(tr("Wie beim Schneiden üblich; das Spiel hängt kaum von der Geschwindigkeit ab."));
                ui.end_row();
                ui.label(tr("Leistung"));
                ui.add(DragValue::new(&mut spec.power).speed(0.5).range(0.1..=100.0).suffix(" %"))
                    .on_hover_text(tr("Nur so viel, dass dünne, gut sichtbare Striche entstehen."));
                ui.end_row();
                ui.label(tr("Position"));
                ui.horizontal(|ui| {
                    ui.add(DragValue::new(&mut spec.position.x).speed(1.0).range(0.0..=3000.0).prefix("X ").mm());
                    ui.add(DragValue::new(&mut spec.position.y).speed(1.0).range(0.0..=3000.0).prefix("Y ").mm());
                })
                .response
                .on_hover_text(tr("Ecke des Musters am Maschinen-Nullpunkt (Maschinenkoordinaten)"));
                ui.end_row();
                ui.label(tr("Aktueller Ausgleich"));
                ui.label(format!("X {} mm   Y {} mm", crate::units::trim(bx, 3), crate::units::trim(by, 3)));
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.horizontal_top(|ui| {
                if let Some(job) = &app.backlash.job {
                    preview(ui, job, vec2(220.0, 110.0));
                }
                ui.vertical(|ui| {
                    ui.label(RichText::new(tr("Ablesen")).strong());
                    ui.label(RichText::new(tr(
                        "Je Achse die Stelle suchen, an der sich ein oberer und ein unterer Strich genau decken. Striche \
                         von der langen Mitte bis dorthin zählen und die Seite wählen, auf der sie liegt („+“ oder „−“ \
                         neben dem Muster).",
                    ))
                    .small()
                    .color(pal.text_weak));
                    ui.add_space(4.0);
                    for (axis, r, current) in [("X", &mut rx, bx), ("Y", &mut ry, by)] {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(axis).strong());
                            ui.add(DragValue::new(&mut r.steps).range(0..=STEPS as u32).suffix(tr(" Striche")));
                            ui.add_enabled_ui(r.steps > 0, |ui| {
                                ui.radio_value(&mut r.plus, true, tr("Seite „+“"));
                                ui.radio_value(&mut r.plus, false, tr("Seite „−“"));
                            });
                            let new = r.apply(current);
                            let text = if r.steps == 0 {
                                tr("stimmt").to_string()
                            } else {
                                trs(&format!("→ {} mm", crate::units::trim(new, 3)))
                            };
                            ui.label(RichText::new(text).color(pal.text_weak));
                        });
                    }
                    if r_at_limit(rx) || r_at_limit(ry) {
                        ui.label(RichText::new(tr(
                            "Am Ende der Skala: Das Spiel ist womöglich größer. Übernehmen und das Muster noch einmal \
                             lasern.",
                        ))
                        .small()
                        .color(pal.warning));
                    }
                    ui.add_space(4.0);
                    let changed = rx.steps > 0 || ry.steps > 0;
                    if ui.add_enabled(changed, Button::new(trs(&format!("{} Übernehmen", icon::CHECK)))).clicked() {
                        action = Some("apply");
                    }
                });
            });
            if let Some(e) = &app.backlash.error {
                ui.label(RichText::new(trs(e)).color(pal.danger));
            }
            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                let ready = app.backlash.job.is_some() && app.settings.has_device();
                let connected = app.machine.connected && !app.machine.job_running;
                if ui
                    .add_enabled(ready && connected, Button::new(trs(&format!("{} Rahmen", icon::BOUNDING_BOX))))
                    .clicked()
                {
                    action = Some("frame");
                }
                if ui
                    .add_enabled(
                        ready && connected,
                        Button::new(RichText::new(trs(&format!("{} Muster lasern", icon::PLAY))).color(Color32::WHITE))
                            .fill(pal.success),
                    )
                    .clicked()
                {
                    action = Some("start");
                }
                if ui
                    .add_enabled(ready, Button::new(trs(&format!("{} G-Code speichern…", icon::FLOPPY_DISK))))
                    .clicked()
                {
                    action = Some("export");
                }
                if !app.machine.connected {
                    ui.label(RichText::new(tr("Kein Laser verbunden")).small().color(pal.text_weak));
                }
            });
        });
    app.backlash.spec = spec;
    app.backlash.x = rx;
    app.backlash.y = ry;
    match (action, app.backlash.job.clone()) {
        (Some("frame"), Some(job)) => {
            if app.ready_to_frame() {
                app.run_frame(&job, false);
            }
        }
        (Some("start"), Some(job)) => {
            if app.ready_to_start() {
                app.launch_job(job);
            }
        }
        (Some("export"), Some(job)) => {
            if let Some(path) = crate::system::FileDialog::new()
                .add_filter(tr("G-Code"), &["gcode", "nc"])
                .set_file_name(tr("Umkehrspiel.gcode"))
                .save_file()
            {
                app.write_gcode(&job, &path);
            }
        }
        (Some("apply"), _) => app.apply_backlash_reading(),
        _ => {}
    }
    if !open {
        app.backlash.open = false;
    }
}

fn r_at_limit(r: Reading) -> bool {
    r.steps as i32 >= STEPS
}

/// Verkleinerte Ansicht der Brennstriche (Draufsicht wie auf der Arbeitsfläche).
fn preview(ui: &mut egui::Ui, job: &Job, size: egui::Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let pal = theme::of(ui);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, pal.panel_alt);
    let b = job.path_bounds;
    if b.is_empty() {
        return;
    }
    let scale = ((size.x - 12.0) as f64 / b.width()).min((size.y - 12.0) as f64 / b.height());
    let off = rect.center() - vec2((b.width() * scale) as f32, (b.height() * scale) as f32) / 2.0;
    let map = |p: vl_core::Point| off + vec2(((p.x - b.min.x) * scale) as f32, ((p.y - b.min.y) * scale) as f32);
    let stroke = Stroke::new(1.0, pal.text);
    for block in &job.toolpath.blocks {
        let mut cur = None;
        for op in &block.ops {
            match op {
                Op::Rapid(p) => cur = Some(*p),
                Op::Move { to, .. } => {
                    if let Some(c) = cur {
                        painter.line_segment([map(c), map(*to)], stroke);
                    }
                    cur = Some(*to);
                }
            }
        }
    }
}
