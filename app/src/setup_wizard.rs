//! Assistent „Gerät einrichten“: öffnet sich bei jedem Start, solange noch kein Gerät eingerichtet ist, und über
//! *Laser → Gerät einrichten…*. Vier Schritte: Laser aus dem Katalog wählen, verbinden (GRBL: Arbeitsfläche und S-Max
//! werden aus `$$` gelesen), Steuerung/Arbeitsfläche/Nullpunkt prüfen, Namen vergeben. Abbrechen lässt die Geräteliste
//! unverändert.

use crate::app::VectorLumeApp;
use crate::i18n::{tr, trs};
use crate::theme;
use crate::units::UnitField;
use egui::{Align2, Button, Color32, DragValue, RichText, vec2};
use egui_phosphor::regular as icon;
use vl_cam::catalog::{self, GENERIC, MODELS, Model, UNSUPPORTED};
use vl_cam::{MachineProfile, OriginCorner};
use vl_core::Firmware;

const STEPS: usize = 4;

#[derive(Default)]
pub struct SetupWizard {
    pub open: bool,
    pub step: usize,
    pub profile: MachineProfile,
    /// Gewählte Vorlage (Index in [`MODELS`]); ohne Auswahl geht es nicht weiter.
    pub model: Option<usize>,
    pub search: String,
    /// Der Assistent hat die Verbindung aufgebaut (beim Abbrechen wieder trennen).
    connected_here: bool,
    /// Aus der Steuerung übernommene Werte, zur Anzeige.
    pub read: Vec<String>,
}

