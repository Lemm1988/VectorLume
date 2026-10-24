//! Drehachse einrichten: Bauart, Anschluss, Durchmesser, Weg je Umdrehung und Testdrehungen.

use crate::app::VectorLumeApp;
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
    egui::Window::new(format!("{} Drehachse", icon::ARROWS_CLOCKWISE))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(400.0)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            ui.checkbox(&mut r.enabled, "Drehachse verwenden");
            ui.add_space(4.0);
            ui.add_enabled_ui(r.enabled, |ui| {
                egui::Grid::new("rotary").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                    ui.label("Bauart");
                    egui::ComboBox::from_id_salt("rotary_kind").selected_text(r.kind.label()).show_ui(ui, |ui| {
                        for k in RotaryKind::ALL {
                            ui.selectable_value(&mut r.kind, k, k.label());
                        }
                    });
                    ui.end_row();
                    ui.label("Anschluss");
                    egui::ComboBox::from_id_salt("rotary_axis").selected_text(r.axis.label()).show_ui(ui, |ui| {
                        for a in RotaryAxis::ALL {
                            ui.selectable_value(&mut r.axis, a, a.label());
                        }
                    });
                    ui.end_row();
                    let unit = if r.axis == RotaryAxis::A { "" } else { " mm" };
                    ui.label("Weg je Umdrehung");
                    ui.add(DragValue::new(&mut r.travel_per_rotation).speed(0.1).range(0.1..=100_000.0).suffix(unit))
                        .on_hover_text(
                            "Achsweg, den die Steuerung für eine volle Umdrehung der Rolle bzw. des Futters fährt. \
                             Mit „1× drehen“ prüfen und anpassen, bis es genau eine Umdrehung ist.",
                        );
                    ui.end_row();
                    if r.kind == RotaryKind::Roller {
                        ui.label("Rollendurchmesser");
                        ui.add(DragValue::new(&mut r.roller_diameter).speed(0.1).range(1.0..=500.0).suffix(" mm"))
                            .on_hover_text("Durchmesser der angetriebenen Rolle, auf der das Werkstück liegt");
                        ui.end_row();
                    }
                    ui.label("Werkstück Ø");
                    ui.horizontal(|ui| {
                        ui.add(DragValue::new(&mut r.object_diameter).speed(0.1).range(1.0..=1000.0).suffix(" mm"));
                        let mut u = r.circumference();
                        ui.label("Umfang");
                        if ui
                            .add(DragValue::new(&mut u).speed(0.2).range(3.0..=3200.0).max_decimals(1).suffix(" mm"))
                            .changed()
                        {
                            r.object_diameter = u / PI;
                        }
                    });
                    ui.end_row();
                    ui.label("Richtung");
                    ui.checkbox(&mut r.reverse, "umkehren");
                    ui.end_row();
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "1 mm Oberfläche = {} Achsweg · Motivhöhe bis {} mm (Umfang)",
                        format_scale(r.scale().abs(), r.axis),
                        vl_cam::gcode::fmt_mm((r.circumference() * 10.0).round() / 10.0)
                    ))
                    .small()
                    .color(pal.text_weak),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let what = if r.kind == RotaryKind::Roller { "Rolle" } else { "Futter" };
                    let sign = if r.reverse { -1.0 } else { 1.0 };
                    if ui
                        .add_enabled(can_move, Button::new(format!("{} {what} 1× drehen", icon::ARROW_CLOCKWISE)))
                        .on_hover_text("Fährt den eingestellten Weg je Umdrehung (Laser aus)")
                        .clicked()
                    {
                        test = Some(r.travel_per_rotation * sign);
                    }
                    if r.kind == RotaryKind::Roller
                        && ui
                            .add_enabled(
                                can_move,
                                Button::new(format!("{} Werkstück 1× drehen", icon::ARROW_CLOCKWISE)),
                            )
                            .on_hover_text("Prüft Rollen- und Werkstückdurchmesser: dreht das Werkstück genau einmal")
                            .clicked()
                    {
                        test = Some(r.circumference() * r.scale());
                    }
                });
                if !can_move {
                    ui.label(RichText::new("Zum Testen mit dem Laser verbinden.").small().color(pal.text_weak));
                }
                ui.add_space(6.0);
                let hint = match r.axis {
                    RotaryAxis::Y => {
                        "Den Stecker des Y-Motors auf die Drehachse umstecken. Keine Referenzfahrt ausführen – \
                         der Y-Endschalter wird nicht mehr erreicht – und Softlimits ($20=0) ausschalten. \
                         Y im Dokument ist die abgewickelte Oberfläche."
                    }
                    RotaryAxis::A => {
                        "Nur für Steuerungen mit vierter Achse (z. B. FluidNC, grblHAL). Der Kopf bleibt in Y stehen; \
                         Y im Dokument ist die abgewickelte Oberfläche."
                    }
                };
                ui.label(RichText::new(hint).small().color(pal.text_weak));
                if app.settings.job.start_from != StartFrom::CurrentPosition {
                    ui.add_space(4.0);
                    if ui
                        .button(format!("{} Start auf „Aktuelle Position“ stellen", icon::CROSSHAIR))
                        .on_hover_text("Empfohlen: der Job beginnt dort, wo der Laser über dem Werkstück steht")
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
        RotaryAxis::A => format!("{k:.4} Einheiten"),
    }
}

/// Zeile im Job-Bereich: Drehachse an/aus und Einrichten.
pub fn job_row(app: &mut VectorLumeApp, ui: &mut Ui) {
    let pal = theme::of(ui);
    ui.horizontal(|ui| {
        let mut on = app.settings.profile.rotary.enabled;
        if ui.checkbox(&mut on, "Drehachse").changed() {
            app.settings.profile.rotary.enabled = on;
            app.settings.save();
            app.revision += 1;
        }
        if on {
            let r = &app.settings.profile.rotary;
            ui.label(
                RichText::new(format!("Ø {} mm · {}", vl_cam::gcode::fmt_mm(r.object_diameter), r.kind.label()))
                    .small()
                    .color(pal.accent),
            );
        }
        if ui.small_button(icon::GEAR).on_hover_text("Drehachse einrichten").clicked() {
            app.show_rotary = true;
        }
    });
}
