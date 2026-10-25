//! Oberflächentests ohne Bildschirm (egui_kittest): die App läuft unsichtbar mit eigenem Einstellungsordner,
//! Knöpfe werden über ihre Beschriftung bedient. Die mit `#[ignore]` markierten Tests rendern zusätzlich Bilder
//! nach `target/ui-bilder` zur Sichtprüfung: `cargo test -p vectorlume -- --ignored`.

use crate::app::VectorLumeApp;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::sync::Once;
use vl_core::{Document, ObjectId, Point, Shape, VectorPath};

fn harness(render: bool) -> Harness<'static, VectorLumeApp> {
    harness_sized(render, egui::vec2(1600.0, 1000.0))
}

fn harness_sized(render: bool, size: egui::Vec2) -> Harness<'static, VectorLumeApp> {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // Eigener, bei jedem Lauf geleerter Ordner: keine alte Autosicherung, nie die echten Einstellungen
        let dir = std::env::temp_dir().join("vectorlume-ui-tests");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        // SAFETY: einmalig vor dem ersten Start der App; danach wird die Variable nur gelesen.
        unsafe { std::env::set_var("VECTORLUME_CONFIG_DIR", &dir) };
    });
    let mut builder = Harness::builder().with_size(size);
    if render {
        builder = builder.wgpu();
    }
    let mut h = builder.build_eframe(|cc| VectorLumeApp::new(cc));
    {
        let app = h.state_mut();
        app.doc = Document::new();
        app.selection.clear();
        app.modal = None;
    }
    h.run_ok();
    h
}

fn add(h: &mut Harness<'static, VectorLumeApp>, path: VectorPath) -> ObjectId {
    let app = h.state_mut();
    let id = app.doc.add_path("Form", 0, path);
    app.selection = vec![id];
    id
}

fn save(h: &mut Harness<'static, VectorLumeApp>, name: &str) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/ui-bilder");
    let _ = std::fs::create_dir_all(&dir);
    let img = h.render().expect("Rendern");
    img.save(dir.join(format!("{name}.png"))).expect("Bild speichern");
}

fn segments(h: &Harness<'static, VectorLumeApp>, id: ObjectId) -> usize {
    match &h.state().doc.get(id).unwrap().shape {
        Shape::Path(p) => p.segment_count(),
        Shape::Image(_) => 0,
    }
}

#[test]
fn corners_dialog_rounds_selection() {
    let mut h = harness(false);
    let id = add(&mut h, VectorPath::rect(50.0, 50.0, 60.0, 40.0, 0.0));
    let before = segments(&h, id);
    h.state_mut().shape_dialogs.corners_open = true;
    h.run_ok();
    h.get_by_label("Anwenden").click();
    h.run_ok();
    assert!(segments(&h, id) > before, "{} → {}", before, segments(&h, id));
}

#[test]
fn arrange_menu_lists_new_tools() {
    let mut h = harness(false);
    add(&mut h, VectorPath::rect(50.0, 50.0, 60.0, 40.0, 0.0));
    h.get_by_label("Anordnen").click();
    h.run_ok();
    // Untermenüs tragen einen Pfeil in der Beschriftung
    assert!(h.query_by_label_contains("Andocken").is_some(), "Andocken fehlt im Menü");
    for label in ["Ecken abrunden…", "Schlitze anpassen…", "Verschachteln…", "Formen vereinfachen…"] {
        assert!(h.query_by_label(label).is_some(), "{label} fehlt im Menü");
    }
}

#[test]
fn dock_and_cut_change_document() {
    let mut h = harness(false);
    let left = add(&mut h, VectorPath::rect(20.0, 20.0, 30.0, 30.0, 0.0));
    let right = add(&mut h, VectorPath::rect(100.0, 30.0, 20.0, 20.0, 0.0));
    let app = h.state_mut();
    app.dock_selection(Point::new(-1.0, 0.0));
    let b = app.doc.bounds_of(&[right]);
    assert!((b.min.x - 50.0).abs() < 1e-6, "{b:?}");
    // Das rechte Quadrat ragt jetzt nicht hinein – zum Schneiden darüber schieben
    app.transform_selection(&vl_core::Affine::translate(-10.0, 0.0));
    app.selection = vec![left, right];
    app.cut_shapes();
    assert_eq!(app.doc.objects.len(), 2, "linkes Quadrat in zwei Teile, Schneidform entfällt");
}

#[test]
fn image_panel_offers_sharpening() {
    let mut h = harness(false);
    add(&mut h, VectorPath::rect(40.0, 40.0, 30.0, 30.0, 0.0));
    h.state_mut().convert_to_bitmap();
    h.run_ok();
    assert!(h.state().doc.objects.iter().any(|o| o.is_image()));
    assert!(h.query_by_label("Schärfen").is_some());
}

