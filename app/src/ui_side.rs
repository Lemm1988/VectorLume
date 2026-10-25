//! Rechte Seitenleiste: Ebenen mit Laser-Einstellungen, Laser-Steuerung, Verfahren und Konsole.

use crate::app::VectorLumeApp;
use crate::i18n::{tr, trs};
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

pub(crate) fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
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
                            (RightTab::Laser, icon::LIGHTNING, tr("Laser")),
                            (RightTab::Jog, icon::ARROWS_OUT, tr("Bewegen")),
                            (RightTab::Console, icon::TERMINAL, tr("Konsole")),
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
    section_title(ui, icon::STACK, tr("Ebenen"));
    ui.add_space(4.0);
    let used = app.doc.used_layers();
    if used.is_empty() {
        ui.label(
            RichText::new(tr("Noch keine Objekte. Zeichne links eine Form oder importiere eine Datei (Strg+I)."))
                .color(pal.text_weak),
        );
    }
    for (pos, &l) in used.iter().enumerate() {
        let layer = app.doc.layers[l].clone();
        let selected = app.current_layer == l;
        let fill = if selected { pal.widget } else { pal.panel_alt };
        // Die Zeile reagiert auf Klicks, wird aber vor den Schaltflächen angemeldet, damit diese oben liegen.
        let resp = ui
            .scope_builder(egui::UiBuilder::new().sense(Sense::click()), |ui| {
                ui.style_mut().interaction.selectable_labels = false;
                Frame::NONE
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
                            let modes: Vec<&str> = layer.mode_labels().into_iter().map(tr).collect();
                            ui.label(RichText::new(modes.join(" + ")).color(pal.text_weak).size(12.0));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                if ui.small_button(icon::CARET_DOWN).on_hover_text(tr("Später ausgeben")).clicked() {
                                    app.checkpoint();
                                    app.doc.move_layer(l, 1);
                                    app.mark_changed();
                                }
                                if ui
                                    .add_enabled(pos > 0, Button::new(icon::CARET_UP).small())
                                    .on_hover_text(tr("Früher ausgeben"))
                                    .clicked()
                                {
                                    app.checkpoint();
                                    app.doc.move_layer(l, -1);
                                    app.mark_changed();
                                }
                                let lock = if layer.locked { icon::LOCK_SIMPLE } else { icon::LOCK_SIMPLE_OPEN };
                                if ui
                                    .small_button(lock)
                                    .on_hover_text(tr(
                                        "Sperren: Objekte bleiben sichtbar, lassen sich aber nicht auswählen",
                                    ))
                                    .clicked()
                                {
                                    app.checkpoint();
                                    app.doc.layers[l].locked = !layer.locked;
                                    if !layer.locked {
                                        let doc = &app.doc;
                                        app.selection.retain(|id| doc.get(*id).is_some_and(|o| o.layer != l));
                                    }
                                    app.mark_changed();
                                }
                                let vis = if layer.visible { icon::EYE } else { icon::EYE_SLASH };
                                if ui.small_button(vis).on_hover_text(tr("Anzeigen/Ausblenden")).clicked() {
                                    app.checkpoint();
                                    app.doc.layers[l].visible = !layer.visible;
                                    app.mark_changed();
                                }
                                let out_color = if layer.settings.output { pal.success } else { pal.text_weak };
                                if ui
                                    .small_button(RichText::new(icon::POWER).color(out_color))
                                    .on_hover_text(if layer.settings.output {
                                        tr("Wird gelasert (klicken zum Abschalten)")
                                    } else {
                                        tr("Wird nicht gelasert")
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
                                if layer.settings.air_assist { tr("· Luft") } else { "" }
                            ))
                            .size(12.0)
                            .color(pal.text_weak),
                        );
                    })
            })
            .response;
        if resp.clicked() {
            if app.current_layer != l {
                app.sub_tab = 0;
            }
            app.current_layer = l;
        }
        ui.add_space(3.0);
    }

    ui.add_space(6.0);
    let l = app.current_layer;
    let has_images = app.doc.objects_on_layer(l).any(|o| o.is_image());
    let z_axis = app.settings.profile.z_axis;
    let mut layer = app.doc.layers[l].clone();
    card(ui, |ui| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(4), theme::layer_color(layer.color));
            ui.add(egui::TextEdit::singleline(&mut layer.name).desired_width(120.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button(icon::STACK_PLUS)
                    .on_hover_text(tr(
                        "Unterebene hinzufügen: weitere Bearbeitung derselben Objekte, z. B. erst füllen, dann die Kontur schneiden",
                    ))
                    .clicked()
                {
                    layer.sub.push(CutSettings::default_for(LayerMode::Line));
                    app.sub_tab = layer.sub.len();
                }
                if ui
                    .small_button(trs(&format!("{} Material", icon::BOOKS)))
                    .on_hover_text(tr("Materialbibliothek öffnen"))
                    .clicked()
                {
                    app.materials.open = true;
                }
            });
        });
        // Reiter für Haupt- und Unterebenen
        app.sub_tab = app.sub_tab.min(layer.sub.len());
        if !layer.sub.is_empty() {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (i, cs) in layer.operations().enumerate() {
                    let label = format!("{} · {}", i + 1, tr(cs.mode.label()));
                    if ui.selectable_label(app.sub_tab == i, label).clicked() {
                        app.sub_tab = i;
                    }
                }
                if app.sub_tab > 0
                    && ui.small_button(icon::STACK_MINUS).on_hover_text(tr("Diese Unterebene entfernen")).clicked()
                {
                    layer.sub.remove(app.sub_tab - 1);
                    app.sub_tab -= 1;
                }
            });
        }
        ui.add_space(4.0);
        let tab = app.sub_tab;
        let images_here = has_images && layer.image_operation() == tab;
        if let Some(cs) = layer.operation_mut(tab) {
            cut_settings_ui(ui, cs, images_here, z_axis);
        }
    });
    if layer != app.doc.layers[l] {
        app.checkpoint_coalesced("layer");
        app.doc.layers[l] = layer;
        app.mark_changed();
    }

    image_adjust_ui(app, ui);
}

