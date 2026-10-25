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
        app.settings.units = crate::units::Units::Mm;
        crate::units::set_units(crate::units::Units::Mm);
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
fn keys_reorder_and_paste_in_place() {
    let mut h = harness(false);
    let ids: Vec<ObjectId> =
        (0..3).map(|k| add(&mut h, VectorPath::rect(10.0 + 20.0 * k as f64, 10.0, 10.0, 10.0, 0.0))).collect();
    let order = |h: &Harness<'static, VectorLumeApp>| h.state().doc.objects.iter().map(|o| o.id).collect::<Vec<_>>();
    h.state_mut().selection = vec![ids[0]];
    h.key_press(egui::Key::PageUp);
    h.run_ok();
    assert_eq!(order(&h), vec![ids[1], ids[0], ids[2]]);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::PageUp);
    h.run_ok();
    assert_eq!(order(&h), vec![ids[1], ids[2], ids[0]]);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::PageDown);
    h.run_ok();
    assert_eq!(order(&h), ids);

    let before = h.state().doc.bounds_of(&[ids[0]]);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::C);
    h.run_ok();
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::V);
    h.run_ok();
    let app = h.state();
    assert_eq!(app.doc.objects.len(), 4);
    assert_eq!(app.doc.bounds_of(&app.selection), before, "an Ort und Stelle");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::V);
    h.run_ok();
    let app = h.state();
    assert_eq!(app.doc.bounds_of(&app.selection).min.x, before.min.x + 5.0, "normal versetzt");
}

/// Verzerren: Eckgriff mit der Maus ziehen, mit Enter übernehmen; Esc bricht ab und lässt die Auswahl stehen.
fn warp_flow(render: bool) {
    use vl_core::warp::WarpKind;
    let mut h = harness(render);
    let id = add(&mut h, VectorPath::rect(40.0, 40.0, 60.0, 30.0, 0.0));
    h.state_mut().settings.snap_to_grid = true;
    h.state_mut().start_warp(WarpKind::Perspective);
    h.run_ok();
    assert!(h.query_by_label_contains("Anwenden").is_some());
    // Griff oben rechts (100/40) nach 110/20 ziehen
    let tf = h.state().view.tf.expect("Ansicht");
    let (from, to) = (tf.to_screen(Point::new(100.0, 40.0)), tf.to_screen(Point::new(110.0, 20.0)));
    h.hover_at(from);
    h.run_ok();
    h.drag_at(from);
    h.run_ok();
    for k in 1..=4 {
        h.hover_at(from + (to - from) * (k as f32 / 4.0));
        h.run_ok();
    }
    h.drop_at(to);
    h.run_ok();
    let handle = h.state().warp.as_ref().expect("Sitzung").warp.handles[1];
    assert!(handle.distance(Point::new(110.0, 20.0)) < 0.6, "{handle:?}");
    if render {
        save(&mut h, "verzerren_perspektive");
    }
    h.state_mut().warp.as_mut().unwrap().warp.set_kind(WarpKind::Envelope);
    h.state_mut().warp.as_mut().unwrap().warp.handles[6] = Point::new(70.0, 80.0);
    h.run_ok();
    if render {
        save(&mut h, "verzerren_huelle");
    }
    h.key_press(egui::Key::Enter);
    h.run_ok();
    let app = h.state();
    assert!(app.warp.is_none());
    let b = app.doc.bounds();
    assert!((b.max.x - 110.0).abs() < 0.6 && (b.min.y - 20.0).abs() < 0.6 && (b.max.y - 80.0).abs() < 0.05, "{b:?}");
    let bent = match &h.state().doc.get(id).unwrap().shape {
        Shape::Path(p) => p.subpaths[0].segments.iter().any(|s| matches!(s, vl_core::Segment::Cubic(..))),
        Shape::Image(_) => false,
    };
    assert!(bent, "untere Kante gebogen");
    // Abbrechen
    h.state_mut().start_warp(WarpKind::Perspective);
    h.run_ok();
    h.key_press(egui::Key::Escape);
    h.run_ok();
    assert!(h.state().warp.is_none() && h.state().selection == vec![id]);
    assert!(h.state_mut().history.can_undo());
}

