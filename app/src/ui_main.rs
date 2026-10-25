//! Menü, Werkzeugleisten, Eigenschaftsfelder, Farbpalette, Statusleiste und Tastenkürzel.

use crate::app::{ToastKind, Tool, VectorLumeApp};
use crate::hotkeys::Action;
use crate::i18n::{tr, trs};
use crate::settings::ThemeChoice;
use crate::theme;
use crate::units::UnitField;
use egui::{
    Align, Button, Color32, CornerRadius, DragValue, Frame, Key, KeyboardShortcut, Layout, Margin, Modifiers, RichText,
    Sense, Stroke, Ui, vec2,
};
use egui_phosphor::regular as icon;
use vl_core::primitive::Primitive;
use vl_core::{Affine, LAYER_COUNT, Point};

pub fn shortcuts(app: &mut VectorLumeApp, ctx: &egui::Context) {
    // Während der Tastenaufnahme im Kürzel-Fenster gehört die Taste dort hin.
    if ctx.egui_wants_keyboard_input() || app.modal.is_some() || app.hotkey_editor.recording.is_some() {
        return;
    }
    let sc = |m: Modifiers, k: Key| KeyboardShortcut::new(m, k);
    let pressed = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
    // Schnittreihenfolge: nur die Tasten des Klickmodus, damit z. B. Entf nichts löscht
    if app.cut_order.is_some() {
        if pressed(sc(Modifiers::NONE, Key::Escape)) {
            app.cut_order = None;
        } else if pressed(sc(Modifiers::NONE, Key::Enter)) {
            app.apply_cut_order();
        } else if (pressed(sc(Modifiers::NONE, Key::Backspace)) || pressed(sc(Modifiers::NONE, Key::Delete)))
            && let Some(pick) = app.cut_order.as_mut()
        {
            pick.picked.pop();
        }
        return;
    }
    for action in app.keys.pressed(ctx) {
        crate::hotkeys::run(app, action);
    }
    if app.warp.is_some() && pressed(sc(Modifiers::NONE, Key::Enter)) {
        app.apply_warp();
    }
    if pressed(sc(Modifiers::NONE, Key::Escape)) {
        // Klickmodi auf der Arbeitsfläche zuerst beenden
        if app.tangent.is_some() {
            app.tangent = None;
        } else if app.print_cut.picking.is_some() {
            app.print_cut.picking = None;
        } else if app.warp.is_some() {
            app.cancel_warp();
        } else if app.preview.active {
            app.preview.active = false;
        } else if app.tool != Tool::Select {
            app.tool = Tool::Select;
        } else {
            app.selection.clear();
        }
    }
    let step = if ctx.input(|i| i.modifiers.shift) { 10.0 } else { 1.0 };
    let (l, r, u, d) = ctx.input(|i| {
        (
            i.key_pressed(Key::ArrowLeft),
            i.key_pressed(Key::ArrowRight),
            i.key_pressed(Key::ArrowUp),
            i.key_pressed(Key::ArrowDown),
        )
    });
    let dx = (r as i32 - l as i32) as f64 * step;
    let dy = (d as i32 - u as i32) as f64 * step;
    if dx != 0.0 || dy != 0.0 {
        app.nudge(dx, dy);
    }
}

fn icon_button(ui: &mut Ui, glyph: &str, tooltip: &str, enabled: bool) -> bool {
    ui.add_enabled(enabled, Button::new(RichText::new(glyph).size(17.0)).min_size(vec2(30.0, 28.0)))
        .on_hover_text(tooltip)
        .clicked()
}

