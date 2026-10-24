//! Rechte Seitenleiste: Ebenen mit Laser-Einstellungen, Laser-Steuerung, Verfahren und Konsole.

use crate::app::VectorLumeApp;
use crate::theme;
use egui::{Align, Button, Color32, CornerRadius, DragValue, Frame, Layout, Margin, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;
use vl_cam::{Anchor, StartFrom};
use vl_core::{CutSettings, DitherMode, LayerMode, Point, Shape};
use vl_device::{Command, Direction, MachineState};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RightTab {
    Laser,
    Jog,
    Console,
}

fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let pal = theme::of(ui);
    Frame::NONE
        .fill(pal.panel_alt)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn section_title(ui: &mut Ui, glyph: &str, text: &str) {
    let pal = theme::of(ui);
    ui.label(RichText::new(format!("{glyph}  {text}")).font(theme::strong_font(14.5)).color(pal.text));
}

pub fn right_panel(app: &mut VectorLumeApp, ui: &mut Ui) {
    let pal = theme::of(ui);
    egui::Panel::right("side")
        .resizable(true)
        .default_size(350.0)
        .size_range(300.0..=560.0)
        .frame(Frame::NONE.fill(pal.panel).inner_margin(Margin::same(10)).stroke(Stroke::new(1.0, pal.border)))
        .show(ui, |ui| {
            let half = ui.available_height() * 0.52;
            egui::Panel::top("layers_section")
                .resizable(true)
                .default_size(half)
                .size_range(160.0..=ui.available_height() - 160.0)
                .frame(Frame::NONE.inner_margin(Margin { left: 0, right: 0, top: 0, bottom: 8 }))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("layers_scroll").auto_shrink([false, false]).show(ui, |ui| {
                        layers_ui(app, ui);
                    });
                });
            egui::CentralPanel::default()
                .frame(Frame::NONE.inner_margin(Margin { left: 0, right: 0, top: 8, bottom: 0 }))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (tab, glyph, label) in [
                            (RightTab::Laser, icon::LIGHTNING, "Laser"),
                            (RightTab::Jog, icon::ARROWS_OUT, "Bewegen"),
                            (RightTab::Console, icon::TERMINAL, "Konsole"),
                        ] {
                            let selected = app.right_tab == tab;
                            let text = RichText::new(format!("{glyph} {label}")).color(if selected {
                                pal.accent
                            } else {
                                pal.text
                            });
                            if ui.add(Button::selectable(selected, text).min_size(vec2(0.0, 28.0))).clicked() {
                                app.right_tab = tab;
                            }
                        }
                    });
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().id_salt("tab_scroll").auto_shrink([false, false]).show(ui, |ui| {
                        match app.right_tab {
                            RightTab::Laser => laser_ui(app, ui),
                            RightTab::Jog => jog_ui(app, ui),
                            RightTab::Console => console_ui(app, ui),
                        }
                    });
                });
        });
}

// ------------------------------------------------------------------ Ebenen

