//! Fenster und Abfragen: Maschineneinstellungen, Info, Sicherheitsabfragen, Benachrichtigungen.

use crate::app::{Modal, ToastKind, VectorLumeApp};
use crate::i18n::{tr, trs};
use crate::theme;
use crate::units::UnitField;
use egui::{Align2, Button, Color32, CornerRadius, DragValue, Frame, Margin, RichText, Stroke, vec2};
use egui_phosphor::regular as icon;
use std::time::Instant;
use vl_cam::{MachineProfile, OriginCorner};
use vl_core::Firmware;

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    crate::text_tool::show(app, ctx);
    crate::rotary_ui::show(app, ctx);
    crate::print_cut::show(app, ctx);
    crate::center_finder::show(app, ctx);
    crate::box_gen::show(app, ctx);
    crate::hinge_tool::show(app, ctx);
    crate::puzzle_tool::show(app, ctx);
    crate::warp_tool::show(app, ctx);
    device_settings(app, ctx);
    about(app, ctx);
    notes(app, ctx);
    modal(app, ctx);
    toasts(app, ctx);
}

fn device_settings(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.show_device_settings {
        return;
    }
    let mut open = true;
    let mut p = app.settings.profile.clone();
    let have_device_values = !app.machine.settings.is_empty();
    let mut apply_device = false;
    let mut device_action: Option<DeviceAction> = None;
    egui::Window::new(trs(&format!("{} Maschineneinstellungen", icon::GEAR)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(420.0)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            device_row(app, ui, &mut device_action);
            ui.separator();
            egui::Grid::new("profile").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                ui.label(tr("Name"));
                ui.text_edit_singleline(&mut p.name);
                ui.end_row();
                ui.label(tr("Steuerung"));
                let before = p.firmware;
                egui::ComboBox::from_id_salt("firmware").selected_text(tr(p.firmware.label())).show_ui(ui, |ui| {
                    for f in Firmware::ALL {
                        ui.selectable_value(&mut p.firmware, f, tr(f.label()));
                    }
                });
                if p.firmware != before && p.s_max == before.default_s_max() {
                    p.s_max = p.firmware.default_s_max();
                }
                ui.end_row();
                ui.label(tr("Arbeitsfläche"));
                ui.horizontal(|ui| {
                    ui.add(DragValue::new(&mut p.bed_width).speed(1.0).range(10.0..=3000.0).mm());
                    ui.label("×");
                    ui.add(DragValue::new(&mut p.bed_height).speed(1.0).range(10.0..=3000.0).mm());
                });
                ui.end_row();
                ui.label(tr("Nullpunkt"));
                egui::ComboBox::from_id_salt("origin").selected_text(tr(p.origin.label())).show_ui(ui, |ui| {
                    for o in OriginCorner::ALL {
                        ui.selectable_value(&mut p.origin, o, tr(o.label()));
                    }
                });
                ui.end_row();
                let (s_label, s_tip) = match p.firmware {
                    Firmware::Grbl => (tr("S-Max ($30)"), tr("Höchstwert für S, wie in $30 eingestellt")),
                    Firmware::Marlin => (tr("S-Max"), tr("Höchstwert für S, meist 255 (PWM) oder 100 (Prozent)")),
                    Firmware::Smoothieware => (tr("S-Max"), tr("Smoothieware erwartet 0…1 – S-Max 1 = volle Leistung")),
                };
                ui.label(s_label);
                let s_speed = if p.s_max <= 10.0 { 0.01 } else { 10.0 };
                ui.add(DragValue::new(&mut p.s_max).speed(s_speed).range(0.01..=100000.0)).on_hover_text(s_tip);
                ui.end_row();
                ui.label(tr("Leistungsmodus"));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut p.dynamic_power, true, tr("M4 dynamisch"));
                    ui.radio_value(&mut p.dynamic_power, false, tr("M3 konstant"));
                });
                ui.end_row();
                ui.label(tr("Eilgang"));
                ui.add(DragValue::new(&mut p.rapid_rate).speed(50.0).range(100.0..=100000.0).mm_per_min());
                ui.end_row();
                ui.label(tr("Beschleunigung"));
                ui.add(DragValue::new(&mut p.acceleration).speed(10.0).range(10.0..=50000.0).suffix(" mm/s²"));
                ui.end_row();
                ui.label(tr("Luftunterstützung"));
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
                ui.label(tr("Z-Achse"));
                ui.checkbox(&mut p.z_axis, tr("vorhanden (Fokus per G-Code)")).on_hover_text(tr(
                    "Nur aktivieren, wenn die Steuerung eine Z-Achse bzw. einen motorischen Tisch hat. \
                     Dann werden Z-Versatz und Absenkung je Durchgang ausgegeben.",
                ));
                ui.end_row();
                ui.label(tr("Baudrate"));
                egui::ComboBox::from_id_salt("baud").selected_text(p.baud.to_string()).show_ui(ui, |ui| {
                    for b in [9600, 38400, 57600, 115_200, 230_400, 250_000, 921_600] {
                        ui.selectable_value(&mut p.baud, b, b.to_string());
                    }
                });
                ui.end_row();
                if p.firmware == Firmware::Grbl {
                    ui.label(tr("Empfangspuffer"));
                    ui.add(DragValue::new(&mut p.rx_buffer).range(32..=4096).suffix(tr(" Byte")));
                    ui.end_row();
                }
                ui.label(tr("Nach dem Job"));
                ui.checkbox(&mut p.return_to_origin, tr("Zum Ursprung zurückfahren"));
                ui.end_row();
                ui.label(tr("Rahmen"));
                ui.horizontal(|ui| {
                    ui.add(DragValue::new(&mut p.frame_speed).speed(50.0).range(100.0..=30000.0).mm_per_min());
                    ui.add(DragValue::new(&mut p.frame_power).speed(0.1).range(0.0..=5.0).suffix(" %"))
                        .on_hover_text(tr("0 % = Laser aus. Höchstens 5 %, nur mit Schutzbrille!"));
                });
                ui.end_row();
                ui.label(tr("Scan-Versatz")).on_hover_text(tr("Beim Gravieren in beide Richtungen landen hin- und zurückgefahrene Zeilen versetzt, die \
                         Kanten wirken gezackt oder doppelt. Ein Rechteck mit der jeweiligen Geschwindigkeit bidirektional füllen, die Breite \
                         der Zacken messen und hier eintragen. Jede Zeile wird dann um die Hälfte gegen ihre Fahrtrichtung verschoben, \
                         Zwischenwerte werden interpoliert. Ohne Eintrag ist der Ausgleich aus."));
                scan_offsets_ui(ui, &mut p.scan_offsets);
                ui.end_row();
                ui.label(tr("Bildgravur"));
                ui.checkbox(&mut p.fast_whitespace, tr("Weiße Lücken im Eilgang überfahren")).on_hover_text(tr(
                    "Lange weiße Stellen innerhalb einer Bildzeile werden nicht mit Gravurgeschwindigkeit, sondern \
                     im Eilgang (G0) überfahren – mit Aus- und Anlauf um den Overscan. Spart bei Logos und Schriften \
                     viel Zeit. Nur Lücken über 5 mm plus zweimal Overscan.",
                ));
                ui.end_row();
            });
            if p.firmware != Firmware::Grbl {
                ui.add_space(4.0);
                let note = match p.firmware {
                    Firmware::Marlin => {
                        tr("Marlin: Laser im Inline-Modus (M3/M4 I), Zeile für Zeile mit Warten auf „ok“. \
                         Für sofortiges Stoppen EMERGENCY_PARSER in der Firmware aktivieren.")
                    }
                    _ => tr("Smoothieware: Leistung als S 0…1, Zeile für Zeile mit Warten auf „ok“."),
                };
                ui.label(RichText::new(note).small().color(theme::of(ui).text_weak));
            }
            if app.machine.connected && p.firmware != app.settings.profile.firmware {
                ui.label(RichText::new(tr("Wirkt beim nächsten Verbinden.")).small().color(theme::of(ui).warning));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        have_device_values,
                        Button::new(trs(&format!("{} Werte vom Gerät", icon::DOWNLOAD_SIMPLE))),
                    )
                    .on_hover_text(tr(
                        "S-Max ($30), Arbeitsfläche ($130/$131), Eilgang ($110) und Beschleunigung ($120) übernehmen",
                    ))
                    .clicked()
                {
                    apply_device = true;
                }
                if ui.button(tr("Sculpfun SF-A9 Standard")).clicked() {
                    // Drehachse (Zubehör) und gemessener Scan-Versatz bleiben erhalten.
                    p = MachineProfile {
                        rotary: p.rotary.clone(),
                        scan_offsets: p.scan_offsets.clone(),
                        ..MachineProfile::sculpfun_sf_a9()
                    };
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
    match device_action {
        Some(DeviceAction::Switch(i)) => app.switch_device(i),
        Some(DeviceAction::New) => {
            app.settings.add_device(MachineProfile { name: tr("Neues Gerät").into(), ..Default::default() });
            app.after_device_change();
        }
        Some(DeviceAction::Duplicate) => {
            let mut copy = app.settings.profile.clone();
            copy.name = trs(&format!("{} (Kopie)", copy.name));
            app.settings.add_device(copy);
            app.after_device_change();
        }
        Some(DeviceAction::Delete) => {
            if app.settings.remove_device() {
                app.after_device_change();
            }
        }
        Some(DeviceAction::Import) => app.import_device_dialog(),
        Some(DeviceAction::Export) => app.export_device_dialog(),
        None => {}
    }
    if !open {
        app.show_device_settings = false;
    }
}

