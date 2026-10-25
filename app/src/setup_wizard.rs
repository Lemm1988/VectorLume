//! Assistent „Gerät einrichten“: führt beim ersten Start und über *Laser → Gerät einrichten…* in vier Schritten
//! durch Steuerung, Verbindung (GRBL: Arbeitsfläche und S-Max werden aus `$$` gelesen), Arbeitsfläche mit
//! Nullpunkt und Namen. Abbrechen lässt die Geräteliste unverändert.

use crate::app::VectorLumeApp;
use crate::i18n::{tr, trs};
use crate::theme;
use crate::units::UnitField;
use egui::{Align2, Button, Color32, DragValue, RichText, vec2};
use egui_phosphor::regular as icon;
use vl_cam::{MachineProfile, OriginCorner};
use vl_core::Firmware;

const STEPS: usize = 4;

#[derive(Default)]
pub struct SetupWizard {
    pub open: bool,
    pub step: usize,
    /// Beim ersten Start das vorhandene Standardgerät ersetzen statt ein weiteres anzulegen.
    pub replace: bool,
    pub profile: MachineProfile,
    /// Der Assistent hat die Verbindung aufgebaut (beim Abbrechen wieder trennen).
    connected_here: bool,
    /// Aus der Steuerung übernommene Werte, zur Anzeige.
    pub read: Vec<String>,
}

fn step_title(step: usize) -> &'static str {
    match step {
        0 => tr("Steuerung"),
        1 => tr("Verbindung"),
        2 => tr("Arbeitsfläche"),
        _ => tr("Name"),
    }
}

fn firmware_hint(f: Firmware) -> &'static str {
    match f {
        Firmware::Grbl => tr(
            "Die meisten Diodenlaser (z. B. Sculpfun, Ortur, Atomstack, Two Trees, Creality Falcon) sowie FluidNC und grblHAL.",
        ),
        Firmware::Marlin => tr("3D-Drucker-Steuerungen mit Laser-Aufsatz (Laser-Inline-Modus)."),
        Firmware::Smoothieware => tr("Smoothieboard und ähnliche, z. B. umgebaute CO₂-Laser."),
    }
}

impl VectorLumeApp {
    pub fn open_setup_wizard(&mut self, replace: bool) {
        self.setup = SetupWizard {
            open: true,
            replace,
            profile: MachineProfile { name: tr("Mein Laser").into(), ..Default::default() },
            ..Default::default()
        };
    }

    fn finish_setup(&mut self) {
        let mut p = self.setup.profile.clone();
        if p.name.trim().is_empty() {
            p.name = tr("Mein Laser").into();
        }
        if self.setup.replace {
            self.settings.profile = p;
        } else {
            self.settings.add_device(p);
        }
        self.setup.open = false;
        self.after_device_change();
    }

    fn cancel_setup(&mut self) {
        if std::mem::take(&mut self.setup.connected_here) && self.machine.connected {
            self.disconnect();
        }
        self.setup.open = false;
    }

