//! Drehachse einrichten: Bauart, Anschluss, Durchmesser, Weg je Umdrehung und Testdrehungen.

use crate::app::VectorLumeApp;
use crate::i18n::{tr, trs};
use crate::theme;
use egui::{Align2, Button, DragValue, RichText, Ui, vec2};
use egui_phosphor::regular as icon;
use std::f64::consts::PI;
use vl_cam::{RotaryAxis, RotaryKind, StartFrom};

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.show_rotary {
        return;
    }
    let mut open = true;
    let mut r = app.settings.profile.rotary.clone();
    let can_move = app.machine.connected && !app.machine.job_running;
    let mut test: Option<f64> = None;
    let mut use_current = false;
    egui::Window::new(trs(&format!("{} Drehachse", icon::ARROWS_CLOCKWISE)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(400.0)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            ui.checkbox(&mut r.enabled, tr("Drehachse verwenden"));
            ui.add_space(4.0);
            ui.add_enabled_ui(r.enabled, |ui| {
                egui::Grid::new("rotary").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                    ui.label(tr("Bauart"));
                    egui::ComboBox::from_id_salt("rotary_kind").selected_text(tr(r.kind.label())).show_ui(ui, |ui| {
                        for k in RotaryKind::ALL {
                            ui.selectable_value(&mut r.kind, k, tr(k.label()));
                        }
                    });
                    ui.end_row();
                    ui.label(tr("Anschluss"));
                    egui::ComboBox::from_id_salt("rotary_axis").selected_text(tr(r.axis.label())).show_ui(ui, |ui| {
                        for a in RotaryAxis::ALL {
                            ui.selectable_value(&mut r.axis, a, tr(a.label()));
                        }
                    });
                    ui.end_row();
                    let unit = if r.axis == RotaryAxis::A { "" } else { " mm" };
                    ui.label(tr("Weg je Umdrehung"));
                    ui.add(DragValue::new(&mut r.travel_per_rotation).speed(0.1).range(0.1..=100_000.0).suffix(unit))
                        .on_hover_text(tr(
                            "Achsweg, den die Steuerung für eine volle Umdrehung der Rolle bzw. des Futters fährt. \
                             Mit „1× drehen“ prüfen und anpassen, bis es genau eine Umdrehung ist.",
                        ));
                    ui.end_row();
                    if r.kind == RotaryKind::Roller {
                        ui.label(tr("Rollendurchmesser"));
                        ui.add(DragValue::new(&mut r.roller_diameter).speed(0.1).range(1.0..=500.0).suffix(" mm"))
                            .on_hover_text(tr("Durchmesser der angetriebenen Rolle, auf der das Werkstück liegt"));
                        ui.end_row();
                    }
                    ui.label(tr("Werkstück Ø"));
                    ui.horizontal(|ui| {
                        ui.add(DragValue::new(&mut r.object_diameter).speed(0.1).range(1.0..=1000.0).suffix(" mm"));
                        let mut u = r.circumference();
                        ui.label(tr("Umfang"));
                        if ui
                            .add(DragValue::new(&mut u).speed(0.2).range(3.0..=3200.0).max_decimals(1).suffix(" mm"))
                            .changed()
                        {
                            r.object_diameter = u / PI;
                        }
                    });
                    ui.end_row();
                    ui.label(tr("Richtung"));
                    ui.checkbox(&mut r.reverse, tr("umkehren"));
                    ui.end_row();
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(trs(&format!(
                        "1 mm Oberfläche = {} Achsweg · Motivhöhe bis {} mm (Umfang)",
                        format_scale(r.scale().abs(), r.axis),
                        vl_cam::gcode::fmt_mm((r.circumference() * 10.0).round() / 10.0)
                    )))
                    .small()
                    .color(pal.text_weak),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let what = if r.kind == RotaryKind::Roller { tr("Rolle") } else { tr("Futter") };
                    let sign = if r.reverse { -1.0 } else { 1.0 };
                    if ui
                        .add_enabled(can_move, Button::new(trs(&format!("{} {what} 1× drehen", icon::ARROW_CLOCKWISE))))
                        .on_hover_text(tr("Fährt den eingestellten Weg je Umdrehung (Laser aus)"))
                        .clicked()
                    {
                        test = Some(r.travel_per_rotation * sign);
                    }
                    if r.kind == RotaryKind::Roller
                        && ui
                            .add_enabled(
                                can_move,
                                Button::new(trs(&format!("{} Werkstück 1× drehen", icon::ARROW_CLOCKWISE))),
                            )
                            .on_hover_text(tr(
                                "Prüft Rollen- und Werkstückdurchmesser: dreht das Werkstück genau einmal",
                            ))
                            .clicked()
                    {
                        test = Some(r.circumference() * r.scale());
                    }
                });
                if !can_move {
                    ui.label(RichText::new(tr("Zum Testen mit dem Laser verbinden.")).small().color(pal.text_weak));
                }
                ui.add_space(6.0);
                let hint = match r.axis {
                    RotaryAxis::Y => {
                        tr("Den Stecker des Y-Motors auf die Drehachse umstecken. Keine Referenzfahrt ausführen – \
                         der Y-Endschalter wird nicht mehr erreicht – und Softlimits ($20=0) ausschalten. \
                         Y im Dokument ist die abgewickelte Oberfläche.")
                    }
                    RotaryAxis::A => tr(
                        "Nur für Steuerungen mit vierter Achse (z. B. FluidNC, grblHAL). Der Kopf bleibt in Y stehen; \
                         Y im Dokument ist die abgewickelte Oberfläche.",
                    ),
                };
                ui.label(RichText::new(hint).small().color(pal.text_weak));
                if app.settings.job.start_from != StartFrom::CurrentPosition {
                    ui.add_space(4.0);
                    if ui
                        .button(trs(&format!("{} Start auf „Aktuelle Position“ stellen", icon::CROSSHAIR)))
                        .on_hover_text(tr("Empfohlen: der Job beginnt dort, wo der Laser über dem Werkstück steht"))
                        .clicked()
                    {
                        use_current = true;
                    }
                }
            });
        });
    if r != app.settings.profile.rotary {
        app.settings.profile.rotary = r;
        app.settings.save();
        app.revision += 1;
    }
    if use_current {
        app.settings.job.start_from = StartFrom::CurrentPosition;
        app.settings.save();
        app.revision += 1;
    }
    if let Some(travel) = test {
        app.jog_rotary(travel);
    }
    if !open {
        app.show_rotary = false;
    }
}

fn format_scale(k: f64, axis: RotaryAxis) -> String {
    match axis {
        RotaryAxis::Y => format!("{k:.4} mm"),
        RotaryAxis::A => trs(&format!("{k:.4} Einheiten")),
    }
}

/// Zeile im Job-Bereich: Drehachse an/aus und Einrichten.
pub fn job_row(app: &mut VectorLumeApp, ui: &mut Ui) {
    let pal = theme::of(ui);
    ui.horizontal(|ui| {
        let mut on = app.settings.profile.rotary.enabled;
        if ui.checkbox(&mut on, tr("Drehachse")).changed() {
            app.settings.profile.rotary.enabled = on;
            app.settings.save();
            app.revision += 1;
        }
        if on {
            let r = &app.settings.profile.rotary;
            ui.label(
                RichText::new(format!("Ø {} mm · {}", vl_cam::gcode::fmt_mm(r.object_diameter), tr(r.kind.label())))
                    .small()
                    .color(pal.accent),
            );
        }
        if ui.small_button(icon::GEAR).on_hover_text(tr("Drehachse einrichten")).clicked() {
            app.show_rotary = true;
        }
    });
}