enum DeviceAction {
    Switch(usize),
    New,
    Duplicate,
    Delete,
    Import,
    Export,
}

/// Geräteauswahl mit Neu, Duplizieren, Löschen (mit Rückfrage), Import und Export.
fn device_row(app: &VectorLumeApp, ui: &mut egui::Ui, action: &mut Option<DeviceAction>) {
    let names = app.settings.device_names();
    let current = app.settings.device_index;
    let can_switch = !app.machine.connected;
    ui.horizontal(|ui| {
        ui.label(tr("Gerät"));
        ui.add_enabled_ui(can_switch, |ui| {
            egui::ComboBox::from_id_salt("device")
                .width(170.0)
                .selected_text(names.get(current).cloned().unwrap_or_default())
                .show_ui(ui, |ui| {
                    for (i, n) in names.iter().enumerate() {
                        if ui.selectable_label(i == current, n).clicked() && i != current {
                            *action = Some(DeviceAction::Switch(i));
                        }
                    }
                });
        })
        .response
        .on_disabled_hover_text(tr("Zum Wechseln erst die Verbindung trennen"));
        if ui.button(icon::PLUS).on_hover_text(tr("Neues Gerät")).clicked() {
            *action = Some(DeviceAction::New);
        }
        if ui.button(icon::COPY).on_hover_text(tr("Gerät duplizieren")).clicked() {
            *action = Some(DeviceAction::Duplicate);
        }
        let confirm_id = ui.id().with("delete_device");
        let confirming = ui.data(|d| d.get_temp::<bool>(confirm_id).unwrap_or(false));
        let delete = if confirming {
            Button::new(RichText::new(tr("Wirklich löschen?")).color(Color32::WHITE)).fill(theme::of(ui).danger)
        } else {
            Button::new(icon::TRASH)
        };
        let resp = ui
            .add_enabled(names.len() > 1 && can_switch, delete)
            .on_hover_text(tr("Gerät löschen"))
            .on_disabled_hover_text(tr("Das letzte Gerät kann nicht gelöscht werden"));
        if resp.clicked() {
            if confirming {
                *action = Some(DeviceAction::Delete);
            }
            ui.data_mut(|d| d.insert_temp(confirm_id, !confirming));
        } else if confirming && resp.clicked_elsewhere() {
            ui.data_mut(|d| d.insert_temp(confirm_id, false));
        }
        if ui.button(icon::DOWNLOAD_SIMPLE).on_hover_text(tr("Gerät aus Datei importieren")).clicked() {
            *action = Some(DeviceAction::Import);
        }
        if ui.button(icon::UPLOAD_SIMPLE).on_hover_text(tr("Gerät in Datei exportieren (zum Weitergeben)")).clicked() {
            *action = Some(DeviceAction::Export);
        }
    });
}

