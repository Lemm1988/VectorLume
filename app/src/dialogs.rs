//! Fenster und Abfragen: Text, Maschineneinstellungen, Info, Sicherheitsabfragen, Benachrichtigungen.

use crate::app::{Modal, ToastKind, VectorLumeApp};
use crate::theme;
use egui::{Align2, Button, Color32, CornerRadius, DragValue, Frame, Margin, RichText, Stroke, vec2};
use egui_phosphor::regular as icon;
use std::time::Instant;
use vl_cam::{MachineProfile, OriginCorner};

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    text_dialog(app, ctx);
    device_settings(app, ctx);
    about(app, ctx);
    modal(app, ctx);
    toasts(app, ctx);
}

fn text_dialog(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.text_dialog.open {
        return;
    }
    let families = app.font_families.lock().unwrap().clone();
    let mut open = true;
    let mut insert = false;
    egui::Window::new(format!("{} Text einfügen", icon::TEXT_T))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(380.0)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let d = &mut app.text_dialog;
            ui.add(
                egui::TextEdit::multiline(&mut d.spec.text)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text("Text"),
            );
            ui.add_space(6.0);
            egui::Grid::new("text_grid").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label("Schrift");
                match &families {
                    Some(list) => {
                        egui::ComboBox::from_id_salt("font")
                            .width(220.0)
                            .selected_text(&d.spec.family)
                            .height(320.0)
                            .show_ui(ui, |ui| {
                                ui.add(egui::TextEdit::singleline(&mut d.filter).hint_text("Suchen…"));
                                let filter = d.filter.to_lowercase();
                                for f in list.iter().filter(|f| filter.is_empty() || f.to_lowercase().contains(&filter))
                                {
                                    ui.selectable_value(&mut d.spec.family, f.clone(), f);
                                }
                            });
                    }
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Schriften werden geladen…");
                        });
                    }
                }
                ui.end_row();
                ui.label("Stil");
                ui.horizontal(|ui| {
                    ui.toggle_value(&mut d.spec.bold, RichText::new("F").strong());
                    ui.toggle_value(&mut d.spec.italic, RichText::new("K").italics());
                });
                ui.end_row();
                ui.label("Höhe");
                ui.add(DragValue::new(&mut d.spec.height_mm).speed(0.1).range(0.5..=500.0).suffix(" mm"));
                ui.end_row();
                ui.label("Zeichenabstand");
                ui.add(DragValue::new(&mut d.spec.letter_spacing).speed(0.05).range(-20.0..=100.0).suffix(" mm"));
                ui.end_row();
                ui.label("Zeilenabstand");
                ui.add(DragValue::new(&mut d.spec.line_spacing).speed(0.01).range(0.5..=4.0).fixed_decimals(2));
                ui.end_row();
                ui.label("Ausrichtung");
                ui.horizontal(|ui| {
                    use vl_io::TextAlign::*;
                    ui.selectable_value(&mut d.spec.align, Left, icon::TEXT_ALIGN_LEFT);
                    ui.selectable_value(&mut d.spec.align, Center, icon::TEXT_ALIGN_CENTER);
                    ui.selectable_value(&mut d.spec.align, Right, icon::TEXT_ALIGN_RIGHT);
                });
                ui.end_row();
            });
            if let Some(e) = &d.error {
                ui.label(RichText::new(e).color(theme::of(ui).danger));
            }
            ui.add_space(6.0);
            let ready = families.is_some() && !d.spec.text.trim().is_empty();
            if ui
                .add_enabled(ready, Button::new(format!("{} Einfügen", icon::PLUS)).min_size(vec2(120.0, 30.0)))
                .clicked()
            {
                insert = true;
            }
        });
    if insert {
        let db = vl_io::system_fonts();
        match vl_io::text_to_path(&db, &app.text_dialog.spec) {
            Ok(path) => {
                let at = app.text_dialog.at;
                let path = path.transformed(&vl_core::Affine::translate(at.x, at.y));
                let name =
                    app.text_dialog.spec.text.lines().next().unwrap_or("Text").chars().take(24).collect::<String>();
                app.checkpoint();
                let id = app.doc.add_path(format!("Text „{name}“"), app.current_layer, path);
                app.selection = vec![id];
                app.mark_changed();
                app.tool = crate::app::Tool::Select;
                app.text_dialog.open = false;
            }
            Err(e) => app.text_dialog.error = Some(e),
        }
    } else if !open {
        app.text_dialog.open = false;
    }
}

