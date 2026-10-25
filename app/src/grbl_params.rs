//! GRBL-Parameter: Werte der Steuerung (`$$`) mit Beschreibung anzeigen, ändern und schreiben, als Datei sichern und
//! aus einer Sicherung laden.

use crate::app::{ToastKind, VectorLumeApp};
use crate::i18n::{tr, trs};
use crate::theme;
use egui::{Align2, Button, Color32, RichText, Stroke, TextEdit, vec2};
use egui_phosphor::regular as icon;
use std::collections::{BTreeMap, BTreeSet};
use vl_device::grbl_settings as gs;

#[derive(Default)]
pub struct GrblParams {
    pub open: bool,
    /// Angezeigte Werte: von Hand geänderte bzw. geladene, sonst die der Steuerung.
    pub edited: BTreeMap<u32, String>,
    /// Von Hand geändert oder aus einer Sicherung geladen.
    pub touched: BTreeSet<u32>,
}

impl GrblParams {
    /// Unveränderte Werte folgen der Steuerung.
    fn sync(&mut self, device: &BTreeMap<u32, String>) {
        for (n, v) in device {
            if !self.touched.contains(n) {
                self.edited.insert(*n, v.clone());
            }
        }
    }
}

impl VectorLumeApp {
    fn grbl_ready(&self) -> bool {
        self.machine.connected && !self.machine.job_running && self.firmware().is_grbl()
    }

    pub(crate) fn open_grbl_params(&mut self) {
        self.grbl.touched.clear();
        self.grbl.edited.clear();
        self.grbl.open = true;
        if self.grbl_ready() {
            self.send_line("$$");
        }
    }

    pub(crate) fn write_grbl_params(&mut self) {
        let lines = gs::changes(&self.machine.settings, &self.grbl.edited);
        if lines.is_empty() || !self.grbl_ready() {
            return;
        }
        for line in &lines {
            self.send_line(line.clone());
        }
        self.send_line("$$");
        self.toast(ToastKind::Success, trs(&format!("An die Steuerung gesendet: {}", lines.join(", "))));
    }

    fn save_grbl_backup(&mut self) {
        let Some(path) = crate::system::FileDialog::new()
            .set_title(tr("GRBL-Parameter sichern"))
            .set_file_name(tr("grbl-parameter.txt"))
            .add_filter(tr("Textdatei"), &["txt"])
            .save_file()
        else {
            return;
        };
        match std::fs::write(&path, gs::to_file(&self.machine.settings)) {
            Ok(()) => self.toast(ToastKind::Success, trs(&format!("Gesichert: {}", crate::app::file_name(&path)))),
            Err(e) => self.toast(ToastKind::Error, trs(&format!("Sichern fehlgeschlagen: {e}"))),
        }
    }

    fn load_grbl_backup(&mut self) {
        let Some(path) = crate::system::FileDialog::new()
            .set_title(tr("GRBL-Parameter laden"))
            .add_filter(tr("Textdatei"), &["txt"])
            .pick_file()
        else {
            return;
        };
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| gs::from_file(&t)) {
            Ok(values) => {
                let n = values.len();
                self.grbl.touched.extend(values.keys());
                self.grbl.edited.extend(values);
                self.toast(ToastKind::Info, trs(&format!("{n} Werte geladen – prüfen und dann schreiben.")));
            }
            Err(e) => self.toast(ToastKind::Error, trs(&e)),
        }
    }
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.grbl.open {
        return;
    }
    app.grbl.sync(&app.machine.settings);
    let mut open = true;
    let mut action: Option<fn(&mut VectorLumeApp)> = None;
    let ready = app.grbl_ready();
    egui::Window::new(trs(&format!("{} GRBL-Parameter", icon::SLIDERS_HORIZONTAL)))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(640.0, 540.0))
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let (state, color) = if ready {
                (tr("Verbunden – Werte der Steuerung."), pal.text_weak)
            } else if app.machine.settings.is_empty() {
                (tr("Keine Werte – zuerst mit einer GRBL-Steuerung verbinden."), pal.warning)
            } else {
                (tr("Nicht bereit (getrennt oder Job läuft) – Werte der letzten Abfrage, Schreiben nicht möglich."), pal.warning)
            };
            ui.label(RichText::new(state).small().color(color));
            let device = &app.machine.settings;
            let g = &mut app.grbl;
            let mut changed: Vec<u32> = Vec::new();
            egui::ScrollArea::vertical().max_height(ui.available_height() - 90.0).auto_shrink([false, true]).show(ui, |ui| {
                egui::Grid::new("grbl_params").num_columns(4).striped(true).spacing(vec2(10.0, 4.0)).show(ui, |ui| {
                    for (n, value) in g.edited.iter_mut() {
                        let (desc, unit) = gs::describe(*n).unwrap_or(("–", ""));
                        ui.label(RichText::new(format!("${n}")).monospace());
                        ui.label(tr(desc));
                        let valid = gs::is_valid(value);
                        let differs = gs::differs(device, *n, value);
                        let stroke = if !valid {
                            Stroke::new(1.5, pal.danger)
                        } else if differs {
                            Stroke::new(1.5, pal.accent)
                        } else {
                            Stroke::NONE
                        };
                        let edit = egui::Frame::NONE
                            .stroke(stroke)
                            .corner_radius(4.0)
                            .show(ui, |ui| ui.add(TextEdit::singleline(value).desired_width(110.0).min_size(vec2(110.0, 0.0))))
                            .inner;
                        if edit.changed() {
                            changed.push(*n);
                        }
                        ui.label(RichText::new(tr(unit)).color(pal.text_weak));
                        ui.end_row();
                    }
                });
            });
            g.touched.extend(changed);
            let lines = gs::changes(device, &g.edited);
            let invalid = g.edited.values().any(|v| !gs::is_valid(v));
            ui.separator();
            ui.label(
                RichText::new(tr(
                    "Falsche Werte (z. B. Schritte je mm, Verfahrweg) lassen die Maschine falsch oder gegen den Anschlag fahren. Vorher sichern!",
                ))
                .small()
                .color(pal.text_weak),
            );
            ui.horizontal(|ui| {
                if ui.add_enabled(ready, Button::new(trs(&format!("{} Lesen", icon::ARROW_CLOCKWISE)))).clicked() {
                    action = Some(|app| app.send_line("$$"));
                }
                if ui.add_enabled(!g.touched.is_empty(), Button::new(tr("Änderungen verwerfen"))).clicked() {
                    g.touched.clear();
                    g.edited.clear();
                }
                if ui
                    .add_enabled(!device.is_empty(), Button::new(trs(&format!("{} Sichern…", icon::FLOPPY_DISK))))
                    .clicked()
                {
                    action = Some(VectorLumeApp::save_grbl_backup);
                }
                if ui.button(trs(&format!("{} Laden…", icon::FOLDER_OPEN))).clicked() {
                    action = Some(VectorLumeApp::load_grbl_backup);
                }
                let write = Button::new(
                    RichText::new(trs(&format!("Schreiben ({})", lines.len()))).color(Color32::WHITE),
                )
                .fill(pal.accent);
                if ui.add_enabled(ready && !lines.is_empty() && !invalid, write).clicked() {
                    action = Some(VectorLumeApp::write_grbl_params);
                }
            });
        });
    if let Some(f) = action {
        f(app);
    }
    if !open {
        app.grbl.open = false;
    }
}
