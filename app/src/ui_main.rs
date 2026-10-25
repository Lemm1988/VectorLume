//! Menü, Werkzeugleisten, Eigenschaftsfelder, Farbpalette, Statusleiste und Tastenkürzel.

use crate::app::{Tool, VectorLumeApp};
use crate::i18n::{tr, trs};
use crate::settings::ThemeChoice;
use crate::theme;
use egui::{
    Align, Button, Color32, CornerRadius, DragValue, Frame, Key, KeyboardShortcut, Layout, Margin, Modifiers, RichText,
    Sense, Stroke, Ui, vec2,
};
use egui_phosphor::regular as icon;
use vl_core::{Affine, LAYER_COUNT, Point};

pub fn shortcuts(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if ctx.egui_wants_keyboard_input() || app.modal.is_some() {
        return;
    }
    let sc = |m: Modifiers, k: Key| KeyboardShortcut::new(m, k);
    let cs = Modifiers::COMMAND | Modifiers::SHIFT;
    let pressed = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));

    if pressed(sc(cs, Key::S)) {
        app.save(true);
    } else if pressed(sc(Modifiers::COMMAND, Key::S)) {
        app.save(false);
    }
    if pressed(sc(Modifiers::COMMAND, Key::N)) {
        app.new_document(false);
    }
    if pressed(sc(Modifiers::COMMAND, Key::O)) {
        app.open_dialog(false, None);
    }
    if pressed(sc(Modifiers::COMMAND, Key::I)) {
        app.import_dialog();
    }
    if pressed(sc(Modifiers::COMMAND, Key::E)) {
        app.export_gcode_dialog();
    }
    if pressed(sc(cs, Key::Z)) || pressed(sc(Modifiers::COMMAND, Key::Y)) {
        app.redo();
    } else if pressed(sc(Modifiers::COMMAND, Key::Z)) {
        app.undo();
    }
    if pressed(sc(Modifiers::COMMAND, Key::A)) {
        app.select_all();
    }
    if pressed(sc(Modifiers::COMMAND, Key::D)) {
        app.duplicate();
    }
    if pressed(sc(Modifiers::COMMAND, Key::C)) {
        app.copy();
    }
    if pressed(sc(Modifiers::COMMAND, Key::X)) {
        app.cut();
    }
    if pressed(sc(Modifiers::COMMAND, Key::V)) {
        app.paste();
    }
    if pressed(sc(cs, Key::G)) {
        app.ungroup();
    } else if pressed(sc(Modifiers::COMMAND, Key::G)) {
        app.group();
    }
    if pressed(sc(Modifiers::ALT, Key::P)) {
        app.preview.active = !app.preview.active;
    }
    if pressed(sc(Modifiers::NONE, Key::Delete)) || pressed(sc(Modifiers::NONE, Key::Backspace)) {
        if app.tool == Tool::Node {
            app.delete_selected_node();
        } else {
            app.delete_selection();
        }
    }
    if pressed(sc(Modifiers::NONE, Key::Escape)) {
        if app.preview.active {
            app.preview.active = false;
        } else if app.tool != Tool::Select {
            app.tool = Tool::Select;
        } else {
            app.selection.clear();
        }
    }
    for (k, t) in [
        (Key::V, Tool::Select),
        (Key::R, Tool::Rect),
        (Key::E, Tool::Ellipse),
        (Key::L, Tool::Line),
        (Key::T, Tool::Text),
        (Key::Q, Tool::Barcode),
        (Key::N, Tool::Node),
        (Key::M, Tool::Measure),
    ] {
        if pressed(sc(Modifiers::NONE, k)) {
            app.tool = t;
        }
    }
    if pressed(sc(Modifiers::COMMAND, Key::Num0)) || pressed(sc(Modifiers::NONE, Key::F)) {
        app.zoom_to_fit();
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
        b = b.shortcut_text(crate::i18n::shortcut(shortcut));
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
                if icon_button(ui, icon::FILE_PLUS, tr("Neu (Strg+N)"), true) {
                    app.new_document(false);
                }
                if icon_button(ui, icon::FOLDER_OPEN, tr("Öffnen (Strg+O)"), true) {
                    app.open_dialog(false, None);
                }
                if icon_button(ui, icon::FLOPPY_DISK, tr("Speichern (Strg+S)"), true) {
                    app.save(false);
                }
                if icon_button(ui, icon::DOWNLOAD_SIMPLE, tr("Importieren: SVG, DXF, Bilder (Strg+I)"), true) {
                    app.import_dialog();
                }
                if icon_button(ui, icon::EXPORT, tr("G-Code speichern (Strg+E)"), !app.doc.objects.is_empty()) {
                    app.export_gcode_dialog();
                }
                ui.separator();
                if icon_button(ui, icon::ARROW_U_UP_LEFT, tr("Rückgängig (Strg+Z)"), app.history.can_undo()) {
                    app.undo();
                }
                if icon_button(ui, icon::ARROW_U_UP_RIGHT, tr("Wiederholen (Strg+Y)"), app.history.can_redo()) {
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
                if ui.add(b).on_hover_text(tr("Werkzeugweg und Laufzeit anzeigen (Alt+P)")).clicked() {
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
    if menu_item(ui, tr("Neu"), "Strg+N", true) {
        app.new_document(false);
    }
    if menu_item(ui, tr("Öffnen…"), "Strg+O", true) {
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
    if menu_item(ui, tr("Speichern"), "Strg+S", true) {
        app.save(false);
    }
    if menu_item(ui, tr("Speichern unter…"), tr("Strg+Umschalt+S"), true) {
        app.save(true);
    }
    ui.separator();
    if menu_item(ui, tr("Importieren…"), "Strg+I", true) {
        app.import_dialog();
    }
    ui.separator();
    let has = !app.doc.objects.is_empty();
    if menu_item(ui, tr("G-Code speichern…"), "Strg+E", has) {
        app.export_gcode_dialog();
    }
    if menu_item(ui, tr("Als SVG exportieren…"), "", has) {
        app.export_svg_dialog();
    }
    if menu_item(ui, tr("Als DXF exportieren…"), "", has) {
        app.export_dxf_dialog();
    }
    ui.separator();
    if menu_item(ui, tr("Beenden"), "", true) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

fn edit_menu(app: &mut VectorLumeApp, ui: &mut Ui) {
    let has = !app.selection.is_empty();
    if menu_item(ui, tr("Rückgängig"), "Strg+Z", app.history.can_undo()) {
        app.undo();
    }
    if menu_item(ui, tr("Wiederholen"), "Strg+Y", app.history.can_redo()) {
        app.redo();
    }
    ui.separator();
    if menu_item(ui, tr("Ausschneiden"), "Strg+X", has) {
        app.cut();
    }
    if menu_item(ui, tr("Kopieren"), "Strg+C", has) {
        app.copy();
    }
    if menu_item(ui, tr("Einfügen"), "Strg+V", !app.clipboard.is_empty()) {
        app.paste();
    }
    if menu_item(ui, tr("Duplizieren"), "Strg+D", has) {
        app.duplicate();
    }
    if menu_item(ui, tr("Löschen"), "Entf", has) {
        app.delete_selection();
    }
    ui.separator();
    if menu_item(ui, tr("Alles auswählen"), "Strg+A", !app.doc.objects.is_empty()) {
        app.select_all();
    }
    if menu_item(ui, tr("Auswahl umkehren"), "", !app.doc.objects.is_empty()) {
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

fn arrange_menu(app: &mut VectorLumeApp, ui: &mut Ui) {
    let has = !app.selection.is_empty();
    let multi = app.selection.len() > 1;
    if menu_item(ui, tr("Gruppieren"), "Strg+G", multi) {
        app.group();
    }
    if menu_item(ui, tr("Gruppierung lösen"), tr("Strg+Umschalt+G"), has) {
        app.ungroup();
    }
    if menu_item(ui, tr("Zerlegen"), "", has) {
        app.break_apart();
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
    ui.separator();
    if menu_item(ui, tr("Horizontal spiegeln"), "", has) {
        app.mirror(true);
    }
    if menu_item(ui, tr("Vertikal spiegeln"), "", has) {
        app.mirror(false);
    }
    if menu_item(ui, tr("90° im Uhrzeigersinn drehen"), "", has) {
        app.rotate_selection(90.0);
    }
    if menu_item(ui, tr("90° gegen den Uhrzeigersinn drehen"), "", has) {
        app.rotate_selection(-90.0);
    }
    if menu_item(ui, tr("Auf Arbeitsfläche zentrieren"), "", has) {
        app.center_on_bed();
    }
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
            ui.separator();
            if menu_item(ui, tr("Verschweißen"), "", has) {
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
                    .suffix(" mm"),
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
    if menu_item(ui, tr("Reihen-Kopien…"), "", has) {
        app.shape_dialogs.array_open = true;
    }
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
    if menu_item(ui, tr("Arbeitsfläche einpassen"), "F", true) {
        app.zoom_to_fit();
    }
    if menu_item(ui, tr("Auf Auswahl zoomen"), "", !app.selection.is_empty()) {
        let b = app.selection_bounds();
        app.view.request_fit(Some(b));
    }
    ui.separator();
    ui.checkbox(&mut app.preview.active, tr("Vorschau (Alt+P)"));
    if ui.checkbox(&mut app.settings.show_travel, tr("Leerfahrten in der Vorschau")).changed() {
        app.settings.save();
    }
    if ui.checkbox(&mut app.settings.snap_to_grid, tr("Am Raster fangen (1 mm)")).changed() {
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
            ui.add(DragValue::new(v).speed(0.2).max_decimals(2).suffix(" mm").range(-10000.0..=10000.0))
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

/// Leistung der ausgewählten Objekte in Prozent der Ebenenleistung (z. B. für Materialtests).
fn power_scale_field(app: &mut VectorLumeApp, ui: &mut Ui) {
    let ids = app.selection.clone();
    let scales: Vec<f64> = ids.iter().filter_map(|id| app.doc.get(*id)).map(|o| o.power_scale).collect();
    let Some(&first) = scales.first() else { return };
    let mixed = scales.iter().any(|s| *s != first);
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
        for id in ids {
            if let Some(o) = app.doc.get_mut(id) {
                o.power_scale = v;
            }
        }
        app.mark_changed();
    }
}

fn tool_options(app: &mut VectorLumeApp, ui: &mut Ui) {
    if app.tool == Tool::Node {
        crate::node_tool::options(app, ui);
        return;
    }
    if app.tool == Tool::Measure {
        match app.measure {
            Some((a, b)) => ui.label(crate::canvas::measure_text(a, b)),
            None => ui.label(
                RichText::new(tr("Ziehen zum Messen · fängt an Knoten und Mitten")).color(theme::of(ui).text_weak),
            ),
        };
        return;
    }
    let o = &mut app.tool_options;
    match app.tool {
        Tool::Rect => {
            ui.label(tr("Eckenradius"));
            ui.add(DragValue::new(&mut o.corner_radius).speed(0.1).range(0.0..=1000.0).suffix(" mm"));
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
                    if ui.add(b).on_hover_text(tr(t.label())).clicked() {
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
                if icon_button(ui, icon::SELECTION, tr("Gruppieren (Strg+G)"), app.selection.len() > 1) {
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
                        ui.label(small(format!("X {:>7.2}  Y {:>7.2} mm", m.x, m.y)));
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
                        "{} ausgewählt · {:.2} × {:.2} mm",
                        app.selection.len(),
                        b.width(),
                        b.height()
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