fn layers_ui(app: &mut VectorLumeApp, ui: &mut Ui) {
    let pal = theme::of(ui);
    section_title(ui, icon::STACK, "Ebenen");
    ui.add_space(4.0);
    let used = app.doc.used_layers();
    if used.is_empty() {
        ui.label(
            RichText::new("Noch keine Objekte. Zeichne links eine Form oder importiere eine Datei (Strg+I).")
                .color(pal.text_weak),
        );
    }
    for (pos, &l) in used.iter().enumerate() {
        let layer = app.doc.layers[l].clone();
        let selected = app.current_layer == l;
        let fill = if selected { pal.widget } else { pal.panel_alt };
        let resp = Frame::NONE
            .fill(fill)
            .corner_radius(CornerRadius::same(8))
            .stroke(Stroke::new(1.0, if selected { pal.accent.gamma_multiply(0.7) } else { pal.border }))
            .inner_margin(Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                    ui.painter().rect_filled(r, CornerRadius::same(4), theme::layer_color(layer.color));
                    ui.label(RichText::new(&layer.name).strong());
                    ui.label(RichText::new(layer.settings.mode.label()).color(pal.text_weak).size(12.0));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if ui.small_button(icon::CARET_DOWN).on_hover_text("Später ausgeben").clicked() {
                            app.checkpoint();
                            app.doc.move_layer(l, 1);
                            app.mark_changed();
                        }
                        if ui
                            .add_enabled(pos > 0, Button::new(icon::CARET_UP).small())
                            .on_hover_text("Früher ausgeben")
                            .clicked()
                        {
                            app.checkpoint();
                            app.doc.move_layer(l, -1);
                            app.mark_changed();
                        }
                        let vis = if layer.visible { icon::EYE } else { icon::EYE_SLASH };
                        if ui.small_button(vis).on_hover_text("Anzeigen/Ausblenden").clicked() {
                            app.checkpoint();
                            app.doc.layers[l].visible = !layer.visible;
                            app.mark_changed();
                        }
                        let out_color = if layer.settings.output { pal.success } else { pal.text_weak };
                        if ui
                            .small_button(RichText::new(icon::POWER).color(out_color))
                            .on_hover_text(if layer.settings.output {
                                "Wird gelasert (klicken zum Abschalten)"
                            } else {
                                "Wird nicht gelasert"
                            })
                            .clicked()
                        {
                            app.checkpoint();
                            app.doc.layers[l].settings.output = !layer.settings.output;
                            app.mark_changed();
                        }
                    });
                });
                ui.label(
                    RichText::new(format!(
                        "{:.0} mm/min · {:.0} % · {}× {}",
                        layer.settings.speed,
                        layer.settings.power_max,
                        layer.settings.passes,
                        if layer.settings.air_assist { "· Luft" } else { "" }
                    ))
                    .size(12.0)
                    .color(pal.text_weak),
                );
            })
            .response;
        if resp.interact(Sense::click()).clicked() {
            app.current_layer = l;
        }
        ui.add_space(3.0);
    }

    ui.add_space(6.0);
    let l = app.current_layer;
    let has_images = app.doc.objects_on_layer(l).any(|o| o.is_image());
    let mut layer = app.doc.layers[l].clone();
    card(ui, |ui| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(4), theme::layer_color(layer.color));
            ui.add(egui::TextEdit::singleline(&mut layer.name).desired_width(120.0));
            ui.label(RichText::new("Einstellungen").color(pal.text_weak));
        });
        ui.add_space(4.0);
        cut_settings_ui(ui, &mut layer.settings, has_images);
    });
    if layer != app.doc.layers[l] {
        app.checkpoint_coalesced("layer");
        app.doc.layers[l] = layer;
        app.mark_changed();
    }

    image_adjust_ui(app, ui);
}