pub(crate) fn cut_settings_ui(ui: &mut Ui, s: &mut CutSettings, has_images: bool, z_axis: bool) {
    egui::Grid::new("cut_settings").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.label(tr("Modus"));
        let before = s.mode;
        egui::ComboBox::from_id_salt("layer_mode").selected_text(tr(s.mode.label())).show_ui(ui, |ui| {
            for m in LayerMode::ALL {
                ui.selectable_value(&mut s.mode, m, tr(m.label()));
            }
        });
        if s.mode != before {
            // Sinnvolle Startwerte für den neuen Modus übernehmen, Ausgabe/Luft beibehalten.
            let keep = (s.output, s.air_assist);
            *s = CutSettings::default_for(s.mode);
            (s.output, s.air_assist) = keep;
        }
        ui.end_row();

        ui.label(tr("Geschwindigkeit"));
        ui.add(DragValue::new(&mut s.speed).speed(10.0).range(1.0..=60000.0).suffix(" mm/min"));
        ui.end_row();

        ui.label(tr("Leistung max."));
        ui.add(egui::Slider::new(&mut s.power_max, 0.0..=100.0).suffix(" %").fixed_decimals(1));
        ui.end_row();

        let raster = s.mode == LayerMode::Image || has_images;
        if raster && s.dither == DitherMode::Grayscale {
            ui.label(tr("Leistung min."));
            ui.add(egui::Slider::new(&mut s.power_min, 0.0..=100.0).suffix(" %").fixed_decimals(1));
            ui.end_row();
            if s.power_min > s.power_max {
                s.power_min = s.power_max;
            }
        }

        ui.label(tr("Durchgänge"));
        ui.add(DragValue::new(&mut s.passes).range(1..=100));
        ui.end_row();

        if z_axis {
            ui.label(tr("Z-Versatz"));
            ui.add(DragValue::new(&mut s.z_offset).speed(0.05).range(-50.0..=50.0).max_decimals(2).suffix(" mm"))
                .on_hover_text(tr("Fokus gegenüber der eingestellten Fokushöhe: positiv = Kopf höher (weicherer Strahl)"));
            ui.end_row();
            if s.passes > 1 {
                ui.label(tr("Z je Durchgang"));
                ui.add(DragValue::new(&mut s.z_step).speed(0.05).range(-20.0..=20.0).max_decimals(2).suffix(" mm"))
                    .on_hover_text(tr("Ab dem zweiten Durchgang wird der Fokus um diesen Wert tiefer gelegt"));
                ui.end_row();
            }
        }

        if s.mode != LayerMode::Line || has_images {
            ui.label(if s.mode == LayerMode::OffsetFill { tr("Linienabstand") } else { tr("Zeilenabstand") });
            ui.horizontal(|ui| {
                ui.add(DragValue::new(&mut s.interval).speed(0.005).range(0.02..=2.0).max_decimals(3).suffix(" mm"));
                ui.label(
                    RichText::new(format!("≙ {:.0} dpi", 25.4 / s.interval.max(0.001)))
                        .small()
                        .color(theme::of(ui).text_weak),
                );
            });
            ui.end_row();
        }
        if matches!(s.mode, LayerMode::Fill | LayerMode::Image) || has_images {
            ui.label(tr("Overscan"));
            ui.add(DragValue::new(&mut s.overscan).speed(0.1).range(0.0..=20.0).suffix(" mm"));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut s.bidirectional, tr("Bidirektional"));
            ui.end_row();
        }
        if s.mode == LayerMode::Line {
            ui.label(tr("Schnittbreite"));
            ui.add(DragValue::new(&mut s.kerf).speed(0.005).range(0.0..=2.0).max_decimals(3).suffix(" mm"))
                .on_hover_text(
                    tr("Kerf-Ausgleich: Außenkonturen wandern um die halbe Schnittbreite nach außen, Löcher nach innen. 0 = aus."),
                );
            ui.end_row();
            ui.label(tr("Laschen"));
            ui.horizontal(|ui| {
                ui.add(DragValue::new(&mut s.tab_count).range(0..=50).custom_formatter(|v, _| {
                    if v == 0.0 { tr("aus").into() } else { trs(&format!("{v:.0} je Kontur")) }
                }));
                if s.tab_count > 0 {
                    ui.add(DragValue::new(&mut s.tab_width).speed(0.05).range(0.1..=10.0).suffix(" mm"));
                }
            })
            .response
            .on_hover_text(tr("Kleine Stege, damit ausgeschnittene Teile im Material bleiben"));
            ui.end_row();
            if s.tab_count > 0 {
                ui.label("");
                ui.checkbox(&mut s.tabs_outer_only, tr("Nur Außenkonturen"));
                ui.end_row();
            }
            ui.label(tr("Überschnitt"));
            ui.add(DragValue::new(&mut s.overcut).speed(0.05).range(0.0..=20.0).max_decimals(2).custom_formatter(
                |v, _| if v == 0.0 { tr("aus").into() } else { format!("{v:.2} mm") },
            ))
            .on_hover_text(tr(
                "Geschlossene Konturen laufen um diese Strecke über den Startpunkt hinaus, damit sich Anfang und Ende sicher überlappen und das Teil ganz herausfällt.",
            ));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut s.perforate, tr("Perforieren"))
                .on_hover_text(tr("Abwechselnd schneiden und aussetzen, z. B. für Falz- und Abreißlinien"));
            ui.end_row();
            if s.perforate {
                ui.label(tr("Schnitt / Lücke"));
                ui.horizontal(|ui| {
                    ui.add(DragValue::new(&mut s.perf_cut).speed(0.05).range(0.05..=100.0).suffix(" mm"));
                    ui.label("/");
                    ui.add(DragValue::new(&mut s.perf_gap).speed(0.05).range(0.05..=100.0).suffix(" mm"));
                });
                ui.end_row();
            }
        }
        if s.mode == LayerMode::Fill {
            ui.label(tr("Winkel"));
            ui.add(DragValue::new(&mut s.fill_angle).speed(1.0).range(-180.0..=180.0).suffix("°"));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut s.crosshatch, tr("Kreuzschraffur"));
            ui.end_row();
        }
        if raster {
            ui.label(tr("Bildmodus"));
            egui::ComboBox::from_id_salt("dither").selected_text(tr(s.dither.label())).show_ui(ui, |ui| {
                for d in DitherMode::ALL {
                    ui.selectable_value(&mut s.dither, d, tr(d.label()));
                }
            });
            ui.end_row();
            if s.dither.is_screen() {
                ui.label(tr("Rasterweite"));
                ui.horizontal(|ui| {
                    ui.add(DragValue::new(&mut s.screen_cell).speed(0.01).range(0.1..=20.0).suffix(" mm"))
                        .on_hover_text(tr("Abstand der Punkte bzw. Linien. Ein Vielfaches des Zeilenabstands wirkt am saubersten."));
                    ui.label(
                        RichText::new(format!("≙ {:.0} lpi", 25.4 / s.screen_cell.max(0.001)))
                            .small()
                            .color(theme::of(ui).text_weak),
                    );
                });
                ui.end_row();
                ui.label(tr("Rasterwinkel"));
                ui.add(DragValue::new(&mut s.screen_angle).speed(1.0).range(-90.0..=90.0).suffix("°"));
                ui.end_row();
            }
            ui.label("");
            ui.checkbox(&mut s.negative, tr("Negativ (invertieren)"));
            ui.end_row();
        }
        ui.label("");
        ui.checkbox(&mut s.air_assist, tr("Luftunterstützung"));
        ui.end_row();
        ui.label("");
        ui.checkbox(&mut s.output, tr("Ausgeben"));
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
        section_title(ui, icon::IMAGE, tr("Bild anpassen"));
        ui.label(
            RichText::new(format!("{} × {} px", img.image.width, img.image.height))
                .small()
                .color(theme::of(ui).text_weak),
        );
        egui::Grid::new("img_adjust").num_columns(2).show(ui, |ui| {
            ui.label(tr("Helligkeit"));
            ui.add(egui::Slider::new(&mut adj.brightness, -100.0..=100.0).fixed_decimals(0));
            ui.end_row();
            ui.label(tr("Kontrast"));
            ui.add(egui::Slider::new(&mut adj.contrast, -100.0..=100.0).fixed_decimals(0));
            ui.end_row();
            ui.label(tr("Gamma"));
            ui.add(egui::Slider::new(&mut adj.gamma, 0.2..=5.0).logarithmic(true).fixed_decimals(2));
            ui.end_row();
        });
        if ui.button(tr("Zurücksetzen")).clicked() {
            adj = vl_core::ImageAdjust::default();
        }
    });
    if adj != img.adjust {
        app.checkpoint_coalesced("image_adjust");
        if let Some(o) = app.doc.get_mut(id)
            && let Shape::Image(i) = &mut o.shape
        {
            i.adjust = adj;
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
        section_title(ui, icon::PLUGS, tr("Verbindung"));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let current = app.ports.iter().find(|p| p.name == app.selected_port);
            let label = current.map(|p| p.name.clone()).unwrap_or_else(|| tr("Anschluss wählen").into());
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
                if ui.button(icon::ARROWS_CLOCKWISE).on_hover_text(tr("Anschlüsse neu suchen")).clicked() {
                    app.ports = vl_device::available_ports();
                }
            });
            if connected {
                if ui.button(trs(&format!("{} Trennen", icon::LINK_BREAK))).clicked() {
                    app.disconnect();
                }
            } else if ui
                .add(
                    Button::new(RichText::new(trs(&format!("{} Verbinden", icon::PLUG))).color(Color32::WHITE))
                        .fill(pal.accent),
                )
                .clicked()
            {
                app.connect();
            }
        });
        if app.selected_port == vl_device::NETWORK_PORT {
            ui.horizontal(|ui| {
                ui.label(tr("Adresse"));
                let edit = egui::TextEdit::singleline(&mut app.settings.network_address)
                    .hint_text("192.168.4.1:23")
                    .desired_width(170.0);
                let resp = ui.add_enabled(!connected, edit).on_hover_text(tr(
                    "IP-Adresse oder Name der Steuerung, optional mit Port (Standard 23 = Telnet). \
                     Für FluidNC, grblHAL und ESP3D.",
                ));
                if resp.lost_focus() {
                    app.settings.save();
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !connected {
                    app.connect();
                }
            });
        }
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
                ui.label(RichText::new(format!("● {}", tr(state.label()))).color(color).strong());
                if let Some(p) = app.machine.head_position() {
                    ui.label(RichText::new(format!("X {:.2}  Y {:.2}", p.x, p.y)).monospace());
                }
            });
            // Laser-Modus ($32) und Einstellungen gibt es nur bei GRBL.
            let fw = app.firmware();
            match app.machine.laser_mode() {
                _ if !fw.is_grbl() => ui.label(RichText::new(tr(fw.label())).small().color(pal.text_weak)),
                Some(true) => ui.label(
                    RichText::new(trs(&format!("{} Laser-Modus aktiv ($32=1)", icon::CHECK)))
                        .small()
                        .color(pal.success),
                ),
                Some(false) => ui.label(
                    RichText::new(trs(&format!(
                        "{} Laser-Modus aus ($32=0) – Leerfahrten könnten brennen!",
                        icon::WARNING
                    )))
                    .small()
                    .color(pal.danger),
                ),
                None => ui.label(RichText::new(tr("Einstellungen werden gelesen…")).small().color(pal.text_weak)),
            };
            if state == MachineState::Alarm {
                ui.label(RichText::new(tr("Alarm: Referenzfahrt oder Entsperren nötig.")).small().color(pal.danger));
            }
        }
    });

    ui.add_space(8.0);
    card(ui, |ui| {
        section_title(ui, icon::TARGET, tr("Maschine"));
        ui.add_space(4.0);
        let w = (ui.available_width() - 8.0) / 2.0;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    connected && !running,
                    Button::new(trs(&format!("{} Referenzfahrt", icon::HOUSE))).min_size(vec2(w, 28.0)),
                )
                .clicked()
            {
                app.home();
            }
            if ui
                .add_enabled(
                    connected && !running,
                    Button::new(trs(&format!("{} Entsperren", icon::LOCK_OPEN))).min_size(vec2(w, 28.0)),
                )
                .clicked()
            {
                app.unlock();
            }
        });
        ui.horizontal(|ui| {
            let spacing = ui.spacing().item_spacing.x;
            ui.spacing_mut().item_spacing.x = 4.0;
            if ui
                .add_enabled(
                    connected && !running,
                    Button::new(trs(&format!("{} Rahmen", icon::FRAME_CORNERS))).min_size(vec2(w - 34.0, 28.0)),
                )
                .on_hover_text(tr("Das umgebende Rechteck des Jobs abfahren (Laser aus)"))
                .clicked()
            {
                app.frame_job(false);
            }
            if ui
                .add_enabled(connected && !running, Button::new(icon::POLYGON).min_size(vec2(30.0, 28.0)))
                .on_hover_text(tr(
                    "Umriss-Rahmen: die konvexe Hülle des Jobs abfahren – folgt runden und schrägen Formen enger als das Rechteck",
                ))
                .clicked()
            {
                app.frame_job(true);
            }
            ui.spacing_mut().item_spacing.x = spacing;
            if ui
                .add_enabled(
                    connected && !running,
                    Button::new(trs(&format!("{} Ursprung hier", icon::CROSSHAIR))).min_size(vec2(w, 28.0)),
                )
                .on_hover_text(tr("Aktuelle Kopfposition als Benutzer-Ursprung speichern"))
                .clicked()
            {
                app.set_user_origin_here();
            }
        });
    });

    ui.add_space(8.0);
    card(ui, |ui| {
        section_title(ui, icon::PLAY, tr("Job"));
        ui.add_space(4.0);
        crate::rotary_ui::job_row(app, ui);
        ui.add_space(2.0);
        let mut js = app.settings.job.clone();
        egui::Grid::new("job_settings").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
            ui.label(tr("Start von"));
            egui::ComboBox::from_id_salt("start_from").selected_text(tr(js.start_from.label())).show_ui(ui, |ui| {
                for s in StartFrom::ALL {
                    ui.selectable_value(&mut js.start_from, s, tr(s.label()));
                }
            });
            ui.end_row();
            if js.start_from != StartFrom::Absolute {
                ui.label(tr("Ankerpunkt"));
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
                ui.label(tr("Ursprung"));
                ui.horizontal(|ui| {
                    ui.add(DragValue::new(&mut js.user_origin.x).speed(0.5).prefix("X ").suffix(" mm"));
                    ui.add(DragValue::new(&mut js.user_origin.y).speed(0.5).prefix("Y ").suffix(" mm"));
                });
                ui.end_row();
            }
        });
        ui.checkbox(&mut js.selected_only, tr("Nur Auswahl ausgeben"));
        ui.checkbox(&mut js.engrave_first, tr("Gravieren vor Schneiden"))
            .on_hover_text(tr("Bild- und Füllebenen zuerst, Linienebenen (Schnitte) zuletzt"));
        ui.checkbox(&mut js.optimize_order, tr("Schnittreihenfolge optimieren"));
        ui.add_enabled(js.optimize_order, egui::Checkbox::new(&mut js.inner_first, tr("Innen vor außen schneiden")));
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
            // Ohne Schätzung (fremde G-Code-Datei) wird die Restzeit aus dem Fortschritt hochgerechnet
            let remaining = if app.machine.job_estimate > 0.0 {
                Some((app.machine.job_estimate - elapsed).max(0.0))
            } else {
                (frac > 0.02).then(|| elapsed * (1.0 - frac as f64) / frac as f64)
            };
            let text = match remaining {
                Some(rest) => trs(&format!(
                    "Läuft seit {} · Rest ca. {}",
                    vl_cam::format_duration(elapsed),
                    vl_cam::format_duration(rest)
                )),
                None => trs(&format!("Läuft seit {}", vl_cam::format_duration(elapsed))),
            };
            ui.label(RichText::new(text).small().color(pal.text_weak));
            ui.add_space(4.0);
            let paused = app.machine.status.as_ref().is_some_and(|s| s.state == MachineState::Hold);
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 8.0) / 2.0;
                let (label, cmd) = if paused {
                    (trs(&format!("{} Fortsetzen", icon::PLAY)), Command::Resume)
                } else {
                    (trs(&format!("{} Pause", icon::PAUSE)), Command::Pause)
                };
                if ui.add(Button::new(label).min_size(vec2(w, 34.0))).clicked() {
                    app.device.send(cmd);
                }
                if ui
                    .add(
                        Button::new(
                            RichText::new(trs(&format!("{} Stopp", icon::STOP))).color(Color32::WHITE).strong(),
                        )
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
                trs(&format!("{}  Job starten", icon::PLAY)),
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
                        Button::new(RichText::new(trs(&format!("{} Stopp", icon::STOP))).color(Color32::WHITE))
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
                        Button::new(trs(&format!("{} G-Code", icon::EXPORT))).min_size(vec2(w, 28.0)),
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
        section_title(ui, icon::EYE, tr("Vorschau"));
        ui.add_space(4.0);
        if let Some(err) = &app.preview.error {
            ui.label(RichText::new(trs(err)).color(pal.warning));
            return;
        }
        let Some(job) = app.preview.job.clone() else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tr("Berechne Werkzeugweg…"));
            });
            return;
        };
        egui::Grid::new("preview_stats").num_columns(2).show(ui, |ui| {
            ui.label(tr("Geschätzte Dauer"));
            ui.label(RichText::new(job.estimate.format_duration()).strong());
            ui.end_row();
            ui.label(tr("Laserweg"));
            ui.label(format!("{:.2} m", job.estimate.burn_distance / 1000.0));
            ui.end_row();
            ui.label(tr("Leerfahrten"));
            ui.label(format!("{:.2} m", job.estimate.travel_distance / 1000.0));
            ui.end_row();
            ui.label(tr("G-Code-Zeilen"));
            ui.label(format!("{}", job.gcode.len()));
            ui.end_row();
        });
        ui.add_space(4.0);
        ui.label(RichText::new(tr("Abspielen")).small().color(pal.text_weak));
        ui.add(egui::Slider::new(&mut app.preview.progress, 0.0..=1.0).show_value(false));
        if ui.checkbox(&mut app.settings.show_travel, tr("Leerfahrten anzeigen")).changed() {
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
        section_title(ui, icon::ARROWS_OUT, tr("Verfahren"));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(tr("Schritt"));
            for s in [0.1, 1.0, 5.0, 10.0, 50.0] {
                if ui.selectable_label((app.settings.jog_step - s).abs() < 1e-9, format!("{s}")).clicked() {
                    app.settings.jog_step = s;
                    app.settings.save();
                }
            }
            ui.label("mm");
        });
        ui.horizontal(|ui| {
            ui.label(tr("Geschwindigkeit"));
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
                        let tip = if dx == 0.0 && dy == 0.0 { tr("Zum Nullpunkt (X0 Y0)") } else { tr("Verfahren") };
                        if ui.add(b).on_hover_text(tip).clicked() {
                            if dx == 0.0 && dy == 0.0 {
                                app.send_jog(false, "X0 Y0");
                            } else {
                                app.jog(Point::new(dx, dy));
                            }
                        }
                    }
                });
            }
            ui.add_space(6.0);
            // Nur GRBL kann eine Verfahrbewegung gezielt abbrechen (Echtzeitbefehl 0x85).
            if app.firmware().is_grbl() && ui.button(trs(&format!("{} Bewegung abbrechen", icon::HAND_PALM))).clicked()
            {
                app.device.send(Command::Realtime(vl_device::RT_JOG_CANCEL));
            }
        });
        if !app.machine.connected {
            ui.label(RichText::new(tr("Zum Verfahren bitte zuerst verbinden.")).small().color(pal.text_weak));
        }
    });
}