fn device_settings(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.show_device_settings {
        return;
    }
    let mut open = true;
    let mut p = app.settings.profile.clone();
    let have_device_values = !app.machine.settings.is_empty();
    let mut apply_device = false;
    egui::Window::new(format!("{} Maschineneinstellungen", icon::GEAR))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(420.0)
        .show(ctx, |ui| {
            egui::Grid::new("profile").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut p.name);
                ui.end_row();
                ui.label("Arbeitsfläche");
                ui.horizontal(|ui| {
                    ui.add(DragValue::new(&mut p.bed_width).speed(1.0).range(10.0..=3000.0).suffix(" mm"));
                    ui.label("×");
                    ui.add(DragValue::new(&mut p.bed_height).speed(1.0).range(10.0..=3000.0).suffix(" mm"));
                });
                ui.end_row();
                ui.label("Nullpunkt");
                egui::ComboBox::from_id_salt("origin").selected_text(p.origin.label()).show_ui(ui, |ui| {
                    for o in OriginCorner::ALL {
                        ui.selectable_value(&mut p.origin, o, o.label());
                    }
                });
                ui.end_row();
                ui.label("S-Max ($30)");
                ui.add(DragValue::new(&mut p.s_max).speed(10.0).range(1.0..=100000.0));
                ui.end_row();
                ui.label("Leistungsmodus");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut p.dynamic_power, true, "M4 dynamisch");
                    ui.radio_value(&mut p.dynamic_power, false, "M3 konstant");
                });
                ui.end_row();
                ui.label("Eilgang");
                ui.add(DragValue::new(&mut p.rapid_rate).speed(50.0).range(100.0..=100000.0).suffix(" mm/min"));
                ui.end_row();
                ui.label("Beschleunigung");
                ui.add(DragValue::new(&mut p.acceleration).speed(10.0).range(10.0..=50000.0).suffix(" mm/s²"));
                ui.end_row();
                ui.label("Luftunterstützung");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut p.air_assist_supported, "");
                    ui.add_enabled(
                        p.air_assist_supported,
                        egui::TextEdit::singleline(&mut p.air_on).desired_width(40.0),
                    );
                    ui.add_enabled(
                        p.air_assist_supported,
                        egui::TextEdit::singleline(&mut p.air_off).desired_width(40.0),
                    );
                });
                ui.end_row();
                ui.label("Z-Achse");
                ui.checkbox(&mut p.z_axis, "vorhanden (Fokus per G-Code)").on_hover_text(
                    "Nur aktivieren, wenn die Steuerung eine Z-Achse bzw. einen motorischen Tisch hat. \
                     Dann werden Z-Versatz und Absenkung je Durchgang ausgegeben.",
                );
                ui.end_row();
                ui.label("Baudrate");
                egui::ComboBox::from_id_salt("baud").selected_text(p.baud.to_string()).show_ui(ui, |ui| {
                    for b in [9600, 38400, 57600, 115_200, 230_400, 250_000, 921_600] {
                        ui.selectable_value(&mut p.baud, b, b.to_string());
                    }
                });
                ui.end_row();
                ui.label("Empfangspuffer");
                ui.add(DragValue::new(&mut p.rx_buffer).range(32..=4096).suffix(" Byte"));
                ui.end_row();
                ui.label("Nach dem Job");
                ui.checkbox(&mut p.return_to_origin, "Zum Ursprung zurückfahren");
                ui.end_row();
                ui.label("Rahmen");
                ui.horizontal(|ui| {
                    ui.add(DragValue::new(&mut p.frame_speed).speed(50.0).range(100.0..=30000.0).suffix(" mm/min"));
                    ui.add(DragValue::new(&mut p.frame_power).speed(0.1).range(0.0..=5.0).suffix(" %"))
                        .on_hover_text("0 % = Laser aus. Höchstens 5 %, nur mit Schutzbrille!");
                });
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(have_device_values, Button::new(format!("{} Werte vom Gerät", icon::DOWNLOAD_SIMPLE)))
                    .on_hover_text(
                        "S-Max ($30), Arbeitsfläche ($130/$131), Eilgang ($110) und Beschleunigung ($120) übernehmen",
                    )
                    .clicked()
                {
                    apply_device = true;
                }
                if ui.button("Sculpfun SF-A9 Standard").clicked() {
                    p = MachineProfile::sculpfun_sf_a9();
                }
            });
        });
    if p != app.settings.profile {
        let bed_changed =
            p.bed_width != app.settings.profile.bed_width || p.bed_height != app.settings.profile.bed_height;
        app.settings.profile = p;
        app.settings.save();
        app.revision += 1;
        if bed_changed {
            app.view.request_fit(None);
        }
    }
    if apply_device {
        app.apply_device_settings();
    }
    if !open {
        app.show_device_settings = false;
    }
}