fn cut_settings_ui(ui: &mut Ui, s: &mut CutSettings, has_images: bool) {
    egui::Grid::new("cut_settings").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.label("Modus");
        let before = s.mode;
        egui::ComboBox::from_id_salt("layer_mode").selected_text(s.mode.label()).show_ui(ui, |ui| {
            for m in LayerMode::ALL {
                ui.selectable_value(&mut s.mode, m, m.label());
            }
        });
        if s.mode != before {
            // Sinnvolle Startwerte für den neuen Modus übernehmen, Ausgabe/Luft beibehalten.
            let keep = (s.output, s.air_assist);
            *s = CutSettings::default_for(s.mode);
            (s.output, s.air_assist) = keep;
        }
        ui.end_row();

        ui.label("Geschwindigkeit");
        ui.add(DragValue::new(&mut s.speed).speed(10.0).range(1.0..=60000.0).suffix(" mm/min"));
        ui.end_row();

        ui.label("Leistung max.");
        ui.add(egui::Slider::new(&mut s.power_max, 0.0..=100.0).suffix(" %").fixed_decimals(1));
        ui.end_row();

        let raster = s.mode == LayerMode::Image || has_images;
        if raster && s.dither == DitherMode::Grayscale {
            ui.label("Leistung min.");
            ui.add(egui::Slider::new(&mut s.power_min, 0.0..=100.0).suffix(" %").fixed_decimals(1));
            ui.end_row();
            if s.power_min > s.power_max {
                s.power_min = s.power_max;
            }
        }

        ui.label("Durchgänge");
        ui.add(DragValue::new(&mut s.passes).range(1..=100));
        ui.end_row();

        if s.mode != LayerMode::Line || has_images {
            ui.label("Zeilenabstand");
            ui.horizontal(|ui| {
                ui.add(DragValue::new(&mut s.interval).speed(0.005).range(0.02..=2.0).max_decimals(3).suffix(" mm"));
                ui.label(
                    RichText::new(format!("≙ {:.0} dpi", 25.4 / s.interval.max(0.001)))
                        .small()
                        .color(theme::of(ui).text_weak),
                );
            });
            ui.end_row();
            ui.label("Overscan");
            ui.add(DragValue::new(&mut s.overscan).speed(0.1).range(0.0..=20.0).suffix(" mm"));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut s.bidirectional, "Bidirektional");
            ui.end_row();
        }
        if s.mode == LayerMode::Fill {
            ui.label("Winkel");
            ui.add(DragValue::new(&mut s.fill_angle).speed(1.0).range(-180.0..=180.0).suffix("°"));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut s.crosshatch, "Kreuzschraffur");
            ui.end_row();
        }
        if raster {
            ui.label("Bildmodus");
            egui::ComboBox::from_id_salt("dither").selected_text(s.dither.label()).show_ui(ui, |ui| {
                for d in DitherMode::ALL {
                    ui.selectable_value(&mut s.dither, d, d.label());
                }
            });
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut s.negative, "Negativ (invertieren)");
            ui.end_row();
        }
        ui.label("");
        ui.checkbox(&mut s.air_assist, "Luftunterstützung");
        ui.end_row();
        ui.label("");
        ui.checkbox(&mut s.output, "Ausgeben");
        ui.end_row();
    });
}

fn image_adjust_ui(app: &mut VectorLumeApp, ui: &mut Ui) {
    let [id] = app.selection.as_slice() else { return };
    let id = *id;
    let Some(Shape::Image(img)) = app.doc.get(id).map(|o| o.shape.clone()) else { return };
    ui.add_space(8.0);
    let mut adj = img.adjust;
    card(ui, |ui| {
        section_title(ui, icon::IMAGE, "Bild anpassen");
        ui.label(
            RichText::new(format!("{} × {} px", img.image.width, img.image.height))
                .small()
                .color(theme::of(ui).text_weak),
        );
        egui::Grid::new("img_adjust").num_columns(2).show(ui, |ui| {
            ui.label("Helligkeit");
            ui.add(egui::Slider::new(&mut adj.brightness, -100.0..=100.0).fixed_decimals(0));
            ui.end_row();
            ui.label("Kontrast");
            ui.add(egui::Slider::new(&mut adj.contrast, -100.0..=100.0).fixed_decimals(0));
            ui.end_row();
            ui.label("Gamma");
            ui.add(egui::Slider::new(&mut adj.gamma, 0.2..=5.0).logarithmic(true).fixed_decimals(2));
            ui.end_row();
        });
        if ui.button("Zurücksetzen").clicked() {
            adj = vl_core::ImageAdjust::default();
        }
    });
    if adj != img.adjust {
        app.checkpoint_coalesced("image_adjust");
        if let Some(o) = app.doc.get_mut(id) {
            if let Shape::Image(i) = &mut o.shape {
                i.adjust = adj;
            }
        }
        app.mark_changed();
    }
}