// ------------------------------------------------------------------ Konsole

/// Makro-Knöpfe: Klick sendet die Zeilen, Rechtsklick bearbeitet oder entfernt.
fn macros_ui(app: &mut VectorLumeApp, ui: &mut Ui) {
    let ready = app.machine.connected && !app.machine.job_running;
    let mut send = None;
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
        for (i, m) in app.settings.macros.iter().enumerate() {
            let resp = ui
                .add_enabled(ready, Button::new(&m.name))
                .on_hover_text(m.gcode.trim())
                .on_disabled_hover_text(tr("Erst mit dem Laser verbinden"));
            if resp.clicked() {
                send = Some(i);
            }
            resp.context_menu(|ui| {
                if ui.button(trs(&format!("{} Bearbeiten…", icon::PENCIL_SIMPLE))).clicked() {
                    app.macro_edit = Some((Some(i), m.clone()));
                    ui.close();
                }
                if ui.button(trs(&format!("{} Entfernen", icon::TRASH))).clicked() {
                    remove = Some(i);
                    ui.close();
                }
            });
        }
        if ui
            .small_button(trs(&format!("{} Makro", icon::PLUS)))
            .on_hover_text(tr("Eigene Befehle als Knopf ablegen, z. B. Luft an/aus oder eine Parkposition. Rechtsklick auf einen Makro-Knopf bearbeitet ihn."))
            .clicked()
        {
            app.macro_edit = Some((None, crate::settings::Macro::default()));
        }
    });
    if let Some(i) = send {
        let lines = vl_cam::lines_from_file(&app.settings.macros[i].gcode);
        for line in lines {
            app.send_line(line);
        }
    }
    if let Some(i) = remove {
        app.settings.macros.remove(i);
        app.settings.save();
    }
    macro_editor(app, ui.ctx());
}