fn about(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.show_about {
        return;
    }
    let mut open = true;
    egui::Window::new("Über VectorLume").open(&mut open).collapsible(false).resizable(false).default_width(420.0).show(
        ctx,
        |ui| {
            let pal = theme::of(ui);
            ui.label(
                RichText::new(format!("{} VectorLume", icon::SPARKLE)).font(theme::strong_font(22.0)).color(pal.accent),
            );
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.add_space(6.0);
            ui.label("Software zum Gestalten und Lasern für GRBL-Diodenlaser. Eigenständig in Rust entwickelt.");
            ui.add_space(6.0);
            ui.label(RichText::new("Tastenkürzel").strong());
            egui::Grid::new("keys").num_columns(2).show(ui, |ui| {
                for (k, d) in [
                    ("V / R / E / L / T", "Auswahl, Rechteck, Ellipse, Linie, Text"),
                    ("Mausrad", "Zoomen"),
                    ("Mittlere Maustaste / Leertaste + Ziehen", "Ansicht verschieben"),
                    ("Umschalt beim Ziehen", "Seitenverhältnis frei / 15°-Schritte / Achse fixieren"),
                    ("Pfeiltasten (+Umschalt)", "Auswahl 1 mm (10 mm) verschieben"),
                    ("Alt+P", "Vorschau"),
                    ("F", "Arbeitsfläche einpassen"),
                ] {
                    ui.label(RichText::new(k).monospace());
                    ui.label(d);
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            ui.label(RichText::new("Privates Projekt – alle Rechte vorbehalten.").small().color(pal.text_weak));
        },
    );
    if !open {
        app.show_about = false;
    }
}

/// Aktion, die nach dem Schließen einer Abfrage ausgeführt wird.
type ModalAction = Box<dyn FnOnce(&mut VectorLumeApp)>;

fn modal(app: &mut VectorLumeApp, ctx: &egui::Context) {
    let Some(m) = app.modal.take() else { return };
    let mut keep = true;
    let mut action: Option<ModalAction> = None;
    let resp = egui::Modal::new(egui::Id::new("vl_modal")).show(ctx, |ui| {
        let pal = theme::of(ui);
        ui.set_max_width(440.0);
        match &m {
            Modal::Safety { job } => {
                ui.label(RichText::new(format!("{} Sicherheitshinweis", icon::WARNING)).font(theme::strong_font(18.0)).color(pal.warning));
                ui.add_space(6.0);
                ui.label("Laser dieser Leistungsklasse (Klasse 4) können Augen dauerhaft schädigen und Brände verursachen.");
                ui.add_space(4.0);
                for t in [
                    "Immer eine zur Wellenlänge passende Laserschutzbrille tragen (Diodenlaser: ca. 450 nm).",
                    "Den Laser während des Betriebs nie unbeaufsichtigt lassen, Feuerlöscher bereithalten.",
                    "Für Absaugung bzw. Belüftung sorgen. Kein PVC/Vinyl lasern (giftige Gase).",
                    "Neue Einstellungen zuerst mit geringer Leistung testen.",
                ] {
                    ui.label(format!("• {t}"));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(Button::new(RichText::new("Verstanden, Job starten").color(Color32::WHITE)).fill(pal.success)).clicked() {
                        let job = job.clone();
                        action = Some(Box::new(move |app: &mut VectorLumeApp| {
                            app.settings.safety_acknowledged = true;
                            app.settings.save();
                            if job.warnings.is_empty() {
                                app.send_job(&job);
                            } else {
                                app.modal = Some(Modal::ConfirmStart { job });
                            }
                        }));
                        keep = false;
                    }
                    if ui.button("Abbrechen").clicked() {
                        keep = false;
                    }
                });
            }
            Modal::ConfirmStart { job } => {
                ui.label(RichText::new(format!("{} Job trotzdem starten?", icon::WARNING)).font(theme::strong_font(18.0)).color(pal.warning));
                ui.add_space(6.0);
                for w in &job.warnings {
                    ui.label(format!("• {w}"));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(Button::new(RichText::new("Trotzdem starten").color(Color32::WHITE)).fill(pal.warning)).clicked() {
                        let job = job.clone();
                        action = Some(Box::new(move |app: &mut VectorLumeApp| app.send_job(&job)));
                        keep = false;
                    }
                    if ui.button("Abbrechen").clicked() {
                        keep = false;
                    }
                });
            }
            Modal::LaserModeOff => {
                ui.label(RichText::new(format!("{} Laser-Modus ist ausgeschaltet", icon::WARNING)).font(theme::strong_font(18.0)).color(pal.danger));
                ui.add_space(6.0);
                ui.label("Die Steuerung meldet $32=0. In diesem Zustand bleibt der Laser bei Leerfahrten an und brennt Linien zwischen den Formen. Außerdem hält die Maschine bei jeder Leistungsänderung kurz an.");
                ui.add_space(4.0);
                ui.label("Empfehlung: Laser-Modus mit „$32=1“ dauerhaft einschalten.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(Button::new(RichText::new("$32=1 senden").color(Color32::WHITE)).fill(pal.accent)).clicked() {
                        action = Some(Box::new(|app: &mut VectorLumeApp| {
                            app.send_line("$32=1");
                            app.send_line("$$");
                            app.toast(ToastKind::Info, "Laser-Modus wird eingeschaltet…");
                        }));
                        keep = false;
                    }
                    if ui.button("Abbrechen").clicked() {
                        keep = false;
                    }
                });
            }
            Modal::Unsaved(then) => {
                ui.label(RichText::new("Ungespeicherte Änderungen").font(theme::strong_font(18.0)));
                ui.add_space(6.0);
                ui.label("Das aktuelle Projekt wurde geändert. Möchtest du es vorher speichern?");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(Button::new(RichText::new("Speichern").color(Color32::WHITE)).fill(pal.accent)).clicked() {
                        let then = then.clone();
                        action = Some(Box::new(move |app: &mut VectorLumeApp| {
                            if app.save(false) {
                                app.after_discard(then);
                            }
                        }));
                        keep = false;
                    }
                    if ui.button("Verwerfen").clicked() {
                        let then = then.clone();
                        action = Some(Box::new(move |app: &mut VectorLumeApp| {
                            app.dirty = false;
                            app.after_discard(then);
                        }));
                        keep = false;
                    }
                    if ui.button("Abbrechen").clicked() {
                        keep = false;
                    }
                });
            }
            Modal::Recover { path, meta } => {
                ui.label(RichText::new(format!("{} Ungespeicherte Arbeit gefunden", icon::CLOCK_COUNTER_CLOCKWISE)).font(theme::strong_font(18.0)));
                ui.add_space(6.0);
                let of = meta.original.as_deref().map(crate::app::file_name).map(|n| format!(" von „{n}“")).unwrap_or_default();
                ui.label(format!(
                    "VectorLume wurde zuletzt nicht regulär beendet. Es gibt eine automatische Sicherung{of} ({}).",
                    meta.age_label()
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(Button::new(RichText::new("Wiederherstellen").color(Color32::WHITE)).fill(pal.accent)).clicked() {
                        let (path, original) = (path.clone(), meta.original.clone());
                        action = Some(Box::new(move |app: &mut VectorLumeApp| app.restore_autosave(&path, original)));
                        keep = false;
                    }
                    if ui.button("Verwerfen").clicked() {
                        action = Some(Box::new(|app: &mut VectorLumeApp| app.autosave.clear()));
                        keep = false;
                    }
                });
            }
            Modal::Message { title, text } => {
                ui.label(RichText::new(title).font(theme::strong_font(18.0)));
                ui.add_space(6.0);
                ui.label(text);
                ui.add_space(8.0);
                if ui.button("OK").clicked() {
                    keep = false;
                }
            }
        }
    });
    if resp.should_close() && keep {
        // Escape oder Klick daneben: abbrechen, außer der Hinweis verlangt eine Entscheidung.
        keep = false;
    }
    if keep {
        app.modal = Some(m);
    }
    if let Some(a) = action {
        a(app);
    }
}

fn toasts(app: &mut VectorLumeApp, ctx: &egui::Context) {
    let now = Instant::now();
    app.toasts.retain(|t| t.until > now);
    if app.toasts.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::new("toasts"))
        .anchor(Align2::RIGHT_BOTTOM, vec2(-380.0, -84.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            for t in &app.toasts {
                let (glyph, color) = match t.kind {
                    ToastKind::Info => (icon::INFO, pal.accent),
                    ToastKind::Success => (icon::CHECK, pal.success),
                    ToastKind::Error => (icon::WARNING, pal.danger),
                };
                Frame::NONE
                    .fill(pal.panel_alt)
                    .stroke(Stroke::new(1.0, color.gamma_multiply(0.7)))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_max_width(360.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(glyph).color(color).size(16.0));
                            ui.label(&t.text);
                        });
                    });
                ui.add_space(6.0);
            }
        });
}