    /// Übernimmt Arbeitsfläche und S-Max aus `$$`, sobald die Steuerung sie gemeldet hat. Läuft in jedem Bild des
    /// Verbindungsschritts, weil die Werte zeilenweise eintreffen.
    fn read_setup_values(&mut self) {
        let m = &self.machine;
        if !m.connected || !self.setup.profile.firmware.is_grbl() {
            return;
        }
        let p = &mut self.setup.profile;
        let mut read = Vec::new();
        if let (Some(x), Some(y)) = (m.setting_f64(130), m.setting_f64(131))
            && x > 0.0
            && y > 0.0
        {
            p.bed_width = x;
            p.bed_height = y;
            read.push(trs(&format!("Arbeitsfläche {x} × {y} mm ($130/$131)")));
        }
        if let Some(s) = m.setting_f64(30).filter(|&s| s > 0.0) {
            p.s_max = s;
            read.push(trs(&format!("S-Max {s} ($30)")));
        }
        self.setup.read = read;
    }
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.setup.open {
        return;
    }
    if app.setup.step == 1 {
        app.read_setup_values();
    }
    let mut open = true;
    let (mut back, mut next, mut connect, mut disconnect) = (false, false, false, false);
    egui::Window::new(trs(&format!("{} Gerät einrichten", icon::MAGIC_WAND)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(520.0)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let step = app.setup.step;
            // Schrittanzeige
            ui.horizontal(|ui| {
                for i in 0..STEPS {
                    let color = if i == step {
                        pal.accent
                    } else if i < step {
                        pal.text
                    } else {
                        pal.text_weak
                    };
                    let text = RichText::new(format!("{} {}", i + 1, step_title(i))).color(color);
                    ui.label(if i == step { text.strong() } else { text });
                    if i + 1 < STEPS {
                        ui.label(RichText::new(icon::CARET_RIGHT).color(pal.text_weak));
                    }
                }
            });
            ui.separator();
            // feste Mindesthöhe des Inhalts: die Knöpfe bleiben in jedem Schritt an derselben Stelle
            ui.vertical(|ui| {
                ui.set_min_height(250.0);
                let w = &mut app.setup;
                match step {
                    0 => {
                        ui.label(RichText::new(tr("Welche Steuerung hat dein Laser?")).strong());
                        ui.add_space(4.0);
                        for f in Firmware::ALL {
                            if ui.radio(w.profile.firmware == f, tr(f.label())).clicked() && w.profile.firmware != f {
                                w.profile.firmware = f;
                                w.profile.s_max = f.default_s_max();
                                w.read.clear();
                            }
                            ui.indent(("fw", f as u8), |ui| {
                                ui.label(RichText::new(firmware_hint(f)).small().color(pal.text_weak));
                            });
                        }
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label(tr("Vorlage:"));
                            if ui.button("Sculpfun SF-A9 40W").clicked() {
                                w.profile = MachineProfile::sculpfun_sf_a9();
                                w.read.clear();
                            }
                        });
                    }
                    1 => {
                        ui.label(
                            tr("Verbinde den Laser jetzt, damit VectorLume Arbeitsfläche und Leistungsbereich aus der \
                                Steuerung liest. Du kannst diesen Schritt auch überspringen."),
                        );
                        ui.add_space(6.0);
                        let connected = app.machine.connected;
                        ui.horizontal(|ui| {
                            let label = if app.selected_port.is_empty() {
                                tr("Anschluss wählen").to_string()
                            } else {
                                app.selected_port.clone()
                            };
                            ui.add_enabled_ui(!connected, |ui| {
                                egui::ComboBox::from_id_salt("setup_port").width(200.0).selected_text(label).show_ui(
                                    ui,
                                    |ui| {
                                        for p in app.ports.clone() {
                                            let text = if p.description.is_empty() {
                                                p.name.clone()
                                            } else {
                                                format!("{} – {}", p.name, p.description)
                                            };
                                            ui.selectable_value(&mut app.selected_port, p.name.clone(), text);
                                        }
                                    },
                                );
                                if ui.button(icon::ARROWS_CLOCKWISE).on_hover_text(tr("Anschlüsse neu suchen")).clicked() {
                                    app.ports = vl_device::available_ports();
                                }
                            });
                            if connected {
                                disconnect = ui.button(trs(&format!("{} Trennen", icon::LINK_BREAK))).clicked();
                            } else {
                                connect = ui
                                    .add(
                                        Button::new(
                                            RichText::new(trs(&format!("{} Verbinden", icon::PLUG))).color(Color32::WHITE),
                                        )
                                        .fill(pal.accent),
                                    )
                                    .clicked();
                            }
                        });
                        if app.selected_port == vl_device::NETWORK_PORT {
                            ui.horizontal(|ui| {
                                ui.label(tr("Adresse"));
                                ui.add_enabled(
                                    !connected,
                                    egui::TextEdit::singleline(&mut app.settings.network_address)
                                        .hint_text("192.168.4.1:23")
                                        .desired_width(200.0),
                                );
                            });
                        }
                        ui.add_space(8.0);
                        let w = &app.setup;
                        if !connected {
                            ui.label(RichText::new(tr("Nicht verbunden.")).color(pal.text_weak));
                        } else if !w.profile.firmware.is_grbl() {
                            ui.label(
                                RichText::new(tr(
                                    "Verbunden. Diese Steuerung meldet keine Arbeitsfläche – bitte im nächsten Schritt eintragen.",
                                ))
                                .color(pal.success),
                            );
                        } else if w.read.is_empty() {
                            ui.label(RichText::new(tr("Verbunden – Einstellungen werden gelesen…")).color(pal.text_weak));
                        } else {
                            ui.label(RichText::new(tr("Verbunden. Aus der Steuerung übernommen:")).color(pal.success));
                            for r in &w.read {
                                ui.label(format!("• {r}"));
                            }
                        }
                        if connected && w.profile.firmware.is_grbl() && app.machine.laser_mode() == Some(false) {
                            ui.label(
                                RichText::new(tr(
                                    "Der Laser-Modus ($32) ist aus: an Ecken und bei Stopps würde der Laser weiterbrennen. \
                                     Unter Laser → GRBL-Parameter auf 1 stellen.",
                                ))
                                .color(pal.warning),
                            );
                        }
                    }
                    2 => {
                        egui::Grid::new("setup_bed").num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
                            ui.label(tr("Breite"));
                            ui.add(DragValue::new(&mut w.profile.bed_width).speed(1.0).range(10.0..=3000.0).mm());
                            ui.end_row();
                            ui.label(tr("Höhe"));
                            ui.add(DragValue::new(&mut w.profile.bed_height).speed(1.0).range(10.0..=3000.0).mm());
                            ui.end_row();
                            ui.label(tr("Nullpunkt"));
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(tr("Wo steht der Laserkopf nach der Referenzfahrt bzw. bei X0 Y0?"))
                                        .small()
                                        .color(pal.text_weak),
                                );
                                // 2×2 wie die Arbeitsfläche von oben gesehen
                                egui::Grid::new("setup_origin").num_columns(2).show(ui, |ui| {
                                    for row in [
                                        [OriginCorner::RearLeft, OriginCorner::RearRight],
                                        [OriginCorner::FrontLeft, OriginCorner::FrontRight],
                                    ] {
                                        for o in row {
                                            let b = Button::selectable(w.profile.origin == o, tr(o.label()))
                                                .min_size(vec2(120.0, 26.0));
                                            if ui.add(b).clicked() {
                                                w.profile.origin = o;
                                            }
                                        }
                                        ui.end_row();
                                    }
                                });
                            });
                            ui.end_row();
                            ui.label(tr("S-Max"));
                            ui.horizontal(|ui| {
                                ui.add(DragValue::new(&mut w.profile.s_max).speed(1.0).range(0.001..=100000.0));
                                ui.label(
                                    RichText::new(tr("Wert für volle Leistung (GRBL: $30)")).small().color(pal.text_weak),
                                );
                            });
                            ui.end_row();
                        });
                    }
                    _ => {
                        ui.horizontal(|ui| {
                            ui.label(tr("Name"));
                            ui.add(egui::TextEdit::singleline(&mut w.profile.name).desired_width(260.0));
                        });
                        ui.add_space(8.0);
                        let p = &w.profile;
                        for line in [
                            trs(&format!("Steuerung: {}", tr(p.firmware.label()))),
                            trs(&format!("Arbeitsfläche: {} × {} mm", p.bed_width, p.bed_height)),
                            trs(&format!("Nullpunkt: {}", tr(p.origin.label()))),
                            trs(&format!("S-Max: {}", p.s_max)),
                        ] {
                            ui.label(format!("• {line}"));
                        }
                        ui.add_space(8.0);
                        let note = if w.replace {
                            tr("Weitere Werte (Geschwindigkeiten, Luftunterstützung, Drehachse) findest du jederzeit unter \
                                Laser → Maschineneinstellungen.")
                        } else {
                            tr("Das Gerät wird zur Geräteliste hinzugefügt und aktiv. Weitere Werte findest du unter \
                                Laser → Maschineneinstellungen.")
                        };
                        ui.label(RichText::new(note).small().color(pal.text_weak));
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                back = ui.add_enabled(step > 0, Button::new(tr("Zurück"))).clicked();
                let last = step + 1 == STEPS;
                let label = if last {
                    trs(&format!("{} Fertig", icon::CHECK))
                } else if step == 1 && !app.machine.connected {
                    tr("Überspringen").to_string()
                } else {
                    tr("Weiter").to_string()
                };
                next = ui.add(Button::new(RichText::new(label).color(Color32::WHITE)).fill(pal.accent)).clicked();
            });
        });
    if connect {
        let p = app.setup.profile.clone();
        app.setup.read.clear();
        app.setup.connected_here = true;
        app.connect_as(&p);
    }
    if disconnect {
        app.setup.read.clear();
        app.disconnect();
    }
    if back {
        app.setup.step -= 1;
    }
    if next {
        if app.setup.step + 1 == STEPS {
            app.finish_setup();
        } else {
            app.setup.step += 1;
        }
    }
    if !open {
        app.cancel_setup();
    }
}