fn menu_item(ui: &mut Ui, text: &str, shortcut: &str, enabled: bool) -> bool {
    let mut b = Button::new(text);
    if !shortcut.is_empty() {
        b = b.shortcut_text(shortcut);
    }
    let clicked = ui.add_enabled(enabled, b).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

pub fn top_bar(app: &mut VectorLumeApp, ui: &mut Ui) {
    let pal = theme::of(ui);
    egui::Panel::top("menu").frame(Frame::NONE.fill(pal.panel).inner_margin(Margin::symmetric(10, 4))).show(ui, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.label(
                RichText::new(format!("{} VectorLume", icon::SPARKLE)).font(theme::strong_font(15.0)).color(pal.accent),
            );
            ui.add_space(10.0);
            ui.menu_button(tr("Datei"), |ui| file_menu(app, ui));
            ui.menu_button(tr("Bearbeiten"), |ui| edit_menu(app, ui));
            ui.menu_button(tr("Anordnen"), |ui| arrange_menu(app, ui));
            ui.menu_button(tr("Ansicht"), |ui| view_menu(app, ui));
            ui.menu_button(tr("Laser"), |ui| laser_menu(app, ui));
            ui.menu_button(tr("Hilfe"), |ui| {
                if menu_item(ui, tr("Handbuch"), &app.keys.text(Action::Manual), true) {
                    app.manual.open = true;
                }
                if menu_item(ui, tr("Beispielprojekt laden"), "", true)
                    && app.confirm_discard(crate::app::AfterDiscard::Demo)
                {
                    app.load_demo();
                }
                if menu_item(ui, tr("Über VectorLume"), "", true) {
                    app.show_about = true;
                }
            });
        });
    });

    egui::Panel::top("toolbar")
        .frame(
            Frame::NONE.fill(pal.panel_alt).inner_margin(Margin::symmetric(10, 6)).stroke(Stroke::new(1.0, pal.border)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if icon_button(ui, icon::FILE_PLUS, &app.keys.hint(tr("Neu"), Action::New), true) {
                    app.new_document(false);
                }
                if icon_button(ui, icon::FOLDER_OPEN, &app.keys.hint(tr("Öffnen"), Action::Open), true) {
                    app.open_dialog(false, None);
                }
                if icon_button(ui, icon::FLOPPY_DISK, &app.keys.hint(tr("Speichern"), Action::Save), true) {
                    app.save(false);
                }
                let hint = app.keys.hint(tr("Importieren: SVG, DXF, Bilder"), Action::Import);
                if icon_button(ui, icon::DOWNLOAD_SIMPLE, &hint, true) {
                    app.import_dialog();
                }
                let hint = app.keys.hint(tr("G-Code speichern"), Action::ExportGcode);
                if icon_button(ui, icon::EXPORT, &hint, !app.doc.objects.is_empty()) {
                    app.export_gcode_dialog();
                }
                ui.separator();
                let hint = app.keys.hint(tr("Rückgängig"), Action::Undo);
                if icon_button(ui, icon::ARROW_U_UP_LEFT, &hint, app.history.can_undo()) {
                    app.undo();
                }
                let hint = app.keys.hint(tr("Wiederholen"), Action::Redo);
                if icon_button(ui, icon::ARROW_U_UP_RIGHT, &hint, app.history.can_redo()) {
                    app.redo();
                }
                ui.separator();
                if icon_button(ui, icon::CORNERS_OUT, tr("Arbeitsfläche einpassen (F)"), true) {
                    app.zoom_to_fit();
                }
                if icon_button(ui, icon::MAGNIFYING_GLASS_PLUS, tr("Vergrößern"), true) {
                    app.view.zoom_by(1.25);
                }
                if icon_button(ui, icon::MAGNIFYING_GLASS_MINUS, tr("Verkleinern"), true) {
                    app.view.zoom_by(0.8);
                }
                ui.separator();
                let label = trs(&format!("{} Vorschau", if app.preview.active { icon::EYE } else { icon::EYE_SLASH }));
                let b =
                    Button::selectable(app.preview.active, RichText::new(label).size(14.0)).min_size(vec2(0.0, 28.0));
                let hint = app.keys.hint(tr("Werkzeugweg und Laufzeit anzeigen"), Action::Preview);
                if ui.add(b).on_hover_text(hint).clicked() {
                    app.preview.active = !app.preview.active;
                }
                ui.separator();
                if app.tool == Tool::Select
                    || app.tool == Tool::Line
                    || app.tool == Tool::Ellipse
                    || app.tool == Tool::Text
                    || app.tool == Tool::Barcode
                {
                    properties(app, ui);
                } else {
                    tool_options(app, ui);
                }
            });
        });
}