fn step_title(step: usize) -> &'static str {
    match step {
        0 => tr("Laser"),
        1 => tr("Verbindung"),
        2 => tr("Steuerung & Fläche"),
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

/// Modellname in der Sprache der Oberfläche (Herstellernamen bleiben, allgemeine Einträge werden übersetzt).
fn model_label(m: &Model) -> String {
    trs(m.name)
}

/// Gerätename für das neue Profil, z. B. „Sculpfun S30 Ultra“.
fn device_name(m: &Model) -> String {
    if m.maker == GENERIC { trs(m.name) } else { format!("{} {}", m.maker, trs(m.name)) }
}

impl VectorLumeApp {
    pub fn open_setup_wizard(&mut self) {
        self.setup = SetupWizard { open: true, ..Default::default() };
    }

    /// Ohne eingerichtetes Gerät: Hinweis und Assistent öffnen. `true`, wenn ein Gerät da ist.
    pub fn require_device(&mut self) -> bool {
        if self.settings.has_device() {
            return true;
        }
        self.toast(crate::app::ToastKind::Info, tr("Zuerst den Laser einrichten (Laser → Gerät einrichten…)."));
        if !self.setup.open {
            self.open_setup_wizard();
        }
        false
    }

    fn select_model(&mut self, index: usize) {
        let m = &MODELS[index];
        let w = &mut self.setup;
        w.model = Some(index);
        w.profile = MachineProfile { name: device_name(m), ..m.profile() };
        w.read.clear();
    }

    fn finish_setup(&mut self) {
        let mut p = self.setup.profile.clone();
        if p.name.trim().is_empty() {
            p.name = tr("Mein Laser").into();
        }
        self.settings.add_device(p);
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

/// Schritt 1: durchsuchbare Liste nach Hersteller. Gibt einen angeklickten Eintrag zurück.
fn catalog_ui(ui: &mut egui::Ui, w: &mut SetupWizard) -> Option<usize> {
    let pal = theme::of(ui);
    let mut picked = None;
    ui.label(RichText::new(tr("Welchen Laser möchtest du einrichten?")).strong());
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon::MAGNIFYING_GLASS).color(pal.text_weak));
        ui.add(
            egui::TextEdit::singleline(&mut w.search)
                .hint_text(tr("Suchen, z. B. „Ortur“ oder „S30“"))
                .desired_width(280.0),
        );
    });
    ui.add_space(4.0);
    let hits = catalog::search(&w.search);
    egui::ScrollArea::vertical().max_height(230.0).auto_shrink([false, true]).show(ui, |ui| {
        for maker in catalog::makers() {
            let models: Vec<usize> = hits.iter().copied().filter(|&i| MODELS[i].maker == maker).collect();
            if models.is_empty() {
                continue;
            }
            ui.add_space(2.0);
            ui.label(RichText::new(trs(maker)).small().strong().color(pal.text_weak));
            for i in models {
                let m = &MODELS[i];
                let text = format!("{}   ·   {} × {} mm", model_label(m), m.width, m.height);
                let mut resp = ui.selectable_label(w.model == Some(i), text);
                let tip = if m.hint.is_empty() {
                    tr(m.firmware.label()).to_string()
                } else {
                    format!("{} – {}", tr(m.firmware.label()), trs(m.hint))
                };
                resp = resp.on_hover_text(tip);
                if resp.clicked() {
                    picked = Some(i);
                }
            }
        }
        if hits.is_empty() {
            ui.label(
                RichText::new(tr(
                    "Kein Treffer. Fast alle Diodenlaser nutzen GRBL – dann „GRBL-Laser“ unter „Allgemein“ wählen.",
                ))
                .color(pal.text_weak),
            );
        }
    });
    ui.add_space(4.0);
    match w.model.map(|i| &MODELS[i]) {
        Some(m) if !m.hint.is_empty() => {
            ui.label(RichText::new(trs(m.hint)).small().color(pal.text_weak));
        }
        Some(_) => {
            ui.label(
                RichText::new(tr(
                    "Arbeitsfläche und Leistungsbereich werden beim Verbinden aus der Steuerung gelesen.",
                ))
                .small()
                .color(pal.text_weak),
            );
        }
        None => {
            ui.label(
                RichText::new(tr("Dein Modell fehlt? „GRBL-Laser“ passt für fast alle Diodenlaser."))
                    .small()
                    .color(pal.text_weak),
            );
        }
    }
    egui::CollapsingHeader::new(RichText::new(tr("Nicht unterstützte Laser")).small()).id_salt("unsupported").show(
        ui,
        |ui| {
            for (name, why) in UNSUPPORTED {
                ui.label(RichText::new(format!("• {} – {}", trs(name), trs(why))).small().color(pal.text_weak));
            }
        },
    );
    picked
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.setup.open {
        return;
    }
    if app.setup.step == 1 {
        app.read_setup_values();
    }
    let first = !app.settings.has_device();
    let mut open = true;
    let (mut back, mut next, mut connect, mut disconnect, mut later) = (false, false, false, false, false);
    let mut picked = None;
    let mut window = egui::Window::new(trs(&format!("{} Gerät einrichten", icon::MAGIC_WAND)))
        .collapsible(false)
        .resizable(false)
        .default_width(560.0)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center());
    // Ohne Gerät gibt es kein Schließkreuz, nur „Später einrichten“ mit Erklärung
    if !first {
        window = window.open(&mut open);
    }
    window.show(ctx, |ui| {
        let pal = theme::of(ui);
        let step = app.setup.step;
        if first && step == 0 {
            ui.label(RichText::new(tr(
                "Willkommen bei VectorLume! Richte zuerst deinen Laser ein – Arbeitsfläche, Nullpunkt und \
                 Leistungsbereich bestimmen, wo und wie stark jede Linie gelasert wird.",
            ))
            .color(pal.text_weak));
            ui.add_space(4.0);
        }
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
            ui.set_min_height(330.0);
            match step {
                0 => picked = catalog_ui(ui, &mut app.setup),
                1 => {
                    ui.label(tr("Verbinde den Laser jetzt, damit VectorLume Arbeitsfläche und Leistungsbereich aus der \
                                Steuerung liest. Du kannst diesen Schritt auch überspringen."));
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
                    let w = &mut app.setup;
                    egui::Grid::new("setup_bed").num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
                        ui.label(tr("Steuerung"));
                        let before = w.profile.firmware;
                        egui::ComboBox::from_id_salt("setup_firmware")
                            .selected_text(tr(w.profile.firmware.label()))
                            .show_ui(ui, |ui| {
                                for f in Firmware::ALL {
                                    ui.selectable_value(&mut w.profile.firmware, f, tr(f.label()))
                                        .on_hover_text(firmware_hint(f));
                                }
                            });
                        if w.profile.firmware != before && w.profile.s_max == before.default_s_max() {
                            w.profile.s_max = w.profile.firmware.default_s_max();
                        }
                        ui.end_row();
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
                        ui.label(tr("Leistungsmodus"));
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut w.profile.dynamic_power, true, tr("M4 dynamisch"));
                            ui.radio_value(&mut w.profile.dynamic_power, false, tr("M3 konstant"));
                        });
                        ui.end_row();
                    });
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(tr(
                            "M4 senkt die Leistung beim Abbremsen in Ecken – sie brennen nicht stärker ein als gerade \
                             Linien. M3 nur für Steuerungen ohne Laser-Modus.",
                        ))
                        .small()
                        .color(pal.text_weak),
                    );
                }
                _ => {
                    let w = &mut app.setup;
                    ui.horizontal(|ui| {
                        ui.label(tr("Name"));
                        ui.add(egui::TextEdit::singleline(&mut w.profile.name).desired_width(260.0));
                    });
                    ui.add_space(8.0);
                    let p = &w.profile;
                    let mode = if p.dynamic_power { tr("M4 dynamisch") } else { tr("M3 konstant") };
                    for line in [
                        trs(&format!("Steuerung: {}", tr(p.firmware.label()))),
                        trs(&format!("Arbeitsfläche: {} × {} mm", p.bed_width, p.bed_height)),
                        trs(&format!("Nullpunkt: {}", tr(p.origin.label()))),
                        trs(&format!("S-Max: {}", p.s_max)),
                        trs(&format!("Leistungsmodus: {mode}")),
                    ] {
                        ui.label(format!("• {line}"));
                    }
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(tr(
                            "Tipp: Nach dem Verbinden mit „Rahmen“ prüfen, ob der Laser dort fährt, wo das Motiv liegt. \
                             Ist es gespiegelt, stimmt die Nullpunkt-Ecke nicht. Für maßgenaue Teile anschließend \
                             Laser → Umkehrspiel messen… und die Schnittbreite ermitteln.",
                        ))
                        .small()
                        .color(pal.text_weak),
                    );
                    ui.add_space(4.0);
                    let note = if first {
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
            let can_go = step > 0 || app.setup.model.is_some();
            next = ui
                .add_enabled(can_go, Button::new(RichText::new(label).color(Color32::WHITE)).fill(pal.accent))
                .on_disabled_hover_text(tr("Bitte zuerst einen Laser aus der Liste wählen"))
                .clicked();
            if first {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    later = ui
                        .button(tr("Später einrichten"))
                        .on_hover_text(tr(
                            "Ohne Gerät kannst du entwerfen, aber weder verbinden noch G-Code speichern. Der Assistent \
                             erscheint beim nächsten Start wieder.",
                        ))
                        .clicked();
                });
            }
        });
    });
    if let Some(i) = picked {
        app.select_model(i);
    }
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
    if !open || later {
        app.cancel_setup();
    }
}