fn macro_editor(app: &mut VectorLumeApp, ctx: &egui::Context) {
    let Some((index, draft)) = &mut app.macro_edit else { return };
    let mut open = true;
    let mut save = false;
    let title = if index.is_some() { tr("Makro bearbeiten") } else { tr("Neues Makro") };
    egui::Window::new(format!("{} {title}", icon::TERMINAL))
        .id(egui::Id::new("macro_editor"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(320.0)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.add(egui::TextEdit::singleline(&mut draft.name).hint_text(tr("Name, z. B. Luft an")));
            ui.add_space(4.0);
            ui.add(
                egui::TextEdit::multiline(&mut draft.gcode)
                    .code_editor()
                    .desired_rows(5)
                    .desired_width(f32::INFINITY)
                    .hint_text(tr("G-Code, eine Zeile je Befehl, z. B. M8")),
            );
            ui.label(
                RichText::new(tr("Die Zeilen werden ungeprüft gesendet; Kommentare nach ; werden entfernt."))
                    .small()
                    .color(theme::of(ui).text_weak),
            );
            ui.add_space(6.0);
            let ok = !draft.name.trim().is_empty() && !vl_cam::lines_from_file(&draft.gcode).is_empty();
            if ui.add_enabled(ok, Button::new(trs(&format!("{} Speichern", icon::CHECK)))).clicked() {
                save = true;
            }
        });
    if save {
        let (index, mut m) = app.macro_edit.take().unwrap_or_default();
        m.name = m.name.trim().to_string();
        match index.filter(|&i| i < app.settings.macros.len()) {
            Some(i) => app.settings.macros[i] = m,
            None => app.settings.macros.push(m),
        }
        app.settings.save();
    } else if !open {
        app.macro_edit = None;
    }
}