/// Notizen zum Projekt; sie werden mit dem Projekt gespeichert.
fn notes(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.show_notes {
        return;
    }
    let mut open = true;
    let before = app.doc.notes.clone();
    egui::Window::new(trs(&format!("{} Projektnotizen", icon::NOTE)))
        .open(&mut open)
        .collapsible(false)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .default_width(380.0)
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let n = &mut app.doc.notes;
            ui.add(
                egui::TextEdit::multiline(&mut n.text)
                    .desired_rows(8)
                    .desired_width(f32::INFINITY)
                    .hint_text(tr("z. B. Material, Einstellungen, Fokus, Hinweise für das nächste Mal")),
            );
            ui.checkbox(&mut n.show_on_open, tr("Beim Öffnen des Projekts anzeigen"));
            ui.label(RichText::new(tr("Wird mit dem Projekt gespeichert.")).small().color(pal.text_weak));
        });
    if app.doc.notes != before {
        app.dirty = true;
    }
    if !open {
        app.show_notes = false;
    }
}

fn about(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.show_about {
        return;
    }
    let mut open = true;
    egui::Window::new(tr("Über VectorLume"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(420.0)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(
        ctx,
        |ui| {
            let pal = theme::of(ui);
            ui.label(
                RichText::new(format!("{} VectorLume", icon::SPARKLE)).font(theme::strong_font(22.0)).color(pal.accent),
            );
            ui.label(trs(&format!("Version {}", env!("CARGO_PKG_VERSION"))));
            ui.add_space(6.0);
            ui.label(tr("Software zum Gestalten und Lasern für Diodenlaser mit GRBL, Marlin oder Smoothieware. Eigenständig in Rust entwickelt."));
            ui.add_space(6.0);
            ui.label(RichText::new(tr("Tastenkürzel")).strong());
            egui::Grid::new("keys").num_columns(2).show(ui, |ui| {
                for (k, d) in [
                    ("V / R / E / L / T / Q", tr("Auswahl, Rechteck, Ellipse, Linie, Text, Code")),
                    ("N / M", tr("Knoten bearbeiten, Messen")),
                    (tr("Mausrad"), tr("Zoomen")),
                    (tr("Mittlere Maustaste / Leertaste + Ziehen"), tr("Ansicht verschieben")),
                    (tr("Umschalt beim Ziehen"), tr("Seitenverhältnis frei / 15°-Schritte / Achse fixieren")),
                    (tr("Pfeiltasten (+Umschalt)"), tr("Auswahl 1 mm (10 mm) verschieben")),
                    ("Alt+P", tr("Vorschau")),
                    ("F", tr("Arbeitsfläche einpassen")),
                    (tr("Strg+Umschalt+V"), tr("An Ort und Stelle einfügen")),
                    (tr("Bild↑ / Bild↓ (+Strg)"), tr("Auswahl eins (ganz) nach vorne / hinten")),
                ] {
                    ui.label(RichText::new(k).monospace());
                    ui.label(d);
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            ui.label(RichText::new(tr("Privates Projekt – alle Rechte vorbehalten.")).small().color(pal.text_weak));
            ui.label(
                RichText::new(
                    tr("Verwendet freie Bibliotheken (u. a. egui, usvg, lopdf, image, serialport, nokhwa).\nThis software is based in part on the work of the Independent JPEG Group."),
                )
                .small()
                .color(pal.text_weak),
            );
            ui.label(
                RichText::new(tr(
                    "Einlinienschriften: The Hershey Fonts were originally created by Dr. A. V. Hershey while working \
                     at the U. S. National Bureau of Standards. The format of the font data was originally created by \
                     James Hurt, Cognition, Inc.",
                ))
                .small()
                .color(pal.text_weak),
            );
            // Liegt dem Setup bzw. dem portablen Paket bei (tools\package.ps1)
            let licenses = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("THIRD-PARTY-LICENSES.html")));
            if let Some(path) = licenses.filter(|p| p.is_file())
                && ui.link(tr("Lizenzen der Bibliotheken anzeigen")).clicked()
            {
                let _ = std::process::Command::new("explorer").arg(path).spawn();
            }
        },
    );
    if !open {
        app.show_about = false;
    }
}

/// Tabelle Geschwindigkeit → gemessener Versatz.
fn scan_offsets_ui(ui: &mut egui::Ui, rows: &mut Vec<vl_cam::ScanOffset>) {
    ui.vertical(|ui| {
        let mut remove = None;
        for (i, row) in rows.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add(DragValue::new(&mut row.speed).speed(50.0).range(1.0..=100_000.0).mm_per_min());
                ui.add(DragValue::new(&mut row.offset).speed(0.005).range(-5.0..=5.0).max_decimals(3).mm());
                if ui.small_button(icon::X).on_hover_text(tr("Eintrag entfernen")).clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            rows.remove(i);
        }
        ui.horizontal(|ui| {
            if ui.small_button(trs(&format!("{} Messwert", icon::PLUS))).clicked() {
                let speed = rows.iter().map(|r| r.speed).fold(0.0, f64::max);
                let speed = if speed > 0.0 { speed * 2.0 } else { 3000.0 };
                rows.push(vl_cam::ScanOffset { speed, offset: 0.0 });
            }
            if rows.is_empty() {
                ui.label(RichText::new(tr("aus")).color(theme::of(ui).text_weak));
            }
        });
    });
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
                ui.label(RichText::new(trs(&format!("{} Sicherheitshinweis", icon::WARNING))).font(theme::strong_font(18.0)).color(pal.warning));
                ui.add_space(6.0);
                ui.label(tr("Laser dieser Leistungsklasse (Klasse 4) können Augen dauerhaft schädigen und Brände verursachen."));
                ui.add_space(4.0);
                for t in [
                    tr("Immer eine zur Wellenlänge passende Laserschutzbrille tragen (Diodenlaser: ca. 450 nm)."),
                    tr("Den Laser während des Betriebs nie unbeaufsichtigt lassen, Feuerlöscher bereithalten."),
                    tr("Für Absaugung bzw. Belüftung sorgen. Kein PVC/Vinyl lasern (giftige Gase)."),
                    tr("Neue Einstellungen zuerst mit geringer Leistung testen."),
                ] {
                    ui.label(format!("• {t}"));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(Button::new(RichText::new(tr("Verstanden, Job starten")).color(Color32::WHITE)).fill(pal.success)).clicked() {
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
                    if ui.button(tr("Abbrechen")).clicked() {
                        keep = false;
                    }
                });
            }
            Modal::SendFile { name, lines } => {
                ui.label(RichText::new(trs(&format!("{} G-Code-Datei senden", icon::FILE_CODE))).font(theme::strong_font(18.0)).color(pal.warning));
                ui.add_space(6.0);
                ui.label(trs(&format!("„{name}“ mit {} Zeilen an den Laser senden?", lines.len())));
                ui.add_space(4.0);
                ui.label(RichText::new(tr("Der Inhalt wird nicht geprüft: Arbeitsbereich, Leistung und Nullpunkt müssen zu dieser Maschine passen. Laserschutzbrille tragen und den Laser nicht unbeaufsichtigt lassen.")).small().color(pal.text_weak));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(Button::new(RichText::new(tr("Senden")).color(Color32::WHITE)).fill(pal.warning)).clicked() {
                        let (name, lines) = (name.clone(), lines.clone());
                        action = Some(Box::new(move |app: &mut VectorLumeApp| app.send_gcode_lines(name, &lines)));
                        keep = false;
                    }
                    if ui.button(tr("Abbrechen")).clicked() {
                        keep = false;
                    }
                });
            }
            Modal::ConfirmStart { job } => {
                ui.label(RichText::new(trs(&format!("{} Job trotzdem starten?", icon::WARNING))).font(theme::strong_font(18.0)).color(pal.warning));
                ui.add_space(6.0);
                for w in &job.warnings {
                    ui.label(format!("• {}", trs(w)));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(Button::new(RichText::new(tr("Trotzdem starten")).color(Color32::WHITE)).fill(pal.warning)).clicked() {
                        let job = job.clone();
                        action = Some(Box::new(move |app: &mut VectorLumeApp| app.send_job(&job)));
                        keep = false;
                    }
                    if ui.button(tr("Abbrechen")).clicked() {
                        keep = false;
                    }
                });
            }
            Modal::LaserModeOff => {
                ui.label(RichText::new(trs(&format!("{} Laser-Modus ist ausgeschaltet", icon::WARNING))).font(theme::strong_font(18.0)).color(pal.danger));
                ui.add_space(6.0);
                ui.label(tr("Die Steuerung meldet $32=0. In diesem Zustand bleibt der Laser bei Leerfahrten an und brennt Linien zwischen den Formen. Außerdem hält die Maschine bei jeder Leistungsänderung kurz an."));
                ui.add_space(4.0);
                ui.label(tr("Empfehlung: Laser-Modus mit „$32=1“ dauerhaft einschalten."));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(Button::new(RichText::new(tr("$32=1 senden")).color(Color32::WHITE)).fill(pal.accent)).clicked() {
                        action = Some(Box::new(|app: &mut VectorLumeApp| {
                            app.send_line("$32=1");
                            app.send_line("$$");
                            app.toast(ToastKind::Info, tr("Laser-Modus wird eingeschaltet…"));
                        }));
                        keep = false;
                    }
                    if ui.button(tr("Abbrechen")).clicked() {
                        keep = false;
                    }
                });
            }
            Modal::Unsaved(then) => {
                ui.label(RichText::new(tr("Ungespeicherte Änderungen")).font(theme::strong_font(18.0)));
                ui.add_space(6.0);
                ui.label(tr("Das aktuelle Projekt wurde geändert. Möchtest du es vorher speichern?"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(Button::new(RichText::new(tr("Speichern")).color(Color32::WHITE)).fill(pal.accent)).clicked() {
                        let then = then.clone();
                        action = Some(Box::new(move |app: &mut VectorLumeApp| {
                            if app.save(false) {
                                app.after_discard(then);
                            }
                        }));
                        keep = false;
                    }
                    if ui.button(tr("Verwerfen")).clicked() {
                        let then = then.clone();
                        action = Some(Box::new(move |app: &mut VectorLumeApp| {
                            app.dirty = false;
                            app.after_discard(then);
                        }));
                        keep = false;
                    }
                    if ui.button(tr("Abbrechen")).clicked() {
                        keep = false;
                    }
                });
            }
            Modal::Recover { path, meta } => {
                ui.label(RichText::new(trs(&format!("{} Ungespeicherte Arbeit gefunden", icon::CLOCK_COUNTER_CLOCKWISE))).font(theme::strong_font(18.0)));
                ui.add_space(6.0);
                let of = meta.original.as_deref().map(crate::app::file_name).map(|n| trs(&format!(" von „{n}“"))).unwrap_or_default();
                ui.label(trs(&format!(
                    "VectorLume wurde zuletzt nicht regulär beendet. Es gibt eine automatische Sicherung{of} ({}).",
                    meta.age_label()
                )));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(Button::new(RichText::new(tr("Wiederherstellen")).color(Color32::WHITE)).fill(pal.accent)).clicked() {
                        let (path, original) = (path.clone(), meta.original.clone());
                        action = Some(Box::new(move |app: &mut VectorLumeApp| app.restore_autosave(&path, original)));
                        keep = false;
                    }
                    if ui.button(tr("Verwerfen")).clicked() {
                        let path = path.clone();
                        action = Some(Box::new(move |_: &mut VectorLumeApp| crate::autosave::discard_previous(&path)));
                        keep = false;
                    }
                });
            }
            Modal::Message { title, text } => {
                ui.label(RichText::new(trs(title)).font(theme::strong_font(18.0)));
                ui.add_space(6.0);
                ui.label(trs(text));
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