// ------------------------------------------------------------------ Laser

fn colored_button(ui: &mut Ui, text: String, fill: Color32, enabled: bool) -> bool {
    let b = Button::new(RichText::new(text).color(Color32::WHITE).strong())
        .fill(fill)
        .min_size(vec2(ui.available_width(), 34.0));
    ui.add_enabled(enabled, b).clicked()
}

fn laser_ui(app: &mut VectorLumeApp, ui: &mut Ui) {
    let pal = theme::of(ui);
    let connected = app.machine.connected;
    let running = app.machine.job_running;

    card(ui, |ui| {
        section_title(ui, icon::PLUGS, "Verbindung");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let current = app.ports.iter().find(|p| p.name == app.selected_port);
            let label = current.map(|p| p.name.clone()).unwrap_or_else(|| "Anschluss wählen".into());
            ui.add_enabled_ui(!connected, |ui| {
                egui::ComboBox::from_id_salt("port").width(150.0).selected_text(label).show_ui(ui, |ui| {
                    for p in app.ports.clone() {
                        let text = if p.description.is_empty() {
                            p.name.clone()
                        } else {
                            format!("{} – {}", p.name, p.description)
                        };
                        ui.selectable_value(&mut app.selected_port, p.name.clone(), text);
                    }
                });
                if ui.button(icon::ARROWS_CLOCKWISE).on_hover_text("Anschlüsse neu suchen").clicked() {
                    app.ports = vl_device::available_ports();
                }
            });
            if connected {
                if ui.button(format!("{} Trennen", icon::LINK_BREAK)).clicked() {
                    app.disconnect();
                }
            } else if ui
                .add(
                    Button::new(RichText::new(format!("{} Verbinden", icon::PLUG)).color(Color32::WHITE))
                        .fill(pal.accent),
                )
                .clicked()
            {
                app.connect();
            }
        });
        if connected {
            ui.add_space(4.0);
            let state = app.machine.status.as_ref().map(|s| s.state).unwrap_or_default();
            let color = match state {
                MachineState::Alarm | MachineState::Door => pal.danger,
                MachineState::Hold => pal.warning,
                MachineState::Run | MachineState::Jog | MachineState::Home => pal.accent,
                _ => pal.success,
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("● {}", state.label())).color(color).strong());
                if let Some(p) = app.machine.head_position() {
                    ui.label(RichText::new(format!("X {:.2}  Y {:.2}", p.x, p.y)).monospace());
                }
            });
            match app.machine.laser_mode() {
                Some(true) => ui.label(
                    RichText::new(format!("{} Laser-Modus aktiv ($32=1)", icon::CHECK)).small().color(pal.success),
                ),
                Some(false) => ui.label(
                    RichText::new(format!("{} Laser-Modus aus ($32=0) – Leerfahrten könnten brennen!", icon::WARNING))
                        .small()
                        .color(pal.danger),
                ),
                None => ui.label(RichText::new("Einstellungen werden gelesen…").small().color(pal.text_weak)),
            };
            if state == MachineState::Alarm {
                ui.label(RichText::new("Alarm: Referenzfahrt oder Entsperren nötig.").small().color(pal.danger));
            }
        }
    });

    ui.add_space(8.0);
    card(ui, |ui| {
        section_title(ui, icon::TARGET, "Maschine");
        ui.add_space(4.0);
        let w = (ui.available_width() - 8.0) / 2.0;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    connected && !running,
                    Button::new(format!("{} Referenzfahrt", icon::HOUSE)).min_size(vec2(w, 28.0)),
                )
                .clicked()
            {
                app.send_line("$H");
            }
            if ui
                .add_enabled(
                    connected && !running,
                    Button::new(format!("{} Entsperren", icon::LOCK_OPEN)).min_size(vec2(w, 28.0)),
                )
                .clicked()
            {
                app.send_line("$X");
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    connected && !running,
                    Button::new(format!("{} Rahmen", icon::FRAME_CORNERS)).min_size(vec2(w, 28.0)),
                )
                .on_hover_text("Den Umriss des Jobs abfahren (Laser aus)")
                .clicked()
            {
                app.frame_job();
            }
            if ui
                .add_enabled(
                    connected && !running,
                    Button::new(format!("{} Ursprung hier", icon::CROSSHAIR)).min_size(vec2(w, 28.0)),
                )
                .on_hover_text("Aktuelle Kopfposition als Benutzer-Ursprung speichern")
                .clicked()
            {
                app.set_user_origin_here();
            }
        });
    });

    ui.add_space(8.0);
    card(ui, |ui| {
        section_title(ui, icon::PLAY, "Job");
        ui.add_space(4.0);
        let mut js = app.settings.job.clone();
        egui::Grid::new("job_settings").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
            ui.label("Start von");
            egui::ComboBox::from_id_salt("start_from").selected_text(js.start_from.label()).show_ui(ui, |ui| {
                for s in StartFrom::ALL {
                    ui.selectable_value(&mut js.start_from, s, s.label());
                }
            });
            ui.end_row();
            if js.start_from != StartFrom::Absolute {
                ui.label("Ankerpunkt");
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                    for row in Anchor::GRID {
                        ui.horizontal(|ui| {
                            for a in row {
                                let sel = js.anchor == a;
                                let glyph = if sel { "●" } else { "○" };
                                if ui.add(Button::selectable(sel, glyph).min_size(vec2(22.0, 20.0))).clicked() {
                                    js.anchor = a;
                                }
                            }
                        });
                    }
                });
                ui.end_row();
            }
            if js.start_from == StartFrom::UserOrigin {
                ui.label("Ursprung");
                ui.horizontal(|ui| {
                    ui.add(DragValue::new(&mut js.user_origin.x).speed(0.5).prefix("X ").suffix(" mm"));
                    ui.add(DragValue::new(&mut js.user_origin.y).speed(0.5).prefix("Y ").suffix(" mm"));
                });
                ui.end_row();
            }
        });
        ui.checkbox(&mut js.selected_only, "Nur Auswahl ausgeben");
        ui.checkbox(&mut js.engrave_first, "Gravieren vor Schneiden")
            .on_hover_text("Bild- und Füllebenen zuerst, Linienebenen (Schnitte) zuletzt");
        ui.checkbox(&mut js.optimize_order, "Schnittreihenfolge optimieren");
        ui.add_enabled(js.optimize_order, egui::Checkbox::new(&mut js.inner_first, "Innen vor außen schneiden"));
        if js != app.settings.job {
            app.settings.job = js;
            app.settings.save();
            app.revision += 1;
        }

        ui.add_space(8.0);
        if running {
            let (acked, total) = app.machine.job_progress;
            let frac = if total > 0 { acked as f32 / total as f32 } else { 0.0 };
            let elapsed = app.machine.job_started.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0);
            ui.label(RichText::new(&app.machine.job_name).strong());
            ui.add(egui::ProgressBar::new(frac).show_percentage());
            let remaining = (app.machine.job_estimate - elapsed).max(0.0);
            ui.label(
                RichText::new(format!(
                    "Läuft seit {} · Rest ca. {}",
                    vl_cam::format_duration(elapsed),
                    vl_cam::format_duration(remaining)
                ))
                .small()
                .color(pal.text_weak),
            );
            ui.add_space(4.0);
            let paused = app.machine.status.as_ref().is_some_and(|s| s.state == MachineState::Hold);
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 8.0) / 2.0;
                let (label, cmd) = if paused {
                    (format!("{} Fortsetzen", icon::PLAY), Command::Resume)
                } else {
                    (format!("{} Pause", icon::PAUSE), Command::Pause)
                };
                if ui.add(Button::new(label).min_size(vec2(w, 34.0))).clicked() {
                    app.device.send(cmd);
                }
                if ui
                    .add(
                        Button::new(RichText::new(format!("{} Stopp", icon::STOP)).color(Color32::WHITE).strong())
                            .fill(pal.danger)
                            .min_size(vec2(w, 34.0)),
                    )
                    .clicked()
                {
                    app.device.send(Command::Stop);
                }
            });
        } else {
            if colored_button(
                ui,
                format!("{}  Job starten", icon::PLAY),
                pal.success,
                connected && !app.doc.objects.is_empty(),
            ) {
                app.start_job();
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 8.0) / 2.0;
                if ui
                    .add_enabled(
                        connected,
                        Button::new(RichText::new(format!("{} Stopp", icon::STOP)).color(Color32::WHITE))
                            .fill(pal.danger)
                            .min_size(vec2(w, 28.0)),
                    )
                    .clicked()
                {
                    app.device.send(Command::Stop);
                }
                if ui
                    .add_enabled(
                        !app.doc.objects.is_empty(),
                        Button::new(format!("{} G-Code", icon::EXPORT)).min_size(vec2(w, 28.0)),
                    )
                    .clicked()
                {
                    app.export_gcode_dialog();
                }
            });
        }
    });

    if app.preview.active {
        ui.add_space(8.0);
        preview_info(app, ui);
    }
}