#[test]
fn node_tool_extends_open_end_to_wall() {
    let mut h = harness(false);
    let mut line = VectorPath::default();
    line.subpaths.push(vl_core::SubPath {
        start: Point::new(10.0, 50.0),
        segments: vec![vl_core::Segment::Line(Point::new(30.0, 50.0))],
        closed: false,
    });
    let id = add(&mut h, line);
    add(&mut h, VectorPath::rect(80.0, 20.0, 10.0, 60.0, 0.0));
    {
        let app = h.state_mut();
        app.selection = vec![id];
        app.tool = crate::app::Tool::Node;
        app.node_edit.target = Some(id);
        app.node_edit.selected = Some(vl_core::nodes::NodeRef { sub: 0, idx: 1 });
    }
    h.run_ok();
    h.get_by_label_contains("Verlängern").click();
    h.run_ok();
    let Shape::Path(p) = &h.state().doc.get(id).unwrap().shape else { panic!() };
    assert_eq!(p.subpaths[0].end(), Point::new(80.0, 50.0));
    // Auftrennen in der Mitte geht bei einer einzelnen Linie nicht, am Ende auch nicht: Meldung statt Änderung
    h.get_by_label_contains("Auftrennen").click();
    h.run_ok();
    let Shape::Path(p) = &h.state().doc.get(id).unwrap().shape else { panic!() };
    assert_eq!(p.subpaths.len(), 1);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_dialogs() {
    let mut h = harness(true);
    add(&mut h, VectorPath::star(Point::new(120.0, 120.0), 50.0, 20.0, 5));
    h.state_mut().shape_dialogs.corners_open = true;
    h.run_ok();
    save(&mut h, "ecken_dialog");
    h.get_by_label("Anwenden").click();
    h.run_ok();
    h.state_mut().shape_dialogs.corners_open = false;
    h.run_ok();
    save(&mut h, "ecken_ergebnis");

    h.get_by_label("Anordnen").click();
    h.run_ok();
    save(&mut h, "menue_anordnen");
    h.get_by_label_contains("Andocken").hover();
    h.run_ok();
    save(&mut h, "menue_andocken");
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_image_panel() {
    let mut h = harness_sized(true, egui::vec2(1600.0, 1700.0));
    add(&mut h, VectorPath::ellipse(Point::new(100.0, 100.0), 40.0, 30.0));
    h.state_mut().convert_to_bitmap();
    h.run_ok();
    if let Some(Shape::Image(img)) = h.state_mut().doc.objects.iter_mut().find(|o| o.is_image()).map(|o| &mut o.shape) {
        img.adjust.sharpen = 120.0;
    }
    h.run_ok();
    save(&mut h, "bild_schaerfen");
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_all_dialogs() {
    type Open = fn(&mut VectorLumeApp, bool);
    let dialogs: [(&str, Open); 14] = [
        ("versatz", |a, v| a.shape_dialogs.offset_open = v),
        ("reihen_kopien", |a, v| a.shape_dialogs.array_open = v),
        ("in_bitmap", |a, v| a.shape_dialogs.bitmap_open = v),
        ("vereinfachen", |a, v| a.shape_dialogs.simplify_open = v),
        ("verschachteln", |a, v| a.shape_dialogs.nest_open = v),
        ("schlitze", |a, v| a.shape_dialogs.slots_open = v),
        ("notizen", |a, v| a.show_notes = v),
        ("geraet", |a, v| a.show_device_settings = v),
        ("drehachse", |a, v| a.show_rotary = v),
        ("info", |a, v| a.show_about = v),
        ("drucken_schneiden", |a, v| a.print_cut.open = v),
        ("materialbibliothek", |a, v| a.materials.open = v),
        ("materialtest", |a, v| a.test.open = v),
        ("text", |a, v| a.text_dialog.open = v),
    ];
    let mut h = harness(true);
    add(&mut h, VectorPath::rect(60.0, 60.0, 80.0, 50.0, 0.0));
    h.state_mut().doc.notes.text = "Pappel 3 mm, Fokus 2 mm tiefer".into();
    for (name, open) in dialogs {
        open(h.state_mut(), true);
        h.run_ok();
        save(&mut h, &format!("dialog_{name}"));
        open(h.state_mut(), false);
        h.run_ok();
    }
    // Werkzeugleisten von Knoten- und Trimmwerkzeug
    for (name, tool) in [("werkzeug_knoten", crate::app::Tool::Node), ("werkzeug_trimmen", crate::app::Tool::Trim)] {
        h.state_mut().tool = tool;
        h.run_ok();
        save(&mut h, name);
    }
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_panels_and_menus() {
    let mut h = harness(true);
    h.state_mut().load_demo();
    h.state_mut().zoom_to_fit();
    h.run_ok();
    save(&mut h, "demo");
    // Vorschau: der Job wird im Hintergrund berechnet
    h.state_mut().preview.active = true;
    for _ in 0..100 {
        h.step();
        if h.state().preview.job.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    h.run_ok();
    assert!(h.state().preview.job.is_some(), "Vorschau wurde nicht berechnet: {:?}", h.state().preview.error);
    save(&mut h, "vorschau");
    h.state_mut().preview.active = false;
    for (name, tab) in [
        ("reiter_bewegen", crate::ui_side::RightTab::Jog),
        ("reiter_konsole", crate::ui_side::RightTab::Console),
        ("reiter_bibliothek", crate::ui_side::RightTab::Library),
    ] {
        h.state_mut().right_tab = tab;
        h.run_ok();
        save(&mut h, name);
    }
    for menu in ["Datei", "Bearbeiten", "Ansicht", "Laser"] {
        h.get_by_label(menu).click();
        h.run_ok();
        save(&mut h, &format!("menue_{}", menu.to_lowercase()));
        h.key_press(egui::Key::Escape);
        h.run_ok();
    }
}