fn file_menu(app: &mut VectorLumeApp, ui: &mut Ui) {
    if menu_item(ui, tr("Neu"), &app.keys.text(Action::New), true) {
        app.new_document(false);
    }
    if menu_item(ui, tr("Öffnen…"), &app.keys.text(Action::Open), true) {
        app.open_dialog(false, None);
    }
    let recent = app.settings.recent_files.clone();
    ui.add_enabled_ui(!recent.is_empty(), |ui| {
        ui.menu_button(tr("Zuletzt geöffnet"), |ui| {
            for f in recent {
                if ui.button(crate::app::file_name(&f)).on_hover_text(f.display().to_string()).clicked() {
                    ui.close();
                    app.open_dialog(false, Some(f));
                }
            }
        });
    });
    if menu_item(ui, tr("Speichern"), &app.keys.text(Action::Save), true) {
        app.save(false);
    }
    if menu_item(ui, tr("Speichern unter…"), &app.keys.text(Action::SaveAs), true) {
        app.save(true);
    }
    ui.separator();
    if menu_item(ui, tr("Importieren…"), &app.keys.text(Action::Import), true) {
        app.import_dialog();
    }
    ui.separator();
    let has = !app.doc.objects.is_empty();
    if menu_item(ui, tr("G-Code speichern…"), &app.keys.text(Action::ExportGcode), has) {
        app.export_gcode_dialog();
    }
    if menu_item(ui, tr("Als SVG exportieren…"), "", has) {
        app.export_svg_dialog();
    }
    if menu_item(ui, tr("Als DXF exportieren…"), "", has) {
        app.export_dxf_dialog();
    }
    if menu_item(ui, tr("Als PDF exportieren…"), "", has) {
        app.export_pdf_dialog();
    }
    ui.separator();
    if menu_item(ui, tr("Beenden"), "", true) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

fn edit_menu(app: &mut VectorLumeApp, ui: &mut Ui) {
    let has = !app.selection.is_empty();
    if menu_item(ui, tr("Rückgängig"), &app.keys.text(Action::Undo), app.history.can_undo()) {
        app.undo();
    }
    if menu_item(ui, tr("Wiederholen"), &app.keys.text(Action::Redo), app.history.can_redo()) {
        app.redo();
    }
    if menu_item(ui, tr("Bearbeitungsverlauf…"), &app.keys.text(Action::History), true) {
        app.history_window.open = !app.history_window.open;
    }
    if menu_item(ui, tr("Tastenkürzel…"), "", true) {
        app.hotkey_editor.open = true;
    }
    ui.separator();
    if menu_item(ui, tr("Ausschneiden"), &app.keys.text(Action::Cut), has) {
        app.cut();
    }
    if menu_item(ui, tr("Kopieren"), &app.keys.text(Action::Copy), has) {
        app.copy();
    }
    if menu_item(ui, tr("Einfügen"), &app.keys.text(Action::Paste), !app.clipboard.is_empty()) {
        app.paste();
    }
    if menu_item(ui, tr("An Ort und Stelle einfügen"), &app.keys.text(Action::PasteInPlace), !app.clipboard.is_empty())
    {
        app.paste_in_place();
    }
    if menu_item(ui, tr("Duplizieren"), &app.keys.text(Action::Duplicate), has) {
        app.duplicate();
    }
    if menu_item(ui, tr("Löschen"), &app.keys.text(Action::Delete), has) {
        app.delete_selection();
    }
    ui.separator();
    if menu_item(ui, tr("Alles auswählen"), &app.keys.text(Action::SelectAll), !app.doc.objects.is_empty()) {
        app.select_all();
    }
    if menu_item(ui, tr("Auswahl umkehren"), &app.keys.text(Action::InvertSelection), !app.doc.objects.is_empty()) {
        app.invert_selection();
    }
    let layer = app.current_layer;
    let on_layer = app.doc.objects_on_layer(layer).next().is_some();
    if menu_item(ui, &trs(&format!("Alles auf Ebene „{}“ auswählen", app.doc.layers[layer].name)), "", on_layer) {
        app.select_layer(layer);
    }
    if menu_item(ui, tr("Offene Formen auswählen"), "", !app.doc.objects.is_empty()) {
        app.select_open_shapes();
    }
}

/// Ziele von „An Arbeitsfläche schieben“, zeilenweise von oben links nach unten rechts.
const BED_POSITIONS: [(&str, &str); 9] = [
    (icon::ARROW_UP_LEFT, "Oben links"),
    (icon::ARROW_UP, "Oben Mitte"),
    (icon::ARROW_UP_RIGHT, "Oben rechts"),
    (icon::ARROW_LEFT, "Links Mitte"),
    (icon::CROSSHAIR, "Mitte"),
    (icon::ARROW_RIGHT, "Rechts Mitte"),
    (icon::ARROW_DOWN_LEFT, "Unten links"),
    (icon::ARROW_DOWN, "Unten Mitte"),
    (icon::ARROW_DOWN_RIGHT, "Unten rechts"),
];

fn arrange_menu(app: &mut VectorLumeApp, ui: &mut Ui) {
    let has = !app.selection.is_empty();
    let multi = app.selection.len() > 1;
    if menu_item(ui, tr("Gruppieren"), &app.keys.text(Action::Group), multi) {
        app.group();
    }
    if menu_item(ui, tr("Gruppierung lösen"), &app.keys.text(Action::Ungroup), has) {
        app.ungroup();
    }
    if menu_item(ui, tr("Zerlegen"), &app.keys.text(Action::BreakApart), has) {
        app.break_apart();
    }
    if menu_item(ui, tr("Automatisch gruppieren"), "", !app.doc.objects.is_empty()) {
        app.auto_group();
    }
    ui.separator();
    ui.add_enabled_ui(multi, |ui| {
        ui.menu_button(tr("Ausrichten"), |ui| {
            for (label, h, m) in [
                (tr("Links"), true, 0),
                (tr("Horizontal zentrieren"), true, 1),
                (tr("Rechts"), true, 2),
                (tr("Oben"), false, 0),
                (tr("Vertikal zentrieren"), false, 1),
                (tr("Unten"), false, 2),
            ] {
                if menu_item(ui, label, "", true) {
                    app.align(h, m);
                }
            }
        });
        ui.menu_button(tr("Verteilen"), |ui| {
            if menu_item(ui, tr("Horizontal"), "", true) {
                app.distribute(true);
            }
            if menu_item(ui, tr("Vertikal"), "", true) {
                app.distribute(false);
            }
        });
        ui.menu_button(tr("Gleiche Größe"), |ui| {
            ui.label(RichText::new(tr("Maß der zuerst gewählten Form")).small().color(theme::of(ui).text_weak));
            if menu_item(ui, tr("Gleiche Breite"), "", true) {
                app.make_same_size(true);
            }
            if menu_item(ui, tr("Gleiche Höhe"), "", true) {
                app.make_same_size(false);
            }
        });
    });
    ui.add_enabled_ui(has, |ui| {
        ui.menu_button(tr("Reihenfolge"), |ui| {
            ui.label(
                RichText::new(tr(
                    "Vorne liegende Objekte werden oben gezeichnet und ohne Optimierung zuletzt gelasert",
                ))
                .small()
                .color(theme::of(ui).text_weak),
            );
            for action in [Action::ToFront, Action::Forward, Action::Backward, Action::ToBack] {
                if menu_item(ui, action.label(), &app.keys.text(action), true) {
                    crate::hotkeys::run(app, action);
                }
            }
        });
        ui.menu_button(tr("Andocken"), |ui| {
            ui.label(
                RichText::new(tr("Schiebt die Auswahl, bis sie eine andere Form oder den Rand berührt"))
                    .small()
                    .color(theme::of(ui).text_weak),
            );
            for (label, glyph, dx, dy) in [
                (tr("Nach links"), egui_phosphor::regular::ARROW_LEFT, -1.0, 0.0),
                (tr("Nach rechts"), egui_phosphor::regular::ARROW_RIGHT, 1.0, 0.0),
                (tr("Nach oben"), egui_phosphor::regular::ARROW_UP, 0.0, -1.0),
                (tr("Nach unten"), egui_phosphor::regular::ARROW_DOWN, 0.0, 1.0),
            ] {
                if menu_item(ui, &format!("{glyph} {label}"), "", true) {
                    app.dock_selection(vl_core::Point::new(dx, dy));
                }
            }
            ui.horizontal(|ui| {
                ui.label(tr("Abstand"));
                ui.add(
                    egui::DragValue::new(&mut app.shape_dialogs.dock_gap)
                        .speed(0.05)
                        .range(0.0..=100.0)
                        .max_decimals(2)
                        .mm(),
                );
            });
        });
    });
    let any = !app.doc.objects.is_empty();
    if menu_item(ui, tr("Schnittreihenfolge per Klick…"), &app.keys.text(Action::CutOrder), any) {
        app.start_cut_order();
    }
    ui.separator();
    if menu_item(ui, tr("Horizontal spiegeln"), &app.keys.text(Action::MirrorH), has) {
        app.mirror(true);
    }
    if menu_item(ui, tr("Vertikal spiegeln"), &app.keys.text(Action::MirrorV), has) {
        app.mirror(false);
    }
    if menu_item(ui, tr("An Linie spiegeln…"), "", has) {
        app.tool = Tool::Measure;
        app.measure = None;
        app.toast(ToastKind::Info, tr("Spiegelachse ziehen, dann oben „An Linie spiegeln“ wählen."));
    }
    if menu_item(ui, tr("90° im Uhrzeigersinn drehen"), &app.keys.text(Action::RotateCw), has) {
        app.rotate_selection(90.0);
    }
    if menu_item(ui, tr("90° gegen den Uhrzeigersinn drehen"), &app.keys.text(Action::RotateCcw), has) {
        app.rotate_selection(-90.0);
    }
    if menu_item(ui, tr("Auf Arbeitsfläche zentrieren"), &app.keys.text(Action::CenterOnBed), has) {
        app.center_on_bed();
    }
    ui.add_enabled_ui(has, |ui| {
        ui.menu_button(tr("An Arbeitsfläche schieben"), |ui| {
            egui::Grid::new("bed_positions").spacing(egui::vec2(4.0, 4.0)).show(ui, |ui| {
                for (row, ay) in [0.0, 0.5, 1.0].into_iter().enumerate() {
                    for (col, ax) in [0.0, 0.5, 1.0].into_iter().enumerate() {
                        let (glyph, label) = BED_POSITIONS[row * 3 + col];
                        if ui
                            .add(Button::new(RichText::new(glyph).size(16.0)).min_size(egui::vec2(32.0, 32.0)))
                            .on_hover_text(tr(label))
                            .clicked()
                        {
                            app.move_to_bed(ax, ay);
                            ui.close();
                        }
                    }
                    ui.end_row();
                }
            });
        });
    });
    ui.separator();
    ui.add_enabled_ui(has, |ui| {
        ui.menu_button(tr("Boolesche Operationen"), |ui| {
            use vl_core::ops::BoolOp;
            // Abziehen: Die übrigen Formen werden von der zuerst ausgewählten abgezogen.
            for op in [BoolOp::Union, BoolOp::Difference, BoolOp::Intersection, BoolOp::Xor] {
                if menu_item(ui, tr(op.label()), "", multi) {
                    app.boolean_op(op);
                }
            }
            if ui
                .add_enabled(multi, Button::new(tr("Zerschneiden")))
                .on_hover_text(tr(
                    "Die oberste Form schneidet die übrigen wie ein Ausstecher in Einzelteile (innen und außen); \
                     sie selbst entfällt.",
                ))
                .clicked()
            {
                app.cut_shapes();
                ui.close();
            }
            ui.separator();
            if menu_item(ui, tr("Verschweißen"), &app.keys.text(Action::Weld), has) {
                app.weld_selection();
            }
        });
    });
    ui.menu_button(tr("Bereinigen"), |ui| {
        ui.label(
            RichText::new(if has { tr("Wirkt auf die Auswahl.") } else { tr("Wirkt auf alle sichtbaren Pfade.") })
                .small()
                .color(theme::of(ui).text_weak),
        );
        ui.horizontal(|ui| {
            ui.label(tr("Toleranz"));
            ui.add(
                egui::DragValue::new(&mut app.shape_dialogs.join_tolerance)
                    .speed(0.01)
                    .range(0.001..=5.0)
                    .max_decimals(3)
                    .mm(),
            );
        });
        if menu_item(ui, tr("Offene Pfade verbinden und schließen"), "", true) {
            app.join_open_paths();
        }
        if menu_item(ui, tr("Doppelte Linien löschen"), "", true) {
            app.delete_duplicate_lines();
        }
    });
    if menu_item(ui, tr("Versatz…"), "", has) {
        app.shape_dialogs.offset_open = true;
    }
    if menu_item(ui, tr("Formen vereinfachen…"), "", has) {
        app.shape_dialogs.simplify_open = true;
    }
    if menu_item(ui, tr("Verschachteln…"), "", has) {
        app.shape_dialogs.nest_open = true;
    }
    if menu_item(ui, tr("Schlitze anpassen…"), "", has) {
        app.shape_dialogs.slots_open = true;
    }
    if menu_item(ui, tr("Ecken abrunden…"), "", has) {
        app.shape_dialogs.corners_open = true;
    }
    if menu_item(ui, tr("Schablonenstege…"), "", has) {
        app.shape_dialogs.stencil_open = true;
    }
    if menu_item(ui, tr("Verzerren…"), "", has) {
        app.start_warp(vl_core::warp::WarpKind::Perspective);
    }
    if menu_item(ui, tr("Reihen-Kopien…"), "", has) {
        app.shape_dialogs.array_open = true;
    }
    ui.menu_button(tr("Generatoren"), |ui| {
        if menu_item(ui, tr("Box-Generator…"), "", true) {
            app.box_gen.open = true;
        }
        if menu_item(ui, tr("Biegescharnier…"), "", has) {
            app.hinge.open = true;
        }
        if menu_item(ui, tr("Puzzle…"), "", true) {
            app.open_puzzle();
        }
        if menu_item(ui, tr("Zahnräder…"), "", true) {
            app.gear.open = true;
        }
        if menu_item(ui, tr("Tangentialkreise…"), "", app.selection.len() == 3) {
            app.start_tangent_circles();
        }
        ui.separator();
        if menu_item(ui, tr("Schnittbreite ermitteln…"), "", true) {
            app.open_kerf_test();
        }
    });
    if menu_item(ui, tr("Gummiband-Umriss"), "", has) {
        app.rubber_band_outline();
    }
    ui.separator();
    let image = app.selection.iter().any(|id| app.doc.get(*id).is_some_and(|o| o.is_image()));
    if menu_item(ui, tr("Bild nachzeichnen…"), "", image) {
        app.open_trace();
    }
    if menu_item(ui, tr("Bild mit Form zuschneiden"), "", app.can_clip_image()) {
        app.clip_image_to_shapes();
    }
    if menu_item(ui, tr("Zuschnitt aufheben"), "", app.selected_clipped_image().is_some()) {
        app.remove_image_clip();
    }
    if menu_item(ui, tr("In Bitmap umwandeln…"), "", has) {
        app.shape_dialogs.bitmap_open = true;
    }
    let path = app.selection.iter().any(|id| app.doc.get(*id).is_some_and(|o| !o.is_image() && o.text.is_none()));
    if menu_item(ui, tr("Text auf Pfad…"), "", path) {
        app.open_text_on_path();
    }
    if menu_item(ui, tr("Text in Kurven umwandeln"), "", app.selected_text().is_some()) {
        crate::text_tool::text_to_curves(app);
    }
}

fn view_menu(app: &mut VectorLumeApp, ui: &mut Ui) {
    if menu_item(ui, tr("Arbeitsfläche einpassen"), &app.keys.text(Action::ZoomFit), true) {
        app.zoom_to_fit();
    }
    if menu_item(ui, tr("Auf Auswahl zoomen"), "", !app.selection.is_empty()) {
        let b = app.selection_bounds();
        app.view.request_fit(Some(b));
    }
    if menu_item(ui, tr("Projektnotizen"), "", true) {
        app.show_notes = true;
    }
    ui.separator();
    let label = app.keys.hint(tr("Vorschau"), Action::Preview);
    ui.checkbox(&mut app.preview.active, label);
    let side = app.keys.hint(tr("Seitenleiste (Ebenen, Laser, Konsole)"), Action::SidePanel);
    if ui.checkbox(&mut app.settings.show_side_panel, side).changed() {
        app.settings.save();
    }
    if ui.checkbox(&mut app.settings.show_travel, tr("Leerfahrten in der Vorschau")).changed() {
        app.settings.save();
    }
    let grid = crate::units::snap_grid_label(app.settings.grid_snap_mm);
    if ui.checkbox(&mut app.settings.snap_to_grid, trs(&format!("Am Raster fangen ({grid})"))).changed() {
        app.settings.save();
    }
    if ui
        .checkbox(&mut app.settings.snap_to_objects, tr("Objektfang"))
        .on_hover_text(tr("Knoten, Mittelpunkte und Kanten anderer Objekte fangen (Alt gedrückt halten = aus)"))
        .changed()
    {
        app.settings.save();
    }
    ui.separator();
    ui.label(RichText::new(tr("Design")).small());
    for (t, label) in
        [(ThemeChoice::Dark, tr("Dunkel")), (ThemeChoice::Light, tr("Hell")), (ThemeChoice::System, tr("Wie System"))]
    {
        if ui.radio(app.settings.theme == t, label).clicked() {
            app.set_theme(t);
            ui.close();
        }
    }
    ui.separator();
    ui.label(RichText::new(tr("Sprache")).small());
    for lang in crate::i18n::Language::ALL {
        if ui.radio(app.settings.language == lang, lang.label()).clicked() {
            app.set_language(lang);
            ui.close();
        }
    }
    ui.separator();
    ui.label(RichText::new(tr("Einheit")).small());
    for u in crate::units::Units::ALL {
        if ui.radio(app.settings.units == u, u.label()).clicked() {
            app.set_units(u);
            ui.close();
        }
    }
}

fn laser_menu(app: &mut VectorLumeApp, ui: &mut Ui) {
    let connected = app.machine.connected;
    if connected {
        if menu_item(ui, tr("Trennen"), "", true) {
            app.disconnect();
        }
    } else if menu_item(ui, tr("Verbinden"), "", true) {
        app.connect();
    }
    ui.separator();
    let fw = app.firmware();
    if menu_item(ui, &trs(&format!("Referenzfahrt ({})", fw.home_command())), "", connected) {
        app.home();
    }
    if menu_item(ui, &trs(&format!("Entsperren ({})", fw.unlock_command())), "", connected) {
        app.unlock();
    }
    if menu_item(ui, tr("Rahmen fahren"), "", connected) {
        app.frame_job(false);
    }
    if menu_item(ui, tr("Umriss-Rahmen fahren"), "", connected) {
        app.frame_job(true);
    }
    if menu_item(ui, tr("Laser zur Auswahl fahren"), "", app.can_jog_to_doc() && !app.doc.objects.is_empty()) {
        app.move_laser_to_selection();
    }
    if menu_item(ui, tr("Mittelpunkt finden…"), "", true) {
        app.open_center_finder();
    }
    if menu_item(ui, tr("Job starten"), "", connected && !app.machine.job_running) {
        app.start_job();
    }
    if menu_item(ui, tr("Stopp"), "", connected) {
        app.device.send(vl_device::Command::Stop);
    }
    if menu_item(ui, tr("G-Code-Datei senden…"), "", connected && !app.machine.job_running) {
        app.send_gcode_file_dialog();
    }
    ui.separator();
    if menu_item(ui, tr("Materialtest…"), "", true) {
        app.open_material_test();
    }
    if menu_item(ui, tr("Materialbibliothek…"), "", true) {
        app.materials.open = true;
    }
    ui.separator();
    if menu_item(ui, tr("Drehachse…"), "", true) {
        app.show_rotary = true;
    }
    if menu_item(ui, tr("Kamera…"), "", true) {
        app.camera.open = true;
    }
    if menu_item(ui, tr("Drucken & Schneiden…"), "", true) {
        app.open_print_cut();
    }
    if menu_item(ui, tr("Maschineneinstellungen…"), "", true) {
        app.show_device_settings = true;
    }
    if menu_item(ui, tr("GRBL-Parameter…"), "", app.firmware().is_grbl()) {
        app.open_grbl_params();
    }
}

/// X/Y/B/H der Auswahl in Maschinenkoordinaten bearbeiten.
fn properties(app: &mut VectorLumeApp, ui: &mut Ui) {
    let b = app.selection_bounds();
    let has = !b.is_empty();
    let profile = app.settings.profile.clone();
    let mr = if has { profile.doc_rect_to_machine(&b) } else { vl_core::Rect::from_xywh(0.0, 0.0, 0.0, 0.0) };
    let (mut x, mut y, mut w, mut h) = (mr.min.x, mr.min.y, b.width(), b.height());
    ui.add_enabled_ui(has, |ui| {
        let field = |ui: &mut Ui, label: &str, v: &mut f64| {
            ui.label(RichText::new(label).color(theme::of(ui).text_weak));
            ui.add(DragValue::new(v).speed(0.2).max_decimals(2).mm().range(-10000.0..=10000.0))
        };
        let rx = field(ui, "X", &mut x);
        let ry = field(ui, "Y", &mut y);
        ui.add_space(6.0);
        let rw = field(ui, tr("B"), &mut w);
        let lock_icon = if app.lock_ratio { icon::LOCK } else { icon::LOCK_OPEN };
        if ui
            .add(Button::selectable(app.lock_ratio, lock_icon))
            .on_hover_text(tr("Seitenverhältnis beibehalten"))
            .clicked()
        {
            app.lock_ratio = !app.lock_ratio;
        }
        let rh = field(ui, "H", &mut h);
        ui.add_space(6.0);
        ui.label(RichText::new(icon::ARROW_CLOCKWISE).color(theme::of(ui).text_weak));
        ui.add(DragValue::new(&mut app.rotate_by).speed(1.0).suffix("°").range(-360.0..=360.0));
        if ui.button(tr("Drehen")).on_hover_text(tr("Auswahl um den eingegebenen Winkel drehen")).clicked() {
            let a = app.rotate_by;
            app.rotate_selection(a);
        }
        ui.add_space(6.0);
        power_scale_field(app, ui);
        primitive_fields(app, ui);

        if !has {
            return;
        }
        let ids = app.selection.clone();
        if rx.changed() || ry.changed() {
            let dm = Point::new(x - mr.min.x, y - mr.min.y);
            let dd = profile.screen_to_machine_delta(dm);
            app.checkpoint_coalesced("position");
            app.doc.transform_objects(&ids, &Affine::translate(dd.x, dd.y));
            app.mark_changed();
        }
        if (rw.changed() && b.width() > 1e-6) || (rh.changed() && b.height() > 1e-6) {
            let anchor = profile.machine_to_doc(mr.min);
            let (mut sx, mut sy) = (
                if b.width() > 1e-6 { w.max(0.01) / b.width() } else { 1.0 },
                if b.height() > 1e-6 { h.max(0.01) / b.height() } else { 1.0 },
            );
            if app.lock_ratio {
                if rw.changed() {
                    sy = sx;
                } else {
                    sx = sy;
                }
            }
            app.checkpoint_coalesced("size");
            app.doc.transform_objects(&ids, &Affine::scale_about(sx, sy, anchor));
            app.mark_changed();
        }
    });
}

/// Parameter einer einzeln ausgewählten Grundform: Eckenradius, Ecken des Vielecks, Zacken und Innenradius.
fn primitive_fields(app: &mut VectorLumeApp, ui: &mut Ui) {
    let [id] = app.selection[..] else { return };
    let Some(obj) = app.doc.get(id) else { return };
    let Some(p) = obj.live_primitive() else { return };
    let weak = theme::of(ui).text_weak;
    ui.add_space(6.0);
    let mut changed = None;
    match p {
        Primitive::Rect { .. } => {
            let mut r = obj.corner_radius().unwrap_or(0.0);
            ui.label(RichText::new(icon::FRAME_CORNERS).color(weak)).on_hover_text(tr("Eckenradius"));
            let resp = ui
                .add(DragValue::new(&mut r).speed(0.1).range(0.0..=1000.0).max_decimals(2).mm())
                .on_hover_text(tr("Eckenradius des Rechtecks"));
            if resp.changed() {
                app.checkpoint_coalesced("primitive");
                if let Some(o) = app.doc.get_mut(id) {
                    o.set_corner_radius(r);
                }
                changed = Some(id);
            }
        }
        Primitive::Polygon { center, radius, mut sides } => {
            ui.label(RichText::new(tr("Ecken")).color(weak));
            if ui.add(DragValue::new(&mut sides).range(3..=64)).changed() {
                changed = Some(id);
                app.checkpoint_coalesced("primitive");
                if let Some(o) = app.doc.get_mut(id) {
                    o.set_primitive(Primitive::Polygon { center, radius, sides });
                }
            }
        }
        Primitive::Star { center, radius, mut inner, mut points } => {
            ui.label(RichText::new(tr("Zacken")).color(weak));
            let a = ui.add(DragValue::new(&mut points).range(3..=64));
            ui.label(RichText::new(tr("Innenradius")).color(weak));
            let b = ui.add(DragValue::new(&mut inner).speed(0.01).range(0.05..=0.95).fixed_decimals(2));
            if a.changed() || b.changed() {
                changed = Some(id);
                app.checkpoint_coalesced("primitive");
                if let Some(o) = app.doc.get_mut(id) {
                    o.set_primitive(Primitive::Star { center, radius, inner, points });
                }
            }
        }
    }
    if let Some(id) = changed {
        app.geom.invalidate(id);
        app.mark_changed();
    }
}

/// Leistung der ausgewählten Objekte in Prozent der Ebenenleistung (z. B. für Materialtests).
fn power_scale_field(app: &mut VectorLumeApp, ui: &mut Ui) {
    let (first, mixed) = {
        let mut scales = app.doc.objects_in(&app.selection).map(|o| o.power_scale);
        let Some(first) = scales.next() else { return };
        (first, scales.any(|s| s != first))
    };
    let pal = theme::of(ui);
    let color = if first != 100.0 || mixed { pal.accent } else { pal.text_weak };
    ui.label(RichText::new(icon::LIGHTNING).color(color)).on_hover_text(tr("Leistung der Auswahl"));
    let mut v = first;
    let resp = ui
        .add(DragValue::new(&mut v).speed(0.5).range(1.0..=100.0).max_decimals(1).suffix(if mixed {
            " % …"
        } else {
            " %"
        }))
        .on_hover_text(tr("Leistung dieser Objekte in Prozent der Ebenenleistung (100 % = wie die Ebene). \
             Praktisch für Materialtests mit mehreren Leistungen auf einer Ebene."));
    if resp.changed() {
        app.checkpoint_coalesced("power_scale");
        for o in app.doc.objects_in_mut(&app.selection) {
            o.power_scale = v;
        }
        app.mark_changed();
    }
}

fn tool_options(app: &mut VectorLumeApp, ui: &mut Ui) {
    if app.tool == Tool::Node {
        crate::node_tool::options(app, ui);
        return;
    }
    if app.tool == Tool::Tabs {
        crate::tab_tool::options(app, ui);
        return;
    }
    if app.tool == Tool::Trim {
        ui.label(
            RichText::new(tr(
                "Auf ein Linienstück klicken: Es wird bis zu den nächsten Schnittpunkten entfernt (rot = Vorschau)",
            ))
            .color(theme::of(ui).text_weak),
        );
        return;
    }
    if app.tool == Tool::Measure {
        match app.measure {
            Some((a, b)) => {
                ui.label(crate::canvas::measure_text(a, b));
                if !app.selection.is_empty() && a.distance(b) > 1e-6 {
                    ui.separator();
                    if ui
                        .button(format!("{} {}", icon::FLIP_HORIZONTAL, tr("An Linie spiegeln")))
                        .on_hover_text(tr("Spiegelt die Auswahl an der gemessenen Linie"))
                        .clicked()
                    {
                        app.mirror_across(a, b, false);
                    }
                    if ui
                        .button(format!("{} {}", icon::COPY, tr("Gespiegelte Kopie")))
                        .on_hover_text(tr("Legt eine an der Linie gespiegelte Kopie an, das Original bleibt"))
                        .clicked()
                    {
                        app.mirror_across(a, b, true);
                    }
                }
            }
            None => {
                let hint = if app.selection.is_empty() {
                    tr("Ziehen zum Messen · fängt an Knoten und Mitten")
                } else {
                    tr("Ziehen zum Messen · danach lässt sich die Auswahl an der Linie spiegeln")
                };
                ui.label(RichText::new(hint).color(theme::of(ui).text_weak));
            }
        }
        return;
    }
    let o = &mut app.tool_options;
    match app.tool {
        Tool::Rect => {
            ui.label(tr("Eckenradius"));
            ui.add(DragValue::new(&mut o.corner_radius).speed(0.1).range(0.0..=1000.0).mm());
        }
        Tool::Polygon => {
            ui.label(tr("Ecken"));
            ui.add(DragValue::new(&mut o.polygon_sides).range(3..=64));
        }
        Tool::Star => {
            ui.label(tr("Zacken"));
            ui.add(DragValue::new(&mut o.star_points).range(3..=64));
            ui.label(tr("Innenradius"));
            ui.add(DragValue::new(&mut o.star_inner).speed(0.01).range(0.05..=0.95).fixed_decimals(2));
        }
        _ => {}
    }
    ui.label(RichText::new(tr("Ziehen zum Zeichnen · Umschalt = gleichmäßig")).small().color(theme::of(ui).text_weak));
}

pub fn tool_bar(app: &mut VectorLumeApp, ui: &mut Ui) {
    let pal = theme::of(ui);
    egui::Panel::left("tools")
        .resizable(false)
        .exact_size(50.0)
        .frame(Frame::NONE.fill(pal.panel).inner_margin(Margin::symmetric(6, 8)).stroke(Stroke::new(1.0, pal.border)))
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                for t in Tool::ALL {
                    let selected = app.tool == t;
                    let text = RichText::new(t.icon()).size(19.0).color(if selected { pal.accent } else { pal.text });
                    let b = Button::selectable(selected, text).min_size(vec2(36.0, 34.0));
                    if ui.add(b).on_hover_text(app.keys.hint(t.label(), Action::Tool(t))).clicked() {
                        app.tool = t;
                    }
                }
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
                let has = !app.selection.is_empty();
                if icon_button(ui, icon::FLIP_HORIZONTAL, tr("Horizontal spiegeln"), has) {
                    app.mirror(true);
                }
                if icon_button(ui, icon::FLIP_VERTICAL, tr("Vertikal spiegeln"), has) {
                    app.mirror(false);
                }
                if icon_button(ui, icon::ARROW_CLOCKWISE, tr("90° drehen"), has) {
                    app.rotate_selection(90.0);
                }
                let hint = app.keys.hint(tr("Gruppieren"), Action::Group);
                if icon_button(ui, icon::SELECTION, &hint, app.selection.len() > 1) {
                    app.group();
                }
                if icon_button(ui, icon::SELECTION_SLASH, tr("Gruppierung lösen"), has) {
                    app.ungroup();
                }
                if icon_button(ui, icon::CROSSHAIR, tr("Auf Arbeitsfläche zentrieren"), has) {
                    app.center_on_bed();
                }
                if icon_button(ui, icon::TRASH, tr("Löschen (Entf)"), has) {
                    app.delete_selection();
                }
            });
        });
}