fn preview_info(app: &mut VectorLumeApp, ui: &mut Ui) {
    let pal = theme::of(ui);
    card(ui, |ui| {
        section_title(ui, icon::EYE, "Vorschau");
        ui.add_space(4.0);
        if let Some(err) = &app.preview.error {
            ui.label(RichText::new(err).color(pal.warning));
            return;
        }
        let Some(job) = app.preview.job.clone() else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Berechne Werkzeugweg…");
            });
            return;
        };
        egui::Grid::new("preview_stats").num_columns(2).show(ui, |ui| {
            ui.label("Geschätzte Dauer");
            ui.label(RichText::new(job.estimate.format_duration()).strong());
            ui.end_row();
            ui.label("Laserweg");
            ui.label(format!("{:.2} m", job.estimate.burn_distance / 1000.0));
            ui.end_row();
            ui.label("Leerfahrten");
            ui.label(format!("{:.2} m", job.estimate.travel_distance / 1000.0));
            ui.end_row();
            ui.label("G-Code-Zeilen");
            ui.label(format!("{}", job.gcode.len()));
            ui.end_row();
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Abspielen").small().color(pal.text_weak));
        ui.add(egui::Slider::new(&mut app.preview.progress, 0.0..=1.0).show_value(false));
        if ui.checkbox(&mut app.settings.show_travel, "Leerfahrten anzeigen").changed() {
            app.settings.save();
        }
        for w in &job.warnings {
            ui.label(RichText::new(format!("{} {w}", icon::WARNING)).small().color(pal.warning));
        }
    });
}