fn console_ui(app: &mut VectorLumeApp, ui: &mut Ui) {
    let pal = theme::of(ui);
    ui.horizontal(|ui| {
        let quick: &[(&str, &str)] = if app.firmware().is_grbl() {
            &[
                ("$$", tr("Einstellungen")),
                ("$I", tr("Firmware")),
                ("$G", tr("G-Code-Zustand")),
                ("$#", tr("Koordinatensysteme")),
            ]
        } else {
            &[("M115", tr("Firmware")), ("M114", tr("Position")), ("M503", tr("Einstellungen (Marlin)"))]
        };
        for &(cmd, tip) in quick {
            if ui.add_enabled(app.machine.connected, Button::new(cmd)).on_hover_text(tip).clicked() {
                app.send_line(cmd);
            }
        }
        if ui.button(icon::BROOM).on_hover_text(tr("Konsole leeren")).clicked() {
            app.console.clear();
        }
    });
    macros_ui(app, ui);
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
                .hint_text(tr("Befehl, z. B. $$ oder G0 X10"))
                .desired_width(ui.available_width() - 70.0),
        );
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button(tr("Senden")).clicked() || enter) && !app.console_input.trim().is_empty() {
            let cmd = std::mem::take(&mut app.console_input);
            app.send_line(cmd.trim().to_string());
            resp.request_focus();
        }
    });
}