pub fn bottom_bar(app: &mut VectorLumeApp, ui: &mut Ui) {
    let pal = theme::of(ui);
    egui::Panel::bottom("status").frame(Frame::NONE.fill(pal.panel).inner_margin(Margin::symmetric(12, 4))).show(
        ui,
        |ui| {
            ui.horizontal(|ui| {
                let small = |t: String| RichText::new(t).size(12.0).color(pal.text_weak);
                match app.view.cursor {
                    Some(c) => {
                        let m = app.settings.profile.doc_to_machine(c);
                        let (x, y) = (crate::units::num(m.x, 2), crate::units::num(m.y, 2));
                        ui.label(small(format!("X {x:>7}  Y {y:>7} {}", crate::units::unit())));
                    }
                    None => {
                        ui.label(small("X –  Y –".into()));
                    }
                }
                ui.separator();
                let b = app.selection_bounds();
                if app.selection.is_empty() {
                    ui.label(small(trs(&format!("{} Objekte", app.doc.objects.len()))));
                } else {
                    ui.label(small(trs(&format!(
                        "{} ausgewählt · {} × {}",
                        app.selection.len(),
                        crate::units::num(b.width(), 2),
                        crate::units::len(b.height(), 2)
                    ))));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(small(format!("{:.0} %", app.view.zoom / 3.78 * 100.0)));
                    ui.separator();
                    let (dot, text) = match (&app.machine.status, app.machine.connected) {
                        (_, false) => (pal.text_weak, tr("Nicht verbunden").to_string()),
                        (Some(st), true) => {
                            let c = match st.state {
                                vl_device::MachineState::Alarm | vl_device::MachineState::Door => pal.danger,
                                vl_device::MachineState::Run
                                | vl_device::MachineState::Jog
                                | vl_device::MachineState::Home => pal.accent,
                                vl_device::MachineState::Hold => pal.warning,
                                _ => pal.success,
                            };
                            let pos = app
                                .machine
                                .head_position()
                                .map(|p| format!(" · X {:.2} Y {:.2}", p.x, p.y))
                                .unwrap_or_default();
                            (c, format!("{}{}", tr(st.state.label()), pos))
                        }
                        (None, true) => (pal.warning, tr("Verbunden, warte auf Status…").into()),
                    };
                    ui.label(small(text));
                    let (r, painter) = ui.allocate_painter(vec2(10.0, 10.0), Sense::hover());
                    painter.circle_filled(r.rect.center(), 4.0, dot);
                    if app.jobs.busy > 0 {
                        ui.separator();
                        ui.spinner();
                        ui.label(small(tr("Berechne…").into()));
                    }
                });
            });
        },
    );

    egui::Panel::bottom("palette")
        .frame(
            Frame::NONE.fill(pal.panel_alt).inner_margin(Margin::symmetric(10, 6)).stroke(Stroke::new(1.0, pal.border)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                ui.label(RichText::new(tr("Ebene")).size(12.0).color(pal.text_weak));
                ui.add_space(4.0);
                let used = app.doc.used_layers();
                for i in 0..LAYER_COUNT {
                    let layer = &app.doc.layers[i];
                    let color = theme::layer_color(layer.color);
                    let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                    let current = app.current_layer == i;
                    let painter = ui.painter();
                    painter.rect_filled(
                        rect.shrink(if resp.hovered() { 0.0 } else { 1.0 }),
                        CornerRadius::same(5),
                        color,
                    );
                    if current {
                        painter.rect_stroke(
                            rect.expand(1.5),
                            CornerRadius::same(6),
                            Stroke::new(2.0, pal.text),
                            egui::StrokeKind::Outside,
                        );
                    }
                    if used.contains(&i) {
                        let c = if layer.color.iter().map(|&v| v as u32).sum::<u32>() > 450 {
                            Color32::BLACK
                        } else {
                            Color32::WHITE
                        };
                        painter.circle_filled(rect.center(), 2.5, c);
                    }
                    let tip = format!("{} – {}", layer.name, trs(&layer.settings.summary()));
                    if resp.on_hover_text(tip).on_hover_text(tr("Klick: Auswahl dieser Ebene zuweisen")).clicked() {
                        app.assign_layer(i);
                    }
                }
            });
        });
}