// ------------------------------------------------------------------ Verfahren

fn jog_ui(app: &mut VectorLumeApp, ui: &mut Ui) {
    let pal = theme::of(ui);
    let enabled = app.machine.connected && !app.machine.job_running;
    card(ui, |ui| {
        section_title(ui, icon::ARROWS_OUT, "Verfahren");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Schritt");
            for s in [0.1, 1.0, 5.0, 10.0, 50.0] {
                if ui.selectable_label((app.settings.jog_step - s).abs() < 1e-9, format!("{s}")).clicked() {
                    app.settings.jog_step = s;
                    app.settings.save();
                }
            }
            ui.label("mm");
        });
        ui.horizontal(|ui| {
            ui.label("Geschwindigkeit");
            if ui
                .add(
                    DragValue::new(&mut app.settings.profile.jog_speed)
                        .speed(50.0)
                        .range(10.0..=30000.0)
                        .suffix(" mm/min"),
                )
                .changed()
            {
                app.settings.save();
            }
        });
        ui.add_space(8.0);
        ui.add_enabled_ui(enabled, |ui| {
            let size = vec2(52.0, 44.0);
            let pad = ((ui.available_width() - 3.0 * size.x - 8.0) / 2.0).max(0.0);
            let dirs = [
                [(-1.0, -1.0, icon::ARROW_UP_LEFT), (0.0, -1.0, icon::ARROW_UP), (1.0, -1.0, icon::ARROW_UP_RIGHT)],
                [(-1.0, 0.0, icon::ARROW_LEFT), (0.0, 0.0, icon::HOUSE), (1.0, 0.0, icon::ARROW_RIGHT)],
                [(-1.0, 1.0, icon::ARROW_DOWN_LEFT), (0.0, 1.0, icon::ARROW_DOWN), (1.0, 1.0, icon::ARROW_DOWN_RIGHT)],
            ];
            for row in dirs {
                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    for (dx, dy, glyph) in row {
                        let b = Button::new(RichText::new(glyph).size(20.0)).min_size(size);
                        let tip = if dx == 0.0 && dy == 0.0 { "Zum Nullpunkt (X0 Y0)" } else { "Verfahren" };
                        if ui.add(b).on_hover_text(tip).clicked() {
                            if dx == 0.0 && dy == 0.0 {
                                let f = app.settings.profile.jog_speed.round();
                                app.send_line(format!("$J=G90 G21 X0 Y0 F{f}"));
                            } else {
                                app.jog(Point::new(dx, dy));
                            }
                        }
                    }
                });
            }
            ui.add_space(6.0);
            if ui.button(format!("{} Bewegung abbrechen", icon::HAND_PALM)).clicked() {
                app.device.send(Command::Realtime(vl_device::RT_JOG_CANCEL));
            }
        });
        if !app.machine.connected {
            ui.label(RichText::new("Zum Verfahren bitte zuerst verbinden.").small().color(pal.text_weak));
        }
    });
}