#[test]
fn warp_drags_handle_and_applies() {
    warp_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_warp() {
    warp_flow(true);
}

/// Mittellinien nachzeichnen: Strichzeichnung (Ring, Diagonale, X, Bogen) aus wenigen offenen Linienzügen.
fn centerline_flow(render: bool) {
    let mut h = harness(render);
    let (w, hgt) = (600u32, 300u32);
    let ink = |x: f64, y: f64| {
        let ring = ((x - 120.0).hypot(y - 150.0) - 80.0).abs() < 6.0;
        let seg = |ax: f64, ay: f64, bx: f64, by: f64| {
            let (dx, dy) = (bx - ax, by - ay);
            let t = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
            (x - ax - t * dx).hypot(y - ay - t * dy) < 5.0
        };
        let cross = seg(250.0, 60.0, 380.0, 240.0) || seg(380.0, 60.0, 250.0, 240.0);
        let arc = (x - 500.0).hypot(y - 240.0) < 80.0 + 4.0 && (x - 500.0).hypot(y - 240.0) > 80.0 - 4.0 && y < 240.0;
        ring || cross || arc
    };
    let pixels = (0..w * hgt).map(|i| if ink((i % w) as f64, (i / w) as f64) { 0 } else { 255 }).collect();
    let image = vl_core::ImageObject {
        image: std::sync::Arc::new(vl_core::RasterImage::new(w, hgt, pixels)),
        width_mm: 120.0,
        height_mm: 60.0,
        adjust: Default::default(),
        clip: vec![],
    };
    let id = h.state_mut().doc.add_image("Strich", 0, image, Point::new(30.0, 60.0));
    h.state_mut().selection = vec![id];
    h.state_mut().open_trace();
    h.state_mut().trace.opts.centerline = true;
    for _ in 0..200 {
        h.run_ok();
        if h.state().trace.result.is_some() && !h.state().trace.busy {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let paths = h.state().trace.result.clone().expect("Ergebnis");
    let polys: Vec<vl_core::Polyline> =
        paths.iter().flat_map(|p| p.flatten(&vl_core::Affine::IDENTITY, 0.05)).collect();
    if render {
        let b = h.state().doc.bounds();
        h.state_mut().view.request_fit(Some(b));
        h.run_ok();
        save(&mut h, "nachzeichnen_mittellinien");
    }
    // Ring (geschlossen), X (zwei durchgehende Diagonalen), Bogen (offen)
    assert_eq!(polys.len(), 4);
    assert_eq!(polys.iter().filter(|p| p.closed).count(), 1);
}

#[test]
fn centerline_trace_of_line_art() {
    centerline_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_centerline() {
    centerline_flow(true);
}

/// Grundformen zeigen ihre Parameter in der oberen Leiste, gewöhnliche Pfade nicht.
fn primitive_fields_flow(render: bool) {
    use vl_core::primitive::Primitive;
    let mut h = harness(render);
    let star = Primitive::Star { center: Point::new(150.0, 150.0), radius: 40.0, inner: 0.45, points: 5 };
    let id = h.state_mut().doc.add_primitive("Stern", 0, star);
    h.state_mut().selection = vec![id];
    h.run_ok();
    assert!(h.query_by_label("Zacken").is_some() && h.query_by_label("Innenradius").is_some());
    if render {
        save(&mut h, "leiste_stern");
    }
    let rect = Primitive::Rect { x: 40.0, y: 40.0, width: 60.0, height: 30.0, radius: 0.0 };
    let id = h.state_mut().doc.add_primitive("Rechteck", 0, rect);
    h.state_mut().selection = vec![id];
    h.run_ok();
    assert!(h.query_by_label("Zacken").is_none());
    if render {
        save(&mut h, "leiste_rechteck");
    }
    // Knoten bearbeiten macht daraus einen gewöhnlichen Pfad
    let app = h.state_mut();
    if let Some(Shape::Path(p)) = app.doc.get_mut(id).map(|o| &mut o.shape) {
        vl_core::nodes::move_node(p, vl_core::nodes::NodeRef { sub: 0, idx: 1 }, Point::new(120.0, 35.0));
    }
    assert!(app.doc.get(id).unwrap().live_primitive().is_none());
}

#[test]
fn primitive_fields_follow_selection() {
    primitive_fields_flow(false);
}

/// Anzeige in Zoll: Maße in der Statusleiste umgerechnet, das Dokument bleibt in mm.
fn inch_flow(render: bool) {
    let mut h = harness(render);
    let id = h.state_mut().doc.add_path("R", 0, vl_core::VectorPath::rect(50.8, 50.8, 25.4, 12.7, 0.0));
    h.state_mut().selection = vec![id];
    h.state_mut().set_units(crate::units::Units::Inch);
    h.run_ok();
    assert!(h.query_by_label_contains("1 ausgewählt · 1.000 × 0.500 in").is_some());
    if render {
        save(&mut h, "zoll_ansicht");
    }
    assert!((h.state().doc.bounds().width() - 25.4).abs() < 1e-9);
    h.state_mut().set_units(crate::units::Units::Mm);
    h.run_ok();
    assert!(h.query_by_label_contains("1 ausgewählt · 25.40 × 12.70 mm").is_some());
}

#[test]
fn units_switch_to_inch() {
    inch_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_inch() {
    inch_flow(true);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_primitive_fields() {
    primitive_fields_flow(true);
}

/// Mittelpunkt finden mit vorgetäuschter Kopfposition: drei Randpunkte übernehmen, Kreis ins Projekt.
fn center_finder_flow(render: bool) {
    let mut h = harness(render);
    let at = |h: &mut Harness<'static, VectorLumeApp>, x: f64, y: f64| {
        h.state_mut().machine.status = Some(vl_device::Status { wpos: Some([x, y, 0.0]), ..Default::default() });
        h.run_ok();
    };
    {
        let app = h.state_mut();
        app.machine.connected = true;
        app.open_center_finder();
    }
    for (x, y) in [(130.0, 80.0), (100.0, 110.0), (70.0, 80.0)] {
        at(&mut h, x, y);
        h.get_by_label(&format!("{} Punkt übernehmen", egui_phosphor::regular::PLUS)).click();
        h.run_ok();
    }
    let (c, r) = h.state().center_finder.result().expect("Kreis");
    assert!(c.distance(Point::new(100.0, 80.0)) < 1e-9 && (r.unwrap() - 30.0).abs() < 1e-9);
    assert!(h.query_by_label_contains("Durchmesser: 60.00 mm").is_some());
    if render {
        save(&mut h, "dialog_mittelpunkt");
    }
    h.get_by_label_contains("Ins Projekt").click();
    h.run_ok();
    // Maschinen- in Dokumentkoordinaten (Nullpunkt der Maschine z. B. unten links: Y gespiegelt)
    let mid = h.state().settings.profile.machine_to_doc(Point::new(100.0, 80.0));
    let b = h.state().doc.bounds();
    assert!(b.center().distance(mid) < 1e-6 && (b.width() - 60.0).abs() < 1e-6, "{b:?} {mid:?}");
    h.state_mut().machine.connected = false;
}

#[test]
fn center_finder_takes_points_and_inserts_circle() {
    center_finder_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_center_finder() {
    center_finder_flow(true);
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