// ------------------------------------------------------------------ Konsole

fn console_ui(app: &mut VectorLumeApp, ui: &mut Ui) {
    let pal = theme::of(ui);
    ui.horizontal(|ui| {
        for (cmd, tip) in
            [("$$", "Einstellungen"), ("$I", "Firmware"), ("$G", "G-Code-Zustand"), ("$#", "Koordinatensysteme")]
        {
            if ui.add_enabled(app.machine.connected, Button::new(cmd)).on_hover_text(tip).clicked() {
                app.send_line(cmd);
            }
        }
        if ui.button(icon::BROOM).on_hover_text("Konsole leeren").clicked() {
            app.console.clear();
        }
    });
    ui.add_space(4.0);
    let height = (ui.available_height() - 40.0).max(160.0);
    Frame::NONE.fill(pal.bg).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(8)).show(ui, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("console")
            .max_height(height)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for line in &app.console {
                    let (prefix, color) = match line.dir {
                        Direction::Sent => ("› ", pal.accent),
                        Direction::Received => ("", pal.text),
                        Direction::Info => ("• ", pal.text_weak),
                    };
                    let color = if line.text.starts_with("error") || line.text.starts_with("ALARM") {
                        pal.danger
                    } else {
                        color
                    };
                    ui.label(RichText::new(format!("{prefix}{}", line.text)).monospace().color(color));
                }
            });
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let resp = ui.add(
            egui::TextEdit::singleline(&mut app.console_input)
                .hint_text("Befehl, z. B. $$ oder G0 X10")
                .desired_width(ui.available_width() - 70.0),
        );
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Senden").clicked() || enter) && !app.console_input.trim().is_empty() {
            let cmd = std::mem::take(&mut app.console_input);
            app.send_line(cmd.trim().to_string());
            resp.request_focus();
        }
    });
}
