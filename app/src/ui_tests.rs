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
        app.pending_crash = None;
        app.setup.open = false;
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

/// Nachzeichnen für Ausschneideteile (z. B. Kühlschrankmagnete): jede Kontur ein Objekt, jedes Teil mit allem darin
/// eine eigene Gruppe, einzelne Flecken bleiben einzeln; Gruppen lassen sich lösen und – auch über Ebenen hinweg –
/// neu bilden.
#[test]
fn traced_parts_are_grouped_and_can_be_regrouped() {
    let mut h = harness(false);
    let (w, hgt) = (200u32, 100u32);
    let ink = |x: f64, y: f64| {
        // zwei Teile: Rand (Ring) mit Punkt in der Mitte, dazu zwei lose Flecken
        let part = |cx: f64| {
            let d = (x - cx).hypot(y - 50.0);
            (35.0..42.0).contains(&d) || d < 8.0
        };
        let speck = |cx: f64, cy: f64| (x - cx).abs() < 3.0 && (y - cy).abs() < 3.0;
        part(50.0) || part(150.0) || speck(100.0, 8.0) || speck(100.0, 92.0)
    };
    let pixels = (0..w * hgt).map(|i| if ink((i % w) as f64 + 0.5, (i / w) as f64 + 0.5) { 0 } else { 255 }).collect();
    let image = vl_core::ImageObject {
        image: std::sync::Arc::new(vl_core::RasterImage::new(w, hgt, pixels)),
        width_mm: 100.0,
        height_mm: 50.0,
        adjust: Default::default(),
        clip: vec![],
    };
    let img = h.state_mut().doc.add_image("Magnete", 0, image, Point::new(20.0, 20.0));
    h.state_mut().selection = vec![img];
    h.state_mut().open_trace();
    for _ in 0..200 {
        h.run_ok();
        if h.state().trace.result.is_some() && !h.state().trace.busy {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.state_mut().apply_trace();
    let app = h.state_mut();
    let traced: Vec<vl_core::ObjectId> = app.selection.clone();
    // je Teil: Außenrand, Innenrand, Punkt; dazu zwei Flecken
    assert_eq!(traced.len(), 8);
    let group_of = |app: &crate::app::VectorLumeApp, id| app.doc.get(id).unwrap().group;
    let groups: std::collections::HashSet<_> = traced.iter().map(|&id| group_of(app, id)).collect();
    assert_eq!(groups.iter().flatten().count(), 2, "zwei Teile");
    assert_eq!(traced.iter().filter(|&&id| group_of(app, id).is_none()).count(), 2, "Flecken einzeln");
    // ein Teil auswählen (Klick wählt die ganze Gruppe) und lösen: nur dessen drei Konturen werden frei
    let left = *traced
        .iter()
        .find(|&&id| app.doc.bounds_of(&[id]).center().x < 50.0 && app.doc.bounds_of(&[id]).width() > 30.0)
        .unwrap();
    app.selection = app.doc.expand_groups(&[left]);
    assert_eq!(app.selection.len(), 3);
    app.ungroup();
    assert!(app.selection.iter().all(|&id| group_of(app, id).is_none()));
    assert_eq!(traced.iter().filter(|&&id| group_of(app, id).is_none()).count(), 5);
    // neu gruppieren, eine Kontur liegt dabei auf einer anderen Ebene: die Gruppe hält trotzdem zusammen
    let (a, b) = (app.selection[0], app.selection[1]);
    app.doc.get_mut(b).unwrap().layer = 1;
    app.selection = vec![a, b];
    app.group();
    assert!(group_of(app, a).is_some() && group_of(app, a) == group_of(app, b));
    assert_eq!(app.doc.expand_groups(&[a]).len(), 2);
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

/// Laschen von Hand: Klick auf die Kontur setzt eine, Klick darauf entfernt sie wieder; der Job lässt die Lücke aus.
/// Klick in eine Vorlage: Schrift und Bild in einem größeren, darüberliegenden Umriss bleiben anklickbar; die Linie
/// des Umrisses wählt den Umriss, die freie Fläche der Vorlage die Vorlage.
#[test]
fn click_selects_smallest_shape_under_pointer() {
    let mut h = harness(false);
    let app = h.state_mut();
    let image = vl_core::ImageObject {
        image: std::sync::Arc::new(vl_core::RasterImage::new(16, 8, vec![128; 16 * 8])),
        width_mm: 20.0,
        height_mm: 10.0,
        adjust: Default::default(),
        clip: Vec::new(),
    };
    let logo = app.doc.add_image("Logo", 29, image, Point::new(60.0, 20.0));
    let text = app.doc.add_path("Schrift", 2, VectorPath::rect(20.0, 22.0, 30.0, 6.0, 0.0));
    let pen = app.doc.add_path("Stift", 3, VectorPath::rect(10.0, 15.0, 100.0, 20.0, 0.0));
    let jig = app.doc.add_path("Vorlage", 0, VectorPath::rect(0.0, 0.0, 297.0, 210.0, 0.0));
    let tol = 0.5;
    assert_eq!(app.hit_test(Point::new(30.0, 25.0), tol), Some(text));
    assert_eq!(app.hit_test(Point::new(70.0, 25.0), tol), Some(logo));
    assert_eq!(app.hit_test(Point::new(15.0, 30.0), tol), Some(pen));
    assert_eq!(app.hit_test(Point::new(10.2, 25.0), tol), Some(pen), "Linie des Stifts");
    assert_eq!(app.hit_test(Point::new(200.0, 100.0), tol), Some(jig));
    assert_eq!(app.hit_test(Point::new(400.0, 300.0), tol), None);
    // gesperrte Ebenen zählen nicht
    app.doc.layers[2].locked = true;
    assert_eq!(app.hit_test(Point::new(30.0, 25.0), tol), Some(pen));
}

/// „Benutzer-Ursprung“ verschiebt den Job: der Rahmen auf der Arbeitsfläche zeigt, wo er wirklich landet.
#[test]
fn output_footprint_follows_start_mode() {
    let mut h = harness(false);
    let app = h.state_mut();
    app.settings.profile.bed_height = 400.0;
    app.doc.add_path("Schrift", 0, VectorPath::rect(40.0, 370.0, 70.0, 8.0, 0.0));
    app.settings.job.start_from = vl_cam::StartFrom::Absolute;
    assert_eq!(app.output_footprint(), None);
    app.settings.job.start_from = vl_cam::StartFrom::UserOrigin;
    app.settings.job.user_origin = Point::new(0.0, 0.0);
    app.settings.job.anchor = vl_cam::Anchor::BottomLeft;
    // Ursprung vorne links (Maschine 0/0 = Dokument unten links): die Schrift rutscht in die Ecke
    let r = app.output_footprint().unwrap();
    assert!((r.min.x - 0.0).abs() < 1e-6 && (r.max.y - 400.0).abs() < 1e-6, "{r:?}");
    assert!((r.width() - 70.0).abs() < 1e-6 && (r.height() - 8.0).abs() < 1e-6);
}

/// Rechtsklick auf eine Ebene öffnet ihre Einstellungen in einem Fenster.
#[test]
fn right_click_on_layer_opens_settings_window() {
    layer_window_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_layer_window() {
    layer_window_flow(true);
}

fn layer_window_flow(render: bool) {
    let mut h = harness(render);
    {
        let app = h.state_mut();
        app.doc.add_path("A", 0, VectorPath::rect(10.0, 10.0, 20.0, 20.0, 0.0));
        app.doc.add_path("B", 3, VectorPath::rect(40.0, 10.0, 20.0, 20.0, 0.0));
        app.doc.layers[3].name = "Gravur".into();
    }
    h.run_ok();
    let rect = h.get_by_label("Gravur").rect();
    h.hover_at(rect.center());
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: rect.center(),
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    h.run_ok();
    assert!(h.state().layer_window.is_some());
    assert_eq!(h.state().current_layer, 3);
    assert!(h.query_by_label_contains("Ebene „Gravur“").is_some());
    if render {
        h.run_steps(4);
        save(&mut h, "ebenenfenster");
    }
    // Änderung im Fenster wirkt sofort, „Abbrechen“ stellt den alten Stand wieder her
    let before = h.state().doc.layers[3].clone();
    h.state_mut().doc.layers[3].settings.speed = 1234.0;
    h.run_ok();
    h.get_by_label("Abbrechen").click();
    h.run_ok();
    assert!(h.state().layer_window.is_none());
    assert_eq!(h.state().doc.layers[3], before);
    // Rechts in der Leiste bleibt nur die Schnelleinstellung
    assert!(h.query_by_label_contains("Einstellungen…").is_some());
}

fn tabs_flow(render: bool) {
    let mut h = harness(render);
    let id = add(&mut h, VectorPath::rect(40.0, 40.0, 60.0, 30.0, 0.0));
    h.state_mut().tool = crate::app::Tool::Tabs;
    h.state_mut().selection.clear();
    h.state_mut().doc.layers[0].settings.tab_width = 2.0;
    h.run_ok();
    let click = |h: &mut Harness<'static, VectorLumeApp>, p: Point| {
        // erst drücken, wenn Hintergrundberechnungen fertig sind und die Stelle zwei Bilder lang stillsteht
        let mut pos = None;
        for _ in 0..200 {
            let now = h.state().view.tf.map(|t| t.to_screen(p));
            if now.is_some() && now == pos && h.state().jobs.busy == 0 {
                break;
            }
            pos = now;
            if let Some(q) = pos {
                h.hover_at(q);
            }
            h.run_ok();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let pos = pos.expect("Ansicht");
        // Drücken und Loslassen in einem Lauf: jedes Ereignis bekommt genau ein Bild (je 0,25 s Testzeit). Mit
        // einem Lauf dazwischen könnten zusätzliche Bilder (z. B. nach einer fertigen Hintergrundberechnung) die
        // Klickdauer über 0,8 s treiben – dann wäre es kein Klick mehr.
        h.drag_at(pos);
        h.drop_at(pos);
        h.run_ok();
    };
    // knapp neben der oberen Kante: landet auf der Kontur
    click(&mut h, Point::new(70.0, 40.4));
    click(&mut h, Point::new(100.0, 55.0));
    let tabs = h.state().doc.get(id).unwrap().tabs.clone();
    assert_eq!(tabs.len(), 2, "{tabs:?}");
    assert!(tabs[0].distance(Point::new(70.0, 40.0)) < 1e-9, "{tabs:?}");
    if render {
        h.hover_at(h.state().view.tf.unwrap().to_screen(Point::new(55.0, 70.0)));
        h.run_ok();
        save(&mut h, "laschen_setzen");
    }
    // wandern mit dem Objekt
    h.state_mut().selection = vec![id];
    h.state_mut().nudge(5.0, 0.0);
    h.run_ok();
    click(&mut h, Point::new(75.0, 40.0));
    let tabs = &h.state().doc.get(id).unwrap().tabs;
    assert_eq!(tabs.len(), 1, "entfernt: {tabs:?}");
    // Job: Umfang 180 mm minus eine Lasche
    let doc = h.state().doc.clone();
    let profile = h.state().settings.profile.clone();
    let job = vl_cam::build_job(&doc, &[], &profile, &vl_cam::JobSettings::default()).unwrap();
    let mut cut = 0.0;
    let mut at = job.start;
    for b in &job.toolpath.blocks {
        for op in &b.ops {
            if let vl_cam::Op::Move { to, power } = *op
                && power > 0.0
            {
                cut += at.distance(to);
            }
            at = op.target();
        }
    }
    assert!((cut - 178.0).abs() < 1e-6, "{cut}");
    h.state_mut().selection.clear();
    h.state_mut().clear_tabs();
    assert!(h.state().doc.get(id).unwrap().tabs.is_empty());
}

#[test]
fn tabs_set_and_removed_by_click() {
    tabs_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_tabs() {
    tabs_flow(true);
}

/// Box-Generator: „Box erzeugen“ legt die Platten nebeneinander als Gruppe auf die aktuelle Ebene; unmögliche
/// Maße sperren den Knopf und zeigen den Grund.
fn box_flow(render: bool) {
    use vl_core::boxgen::BoxSpec;
    let mut h = harness(render);
    let before = h.state().doc.objects.len();
    h.state_mut().current_layer = 2;
    h.state_mut().box_gen.open = true;
    h.state_mut().box_gen.spec =
        BoxSpec { width: 120.0, depth: 90.0, height: 60.0, thickness: 3.0, finger: 12.0, ..Default::default() };
    h.run_ok();
    if render {
        save(&mut h, "box_generator");
    }
    h.get_by_label_contains("Box erzeugen").click();
    h.run_ok();
    let app = h.state();
    assert!(!app.box_gen.open);
    let new: Vec<&vl_core::Object> = app.doc.objects[before..].iter().collect();
    assert_eq!(new.len(), 6);
    assert!(new.iter().all(|o| o.layer == 2 && o.group.is_some() && o.group == new[0].group));
    assert_eq!(app.selection.len(), 6);
    let bed = app.settings.profile.bed_rect();
    let bounds: Vec<vl_core::Rect> = new.iter().map(|o| o.bounds()).collect();
    for (i, a) in bounds.iter().enumerate() {
        assert!(bed.contains_rect(a), "{a:?}");
        for b in &bounds[i + 1..] {
            assert!(!a.intersects(b), "Platten überlappen: {a:?} {b:?}");
        }
    }
    if render {
        let sel = h.state().selection_bounds().expanded(5.0);
        h.state_mut().view.request_fit(Some(sel));
        h.run_ok();
        save(&mut h, "box_platten");
    }
    h.state_mut().box_gen.open = true;
    h.state_mut().box_gen.spec.finger = 2.0;
    h.run_ok();
    assert!(h.query_by_label_contains("zu schmal").is_some());
    // gesperrter Knopf: Klick bewirkt nichts
    let count = h.state().doc.objects.len();
    h.get_by_label_contains("Box erzeugen").click();
    h.run_ok();
    assert_eq!(h.state().doc.objects.len(), count);
    assert!(h.state().box_gen.open);
}

/// Die Generatoren liegen im Untermenü Anordnen → Generatoren.
fn generators_menu_flow(render: bool) {
    let mut h = harness(render);
    h.get_by_label("Anordnen").click();
    h.run_ok();
    h.get_by_label_contains("Generatoren").hover();
    h.run_ok();
    if render {
        save(&mut h, "menue_generatoren");
    }
    h.get_by_label("Box-Generator…").click();
    h.run_ok();
    assert!(h.state().box_gen.open);
}

#[test]
fn generators_menu_opens_box_generator() {
    generators_menu_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_generators_menu() {
    generators_menu_flow(true);
}

#[test]
fn box_generator_lays_out_panels() {
    box_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_box_generator() {
    box_flow(true);
}

/// Biegescharnier: Rechteck als Bereich auswählen, Schnitte erzeugen; die Bereichsform verschwindet.
fn hinge_flow(render: bool) {
    let mut h = harness(render);
    let region = add(&mut h, VectorPath::rect(40.0, 40.0, 80.0, 40.0, 0.0));
    h.state_mut().hinge.open = true;
    h.state_mut().hinge.spec = vl_core::hinge::HingeSpec { cut: 12.0, bridge: 3.0, spacing: 2.0, angle: 0.0 };
    h.run_ok();
    if render {
        save(&mut h, "biegescharnier");
    }
    h.get_by_label_contains("Scharnier erzeugen").click();
    h.run_ok();
    let app = h.state();
    assert!(!app.hinge.open);
    assert!(app.doc.get(region).is_none(), "Bereichsform entfernt");
    let id = app.selection[0];
    let o = app.doc.get(id).unwrap();
    let Shape::Path(p) = &o.shape else { panic!() };
    assert_eq!(
        p.subpaths.len(),
        app.hinge
            .spec
            .lines(&VectorPath::rect(40.0, 40.0, 80.0, 40.0, 0.0).flatten(&vl_core::Affine::IDENTITY, 0.02))
            .len()
    );
    let b = o.bounds();
    assert!(b.min.x > 40.0 && b.max.x < 120.0 && b.min.y >= 40.0 - 1e-9 && b.max.y <= 80.0 + 1e-9, "{b:?}");
    if render {
        h.state_mut().view.request_fit(Some(vl_core::Rect::new(Point::new(35.0, 35.0), Point::new(125.0, 85.0))));
        h.run_ok();
        save(&mut h, "biegescharnier_ergebnis");
    }
}

#[test]
fn hinge_fills_selected_shape() {
    hinge_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_hinge() {
    hinge_flow(true);
}

/// Schablonenstege: Text „OB“ bekommt Stege, wird zu Kurven, kein Innenteil bleibt eingeschlossen.
#[test]
fn stencil_bridges_for_text() {
    let mut h = harness(false);
    let db = vl_io::system_fonts();
    // eine Umrissschrift (die eingebauten Einlinienschriften haben keine Löcher)
    let Some(family) = vl_io::font_families(&db).into_iter().find(|f| f == "Arial" || f == "DejaVu Sans") else {
        return;
    };
    let spec = vl_core::TextSpec { text: "OB".into(), family, height_mm: 30.0, ..Default::default() };
    let Ok(path) = vl_io::text_to_path(&db, &spec) else { return };
    let id = add(&mut h, path);
    h.state_mut().doc.get_mut(id).unwrap().text = Some(Box::new(vl_core::TextData { spec, ..Default::default() }));
    h.state_mut().shape_dialogs.stencil_open = true;
    h.run_ok();
    h.get_by_label("Anwenden").click();
    h.run_ok();
    let o = h.state().doc.get(id).unwrap();
    assert!(o.text.is_none(), "zu Kurven");
    let Shape::Path(p) = &o.shape else { panic!() };
    let polys = p.flatten(&o.transform, 0.01);
    // keine Kontur liegt mehr in einer ungeraden Anzahl anderer (also kein Loch, kein eingeschlossenes Innenteil)
    for (i, q) in polys.iter().enumerate() {
        let inside = polys
            .iter()
            .enumerate()
            .filter(|(j, r)| *j != i && vl_core::path::point_in_polylines(q.points[0], std::slice::from_ref(*r)))
            .count();
        assert_eq!(inside % 2, 0, "Kontur {i} ist noch ein Loch");
    }
    assert!(h.state_mut().history.can_undo());
}

/// Handbuch: F1 öffnet es, die Suche zeigt nur passende Kapitel, ein Klick im Inhaltsverzeichnis springt hin.
fn manual_flow(render: bool) {
    let mut h = harness(render);
    h.key_press(egui::Key::F1);
    h.run_ok();
    assert!(h.state().manual.open);
    if render {
        save(&mut h, "handbuch");
    }
    // erster Treffer: Eintrag im Inhaltsverzeichnis (die Überschrift im Text heißt genauso)
    h.get_all_by_label("GRBL-Parameter").next().unwrap().click();
    h.run_ok();
    assert!(h.state().manual.jump.is_none(), "Sprung ausgeführt");
    h.state_mut().manual.query = "Zahnräder".into();
    h.run_ok();
    // Kapitel mit „Zahnräder“: Anordnen/Generatoren; „Tastenkürzel“ enthält das Wort nicht
    assert!(h.query_all_by_label("Generatoren").count() > 0);
    assert_eq!(h.query_all_by_label("Tastenkürzel").count(), 0);
    if render {
        save(&mut h, "handbuch_suche");
    }
    h.key_press(egui::Key::F1);
    h.run_ok();
    assert!(!h.state().manual.open);
}

#[test]
fn manual_opens_searches_and_closes() {
    manual_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_manual() {
    manual_flow(true);
}

/// Lässt die Oberfläche laufen, bis `done` zutrifft (höchstens etwa 5 s), z. B. auf Antworten des Simulators.
fn wait_until(h: &mut Harness<'static, VectorLumeApp>, done: impl Fn(&VectorLumeApp) -> bool) {
    for _ in 0..250 {
        h.run_ok();
        if done(h.state()) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("Zeitüberschreitung");
}

/// Netzwerkkamera: Adresse eintragen, „Aufnehmen“ holt das Bild per HTTP (hier von einem Testserver).
#[test]
fn network_camera_snapshot() {
    use std::io::{Read, Write};
    let mut h = harness(false);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}/capture", listener.local_addr().unwrap().port());
    let server = std::thread::spawn(move || {
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(8, 6, image::Rgb([10, 200, 10]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let (mut c, _) = listener.accept().unwrap();
        let mut req = [0u8; 2048];
        let n = c.read(&mut req).unwrap();
        c.write_all(b"HTTP/1.0 200 OK\r\nContent-Type: image/png\r\n\r\n").unwrap();
        c.write_all(png.get_ref()).unwrap();
        String::from_utf8_lossy(&req[..n]).to_string()
    });
    {
        let app = h.state_mut();
        app.settings.camera.network = true;
        app.settings.camera.url = url;
        app.camera.open = true;
    }
    h.run_ok();
    assert!(h.query_all_by_label("Adresse").count() > 0, "Adressfeld der Netzwerkkamera");
    h.get_by_label(&format!("{} Aufnehmen", egui_phosphor::regular::CAMERA)).click();
    wait_until(&mut h, |app| app.camera.frame_size() == Some((8, 6)));
    assert!(server.join().unwrap().starts_with("GET /capture HTTP/1.0"));
    let app = h.state_mut();
    app.settings.camera.network = false;
    app.settings.camera.url.clear();
}

/// GRBL-Parameter mit dem eingebauten Simulator: Werte lesen, einen ändern, schreiben, neu lesen.
fn grbl_params_flow(render: bool) {
    let mut h = harness(render);
    h.state_mut().selected_port = vl_device::SIMULATOR_PORT.into();
    h.state_mut().connect();
    wait_until(&mut h, |app| app.machine.connected && app.machine.settings.contains_key(&130));
    h.state_mut().open_grbl_params();
    wait_until(&mut h, |app| app.grbl.edited.contains_key(&130));
    assert!(h.state().grbl.touched.is_empty());
    {
        let g = &mut h.state_mut().grbl;
        g.edited.insert(130, "321.5".into());
        g.touched.insert(130);
    }
    h.run_ok();
    if render {
        save(&mut h, "grbl_parameter");
    }
    h.get_by_label("Schreiben (1)").click();
    h.run_ok();
    wait_until(&mut h, |app| app.machine.setting_f64(130) == Some(321.5));
    // geschrieben und neu gelesen: nichts mehr zu schreiben
    assert!(vl_device::grbl_settings::changes(&h.state().machine.settings, &h.state().grbl.edited).is_empty());
    h.get_by_label("Schreiben (0)");
    h.state_mut().disconnect();
    wait_until(&mut h, |app| !app.machine.connected);
}

/// „Rahmen wiederholen“ mit dem Simulator: mehrere Durchgänge, Ausschalten beendet nach der laufenden Runde.
#[test]
fn frame_loop_repeats_until_switched_off() {
    let mut h = harness(false);
    add(&mut h, VectorPath::rect(1.0, 1.0, 2.0, 2.0, 0.0));
    {
        let app = h.state_mut();
        app.settings.profile.frame_speed = 6000.0;
        app.selected_port = vl_device::SIMULATOR_PORT.into();
        app.connect();
    }
    wait_until(&mut h, |app| app.machine.connected && app.machine.status.is_some());
    let toggle = egui_phosphor::regular::REPEAT;
    assert_eq!(h.query_all_by_label(toggle).count(), 1);
    h.get_by_label(toggle).click();
    h.run_ok();
    assert!(h.state().machine.frame_loop);
    h.state_mut().frame_job(false);
    wait_until(&mut h, |app| app.machine.frame_passes >= 3);
    assert!(h.state().machine.job_running && h.state().machine.frame_repeat.is_some());
    h.get_by_label(toggle).click();
    wait_until(&mut h, |app| !app.machine.job_running);
    let app = h.state();
    assert!(!app.machine.frame_loop && app.machine.frame_repeat.is_none());
    h.state_mut().disconnect();
    wait_until(&mut h, |app| !app.machine.connected);
}

/// Einrichtungsassistent beim ersten Start: Steuerung, Verbindung zum Simulator (Arbeitsfläche aus `$$`),
/// Nullpunkt, Name. Eigener Einstellungsordner, damit das neue Gerät keine anderen Tests beeinflusst.
fn setup_wizard_flow(render: bool) {
    use egui_phosphor::regular as icon;
    let dir = std::env::temp_dir().join(format!("vectorlume-ui-setup-{}-{render}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::settings::TEST_CONFIG_DIR.with(|d| *d.borrow_mut() = Some(dir.clone()));
    let mut h = harness(render);
    assert!(h.state().settings.first_run, "ohne Einstellungsdatei");
    h.state_mut().open_setup_wizard(true);
    h.run_ok();
    if render {
        save(&mut h, "einrichtung");
    }
    h.get_by_label("Weiter").click();
    h.run_ok();
    {
        let app = h.state_mut();
        assert_eq!(app.setup.step, 1);
        app.setup.profile.bed_width = 123.0;
        app.selected_port = vl_device::SIMULATOR_PORT.into();
    }
    h.run_ok();
    // derselbe Knopf steht auch in der Seitenleiste: den im Fenster (Mitte) nehmen
    let label = format!("{} Verbinden", icon::PLUG);
    h.query_all_by_label(&label).find(|n| n.rect().center().x < 1150.0).expect("Verbinden im Assistenten").click();
    // aus $130/$131 gelesen (die Werte treffen zeilenweise ein)
    wait_until(&mut h, |app| app.setup.profile.bed_width == 400.0 && app.setup.read.len() == 2);
    h.run_ok();
    if render {
        save(&mut h, "einrichtung_verbindung");
    }
    h.get_by_label("Weiter").click();
    h.run_ok();
    h.get_by_label("hinten links").click();
    h.run_ok();
    h.get_by_label("Weiter").click();
    h.run_ok();
    h.state_mut().setup.profile.name = "Testlaser".into();
    h.run_ok();
    h.get_by_label(&format!("{} Fertig", icon::CHECK)).click();
    h.run_ok();
    let app = h.state();
    assert!(!app.setup.open);
    assert_eq!(app.settings.devices.len(), 1, "erstes Gerät ersetzt");
    assert_eq!(app.settings.profile.name, "Testlaser");
    assert_eq!(app.settings.profile.origin, vl_cam::OriginCorner::RearLeft);
    assert_eq!((app.settings.profile.bed_width, app.settings.profile.s_max), (400.0, 1000.0));
    assert!(app.machine.connected, "Verbindung bleibt bestehen");
    h.state_mut().disconnect();
    wait_until(&mut h, |app| !app.machine.connected);
    // gespeichert: beim nächsten Start kein Assistent mehr
    let again = crate::settings::AppSettings::load();
    assert!(!again.first_run && again.profile.name == "Testlaser");
    crate::settings::TEST_CONFIG_DIR.with(|d| *d.borrow_mut() = None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn setup_wizard_first_run() {
    setup_wizard_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_setup_wizard() {
    setup_wizard_flow(true);
}

#[test]
fn grbl_params_written_to_simulator() {
    grbl_params_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_grbl_params() {
    grbl_params_flow(true);
}

/// Automatisch gruppieren über das Menü; „An Arbeitsfläche schieben“ legt die Auswahl in die Ecke unten rechts.
fn auto_group_flow(render: bool) {
    let mut h = harness(render);
    let part = add(&mut h, VectorPath::rect(20.0, 20.0, 40.0, 30.0, 0.0));
    let hole = add(&mut h, VectorPath::rect(30.0, 30.0, 10.0, 10.0, 0.0));
    let other = add(&mut h, VectorPath::rect(100.0, 20.0, 10.0, 10.0, 0.0));
    // ohne Auswahl: alle Objekte
    h.state_mut().selection.clear();
    h.get_by_label("Anordnen").click();
    h.run_ok();
    h.get_by_label("Automatisch gruppieren").click();
    h.run_ok();
    {
        let app = h.state();
        let g = |id| app.doc.get(id).unwrap().group;
        assert!(g(part).is_some() && g(part) == g(hole) && g(other).is_none());
    }
    h.state_mut().selection = vec![part, hole];
    h.get_by_label("Anordnen").click();
    h.run_ok();
    h.get_by_label_contains("An Arbeitsfläche schieben").hover();
    h.run_ok();
    if render {
        save(&mut h, "an_arbeitsflaeche");
    }
    h.get_by_label(egui_phosphor::regular::ARROW_DOWN_RIGHT).click();
    h.run_ok();
    let app = h.state();
    let bed = app.settings.profile.bed_rect();
    let b = app.doc.bounds_of(&[part, hole]);
    assert!(b.max.distance(bed.max) < 1e-9, "{b:?} {bed:?}");
    assert!((b.width() - 40.0).abs() < 1e-9);
}

#[test]
fn auto_group_and_move_to_bed_corner() {
    auto_group_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_auto_group() {
    auto_group_flow(true);
}

/// Fehlerbericht aus der letzten Sitzung: Hinweis erscheint, sobald kein anderer offen ist, und lässt sich schließen.
fn crash_notice_flow(render: bool) {
    let mut h = harness(render);
    h.state_mut().modal = Some(crate::app::Modal::Message { title: "Hinweis".into(), text: "Erst dieser".into() });
    h.state_mut().pending_crash = Some((std::env::temp_dir().join("crash-letzter.log"), false));
    h.run_ok();
    assert!(matches!(h.state().modal, Some(crate::app::Modal::Message { .. })));
    h.get_by_label("OK").click();
    h.run_ok();
    assert!(matches!(h.state().modal, Some(crate::app::Modal::Crash { running: false, .. })));
    if render {
        save(&mut h, "fehlerbericht");
    }
    h.get_by_label_contains("Bericht öffnen");
    h.get_by_label("OK").click();
    h.run_ok();
    assert!(h.state().modal.is_none() && h.state().pending_crash.is_none());
}

#[test]
fn crash_notice_after_other_modal() {
    crash_notice_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_crash_notice() {
    crash_notice_flow(true);
}

/// Zahnräder: Paar erzeugen; die Räder liegen nebeneinander ohne Berührung und bilden eine Gruppe.
fn gears_flow(render: bool) {
    let mut h = harness(render);
    h.state_mut().gear.open = true;
    h.state_mut().gear.spec.teeth = 12;
    h.state_mut().gear.teeth2 = 24;
    h.run_ok();
    if render {
        save(&mut h, "zahnraeder");
    }
    h.get_by_label_contains("Zahnräder erzeugen").click();
    h.run_ok();
    let app = h.state();
    assert!(!app.gear.open);
    assert_eq!(app.selection.len(), 2);
    let (a, b) = (app.doc.get(app.selection[0]).unwrap(), app.doc.get(app.selection[1]).unwrap());
    assert!(a.group.is_some() && a.group == b.group);
    // Modul 2: Kopfkreis-Ø 28 bzw. 52 mm (gedreht etwas schmaler), mindestens 4 mm Abstand
    let (ba, bb) = (a.bounds(), b.bounds());
    let near = |w: f64, d: f64| w <= d + 1e-6 && w > d - 1.0;
    assert!(near(ba.width(), 28.0) && near(bb.width(), 52.0), "{ba:?} {bb:?}");
    assert!(bb.min.x - ba.max.x > 3.9);
    let Shape::Path(p) = &a.shape else { panic!() };
    assert_eq!(p.subpaths.len(), 2, "Umriss und Bohrung");
}

#[test]
fn gears_inserted_side_by_side() {
    gears_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_gears() {
    gears_flow(true);
}

/// Schnittbreite ermitteln: Testkamm erzeugen, Messwert eingeben, Ergebnis auf die Ebene übernehmen.
fn kerf_test_flow(render: bool) {
    let mut h = harness(render);
    h.state_mut().open_kerf_test();
    h.run_ok();
    h.get_by_label_contains("Testkamm erzeugen").click();
    h.run_ok();
    {
        let app = h.state();
        assert_eq!(app.selection.len(), 1);
        let o = app.doc.get(app.selection[0]).unwrap();
        assert_eq!(o.layer, app.current_layer);
        let Shape::Path(p) = &o.shape else { panic!() };
        assert_eq!(p.subpaths.len(), 1 + 2 * 9);
        assert!((o.bounds().width() - 60.0).abs() < 1e-9);
    }
    // 10 Streifen à 6 mm, zusammen 58,5 mm → 0,15 mm
    h.state_mut().kerf.value = 58.5;
    h.run_ok();
    if render {
        save(&mut h, "schnittbreite");
    }
    h.get_by_label_contains("übernehmen").click();
    h.run_ok();
    let app = h.state();
    assert!((app.doc.layers[app.current_layer].settings.kerf - 0.15).abs() < 1e-9);
    assert!(app.kerf.open);
}

#[test]
fn kerf_test_applies_measured_kerf() {
    kerf_test_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_kerf_test() {
    kerf_test_flow(true);
}

/// Puzzle: übernimmt Lage und Größe der Auswahl, „Puzzle erzeugen“ legt Schnittlinien und Rahmen als Gruppe an.
fn puzzle_flow(render: bool) {
    let mut h = harness(render);
    let photo = add(&mut h, VectorPath::rect(30.0, 40.0, 120.0, 90.0, 0.0));
    h.state_mut().selection = vec![photo];
    h.state_mut().open_puzzle();
    h.state_mut().puzzle.spec.cols = 4;
    h.state_mut().puzzle.spec.rows = 3;
    h.run_ok();
    if render {
        save(&mut h, "puzzle");
    }
    h.get_by_label_contains("Neu würfeln").click();
    h.run_ok();
    assert_eq!(h.state().puzzle.spec.seed, 2);
    h.get_by_label_contains("Puzzle erzeugen").click();
    h.run_ok();
    let app = h.state();
    assert!(!app.puzzle.open);
    assert_eq!(app.selection.len(), 2);
    let cuts = app.doc.get(app.selection[0]).unwrap();
    let Shape::Path(p) = &cuts.shape else { panic!() };
    assert_eq!(p.subpaths.len(), 3 + 2);
    let frame = app.doc.get(app.selection[1]).unwrap().bounds();
    assert!((frame.min.x - 30.0).abs() < 1e-9 && (frame.max.y - 130.0).abs() < 1e-9, "{frame:?}");
    assert!(cuts.group.is_some() && cuts.group == app.doc.get(app.selection[1]).unwrap().group);
    if render {
        h.state_mut().doc.remove(&[photo]);
        h.state_mut().view.request_fit(Some(vl_core::Rect::new(Point::new(25.0, 35.0), Point::new(155.0, 135.0))));
        h.run_ok();
        save(&mut h, "puzzle_ergebnis");
    }
}

#[test]
fn puzzle_takes_selection_area() {
    puzzle_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_puzzle() {
    puzzle_flow(true);
}

/// Kamerabild übernehmen: dunkles Quadrat in der Bildmitte landet an der passenden Stelle der Arbeitsfläche,
/// auf einer Ebene ohne Ausgabe.
#[test]
fn camera_image_goes_into_project() {
    let mut h = harness(false);
    let app = h.state_mut();
    let (bw, bh) = (app.settings.profile.bed_width, app.settings.profile.bed_height);
    // Kamera sieht genau die Arbeitsfläche: Bildecken = Bettecken (Maschinenkoordinaten, Bild y nach unten)
    let corners = [([0.0, 0.0], [0.0, bh]), ([1.0, 0.0], [bw, bh]), ([1.0, 1.0], [bw, 0.0]), ([0.0, 1.0], [0.0, 0.0])];
    for (p, (img, bed)) in app.settings.camera.points.iter_mut().zip(corners) {
        p.image = Some(img);
        p.bed = bed;
    }
    let (w, hh) = (200usize, 200usize);
    let rgb = (0..w * hh)
        .flat_map(|i| {
            let (x, y) = (i % w, i / w);
            let dark = (80..120).contains(&x) && (80..120).contains(&y);
            [if dark { 0 } else { 250 }; 3]
        })
        .collect();
    app.camera.set_test_frame(w, hh, rgb);
    crate::camera::insert_image(app);
    let o = app.doc.objects.last().expect("Bild");
    let Shape::Image(img) = &o.shape else { panic!("kein Bild") };
    assert!(!app.doc.layers[o.layer].settings.output, "Ebene ohne Ausgabe");
    assert!((img.width_mm - bw).abs() < 1e-9 && (img.height_mm - bh).abs() < 1e-9);
    // Mitte dunkel, Rand hell (Bildmitte = Bettmitte)
    let (iw, ih) = (img.image.width as usize, img.image.height as usize);
    assert!(img.image.pixels[ih / 2 * iw + iw / 2] < 20);
    assert!(img.image.pixels[ih / 10 * iw + iw / 10] > 230);
    // nur der Bereich der Auswahl
    let id = app.doc.add_path("R", 0, VectorPath::rect(10.0, 10.0, 50.0, 20.0, 0.0));
    app.selection = vec![id];
    crate::camera::insert_image(app);
    let Shape::Image(img) = &app.doc.objects.last().unwrap().shape else { panic!() };
    assert!((img.width_mm - 50.0).abs() < 1e-9 && (img.height_mm - 20.0).abs() < 1e-9);
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

/// Bearbeitungsverlauf: Menü öffnet das Fenster, jeder Schritt ist beschriftet, Klicks springen zurück und vor.
fn history_window_flow(render: bool) {
    let mut h = harness(render);
    let (a, b) = {
        let app = h.state_mut();
        app.checkpoint();
        let a = app.doc.add_path("A", 0, VectorPath::rect(10.0, 10.0, 20.0, 20.0, 0.0));
        app.checkpoint();
        let b = app.doc.add_path("B", 0, VectorPath::rect(40.0, 10.0, 20.0, 20.0, 0.0));
        app.checkpoint();
        app.doc.transform_objects(&[b], &vl_core::Affine::translate(30.0, 0.0));
        app.checkpoint();
        app.doc.get_mut(a).unwrap().layer = 2;
        app.doc.get_mut(b).unwrap().layer = 2;
        app.mark_changed();
        (a, b)
    };
    h.get_by_label("Bearbeiten").click();
    h.run_ok();
    h.get_by_label("Bearbeitungsverlauf…").click();
    h.run_ok();
    assert!(h.state().history_window.open);
    for text in ["Ausgangszustand", "Hinzugefügt: A", "Hinzugefügt: B", "Verschoben: B", "Ebene gewechselt: 2"] {
        assert!(h.query_by_label_contains(text).is_some(), "{text} fehlt");
    }
    if render {
        save(&mut h, "bearbeitungsverlauf");
    }
    h.get_by_label_contains("Hinzugefügt: A").click();
    h.run_ok();
    {
        let app = h.state();
        assert_eq!(app.history.undo_len(), 1);
        assert_eq!(app.doc.objects.iter().map(|o| o.id).collect::<Vec<_>>(), [a]);
    }
    // spätere Schritte bleiben sichtbar und lassen sich wiederherstellen
    h.get_by_label_contains("Verschoben: B").click();
    h.run_ok();
    {
        let app = h.state();
        assert_eq!(app.history.undo_len(), 3);
        assert_eq!(app.doc.get(b).unwrap().layer, 0);
        assert!(app.doc.bounds().max.x > 89.0);
    }
    h.get_by_label_contains("Ebene gewechselt").click();
    h.run_ok();
    assert_eq!(h.state().doc.get(a).unwrap().layer, 2);
    assert!(!h.state().history.can_redo());
}

/// 3D-Modell: STL importieren öffnet den Dialog, „Tiefenbild einfügen“ legt ein Graustufenbild an.
fn model_flow(render: bool) {
    let mut h = harness(render);
    // Pyramide 40×20 Grundfläche, 10 hoch, als ASCII-STL
    let (a, b, c, d, top) = ([0, 0, 0], [40, 0, 0], [40, 20, 0], [0, 20, 0], [20, 10, 10]);
    let mut stl = String::from("solid pyramide\n");
    for t in [[a, b, top], [b, c, top], [c, d, top], [d, a, top], [a, c, b], [a, d, c]] {
        stl.push_str("facet normal 0 0 0\nouter loop\n");
        for v in t {
            stl.push_str(&format!("vertex {} {} {}\n", v[0], v[1], v[2]));
        }
        stl.push_str("endloop\nendfacet\n");
    }
    let path = std::env::temp_dir().join("vectorlume-ui-tests").join("pyramide.stl");
    std::fs::write(&path, stl + "endsolid\n").unwrap();
    h.state_mut().import_path(&path);
    h.run_ok();
    assert!(h.state().model.open);
    assert!(h.query_by_label_contains("6 Dreiecke").is_some());
    assert!(h.query_by_label_contains("400 × 200 Pixel").is_some(), "10 px/mm bei 40 mm Breite");
    if render {
        save(&mut h, "dialog_3d_modell");
    }
    h.get_by_label("Tiefenbild einfügen").click();
    h.run_ok();
    let app = h.state();
    assert!(!app.model.open);
    let o = app.doc.get(app.selection[0]).unwrap();
    let Shape::Image(img) = &o.shape else { panic!("kein Bild") };
    assert_eq!((img.width_mm, img.height_mm, img.image.width, img.image.height), (40.0, 20.0, 400, 200));
    assert_eq!(o.name, "pyramide");
    let layer = &app.doc.layers[o.layer];
    assert_eq!(layer.operation(layer.image_operation()).unwrap().dither, vl_core::DitherMode::Grayscale);
    assert!(app.settings.profile.bed_rect().contains_rect(&app.doc.bounds()));
}

/// Tangentialkreise: drei Kreise auswählen, Menü öffnet die Vorschläge, ein Klick auf eine Kontur übernimmt sie.
fn tangent_flow(render: bool) {
    let mut h = harness(render);
    let h3 = 40.0 * 3f64.sqrt() / 2.0;
    let centers = [Point::new(60.0, 60.0 + h3), Point::new(100.0, 60.0 + h3), Point::new(80.0, 60.0)];
    let ids: Vec<ObjectId> = {
        let app = h.state_mut();
        centers.iter().map(|c| app.doc.add_path("K", 0, VectorPath::ellipse(*c, 10.0, 10.0))).collect()
    };
    h.state_mut().selection = ids;
    h.run_ok();
    h.get_by_label("Anordnen").click();
    h.run_ok();
    h.get_by_label_contains("Generatoren").hover();
    h.run_ok();
    h.get_by_label("Tangentialkreise…").click();
    h.run_ok();
    let candidates = h.state().tangent.as_ref().expect("Vorschläge").candidates.clone();
    assert_eq!(candidates.len(), 8);
    // kleinster Vorschlag: im Schwerpunkt der drei Mittelpunkte
    let (c, r) = candidates[0];
    let centroid = Point::new(80.0, 60.0 + 2.0 * h3 / 3.0);
    assert!(c.distance(centroid) < 0.01 && (r - (centroid.distance(centers[0]) - 10.0)).abs() < 0.01);
    let target = c + Point::new(0.0, r);
    let mut pos = None;
    for _ in 0..50 {
        let now = h.state().view.tf.map(|t| t.to_screen(target));
        if now.is_some() && now == pos {
            break;
        }
        pos = now;
        if let Some(q) = pos {
            h.hover_at(q);
        }
        h.run_ok();
    }
    assert_eq!(h.state().tangent.as_ref().unwrap().hover, Some(0), "Zeiger auf der Kontur hebt sie hervor");
    if render {
        save(&mut h, "tangentialkreise");
    }
    let pos = pos.unwrap();
    h.drag_at(pos);
    h.drop_at(pos);
    h.run_ok();
    let app = h.state();
    assert!(app.tangent.is_none(), "ohne Umschalt endet die Auswahl");
    assert_eq!(app.doc.objects.len(), 4);
    let (nc, nr) = crate::tangent_tool::as_circle(app.doc.get(app.selection[0]).unwrap()).unwrap();
    assert!(nc.distance(c) < 0.01 && (nr - r).abs() < 0.01);
    assert!(app.history.can_undo());
}

#[test]
fn tangent_circles_pick_one() {
    tangent_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_tangent_circles() {
    tangent_flow(true);
}

#[test]
fn escape_ends_tangent_suggestions() {
    let mut h = harness(false);
    for x in [40.0, 70.0, 55.0] {
        let id =
            h.state_mut().doc.add_path("K", 0, VectorPath::ellipse(Point::new(x, 50.0 + (x - 55.0).abs()), 10.0, 10.0));
        h.state_mut().selection.push(id);
    }
    h.state_mut().start_tangent_circles();
    assert!(h.state().tangent.is_some());
    h.key_press(egui::Key::Escape);
    h.run_ok();
    assert!(h.state().tangent.is_none());
    assert_eq!(h.state().selection.len(), 3, "Esc beendet nur die Vorschläge");
}

/// Klickt auf die Arbeitsfläche an einen Dokumentpunkt, sobald die Ansicht stillsteht.
fn click_doc(h: &mut Harness<'static, VectorLumeApp>, target: Point) {
    let mut pos = None;
    for _ in 0..50 {
        let now = h.state().view.tf.map(|t| t.to_screen(target));
        if now.is_some() && now == pos {
            break;
        }
        pos = now;
        if let Some(q) = pos {
            h.hover_at(q);
        }
        h.run_ok();
    }
    let pos = pos.expect("Ansicht");
    h.drag_at(pos);
    h.drop_at(pos);
    h.run_ok();
}

fn cut_order_flow(render: bool) {
    let mut h = harness(render);
    let ids: Vec<ObjectId> = (0..3)
        .map(|k| h.state_mut().doc.add_path("R", 0, VectorPath::rect(30.0 + 40.0 * k as f64, 40.0, 25.0, 25.0, 0.0)))
        .collect();
    let center = |k: usize| Point::new(42.5 + 40.0 * k as f64, 52.5);
    h.state_mut().settings.job.optimize_order = true;
    h.run_ok();
    h.get_by_label("Anordnen").click();
    h.run_ok();
    h.get_by_label("Schnittreihenfolge per Klick…").click();
    h.run_ok();
    assert!(h.state().cut_order.is_some());
    click_doc(&mut h, center(2));
    click_doc(&mut h, center(0));
    assert_eq!(h.state().cut_order.as_ref().unwrap().picked, vec![ids[2], ids[0]]);
    // Rücktaste nimmt den letzten Klick zurück und löscht nichts
    h.key_press(egui::Key::Backspace);
    h.run_ok();
    assert_eq!(h.state().doc.objects.len(), 3);
    click_doc(&mut h, center(1));
    let n = crate::cut_order::numbers(&h.state().doc, &h.state().cut_order.as_ref().unwrap().picked);
    assert_eq!((n[&ids[2]], n[&ids[1]], n[&ids[0]]), (1, 2, 3));
    if render {
        save(&mut h, "schnittreihenfolge");
    }
    h.key_press(egui::Key::Enter);
    h.run_ok();
    let app = h.state();
    assert!(app.cut_order.is_none());
    assert_eq!(app.doc.objects.iter().map(|o| o.id).collect::<Vec<_>>(), vec![ids[2], ids[1], ids[0]]);
    assert!(!app.settings.job.optimize_order, "Optimierung aus, damit die Reihenfolge gilt");
    // gemeinsamer Einstellungsordner der Tests: Standard wiederherstellen
    let app = h.state_mut();
    app.settings.job.optimize_order = true;
    app.settings.save();
    app.undo();
    assert_eq!(app.doc.objects.iter().map(|o| o.id).collect::<Vec<_>>(), ids);
}

#[test]
fn cut_order_by_clicks() {
    cut_order_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_cut_order() {
    cut_order_flow(true);
}

/// Leiterplatte: Kupferlage und Bohrdatei landen deckungsgleich in der Arbeitsfläche.
#[test]
fn pcb_layers_stay_aligned() {
    let mut h = harness(false);
    let dir = std::env::temp_dir().join("vectorlume-ui-tests");
    // Zwei Pads Ø 2 bei (100|50) und (110|50) – weit außerhalb der Fläche, Y nach oben
    let gbr = "%FSLAX46Y46*%%MOMM*%%ADD10C,2*%D10*X100000000Y-50000000D03*X110000000Y-50000000D03*M02*";
    let drl = "M48\nMETRIC\nT1C0.8\n%\nT1\nX100.0Y-50.0\nX110.0Y-50.0\nM30\n";
    std::fs::write(dir.join("platine.gtl"), gbr).unwrap();
    std::fs::write(dir.join("platine.drl"), drl).unwrap();
    h.state_mut().import_path(&dir.join("platine.drl"));
    h.state_mut().import_path(&dir.join("platine.gtl"));
    h.run_ok();
    let app = h.state();
    assert_eq!(app.doc.objects.len(), 2);
    let centers: Vec<Point> = app.doc.objects.iter().map(|o| app.doc.bounds_of(&[o.id]).center()).collect();
    assert!(centers[0].distance(centers[1]) < 1e-6, "{centers:?}");
    let bed = app.settings.profile.bed_rect();
    assert!(bed.contains_rect(&app.doc.bounds()));
    // Bohrdatei zuerst: deren linker oberer Rand liegt 10 mm vom Rand der Fläche
    let drill = app.doc.bounds_of(&[app.doc.objects[0].id]);
    assert!((drill.min.x - bed.min.x - 10.0).abs() < 1e-6 && (drill.min.y - bed.min.y - 10.0).abs() < 1e-6);
}

#[test]
fn model_import_inserts_depth_map() {
    model_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_model_dialog() {
    model_flow(true);
}

#[test]
fn history_window_lists_steps_and_jumps() {
    history_window_flow(false);
}

/// Belegt nur Befehle ohne Standardkürzel mit H: andere Tests laden dieselben Einstellungen.
fn hotkey_flow(render: bool) {
    use crate::hotkeys::Action;
    let mut h = harness(render);
    let id = add(&mut h, VectorPath::rect(20.0, 20.0, 60.0, 20.0, 0.0));
    h.get_by_label("Bearbeiten").click();
    h.run_ok();
    h.get_by_label("Tastenkürzel…").click();
    h.run_ok();
    assert!(h.state().hotkey_editor.open);
    if render {
        save(&mut h, "tastenkuerzel");
    }
    // Taste aufnehmen: H dreht danach die Auswahl
    h.state_mut().hotkey_editor.filter = "im Uhrzeigersinn".into();
    h.run_ok();
    assert_eq!(h.query_all_by_label("Ändern").count(), 1);
    h.get_by_label("Ändern").click();
    h.run_ok();
    assert!(h.query_by_label("Taste drücken …").is_some());
    h.key_press(egui::Key::H);
    h.run_ok();
    assert_eq!(h.state().settings.hotkeys.get("rotate_cw"), Some(&vec!["H".to_string()]));
    h.key_press(egui::Key::H);
    h.run_ok();
    let b = h.state().doc.bounds_of(&[id]);
    assert!((b.width() - 20.0).abs() < 1e-6 && (b.height() - 60.0).abs() < 1e-6, "{b:?}");
    // Fest belegte Taste wird abgelehnt
    h.get_by_label("Ändern").click();
    h.run_ok();
    h.key_press(egui::Key::ArrowUp);
    h.run_ok();
    assert!(h.query_by_label_contains("ist fest belegt").is_some());
    assert_eq!(h.state().doc.bounds_of(&[id]), b, "Pfeiltaste verschiebt während der Aufnahme nicht");
    // Gleiche Taste für einen anderen Befehl: beim bisherigen entfernt
    h.state_mut().hotkey_editor.filter = "gegen den Uhrzeigersinn".into();
    h.run_ok();
    h.get_by_label("Ändern").click();
    h.run_ok();
    h.key_press(egui::Key::H);
    h.run_ok();
    assert!(h.query_by_label_contains("zugewiesen und wurde dort entfernt").is_some());
    let app = h.state();
    assert!(app.keys.of(Action::RotateCw).is_empty());
    assert_eq!(app.keys.text(Action::RotateCcw), "H");
    assert!(!app.settings.hotkeys.contains_key("rotate_cw"), "leer = wieder Standard (ohne Taste)");
    h.state_mut().hotkey_editor.filter.clear();
    h.run_ok();
    h.get_by_label("Alle auf Standard zurücksetzen").click();
    h.run_ok();
    let app = h.state();
    assert!(app.settings.hotkeys.is_empty());
    assert!(app.keys.of(Action::RotateCcw).is_empty());
}

#[test]
fn hotkeys_record_resolve_conflicts_and_reset() {
    hotkey_flow(false);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_hotkeys() {
    hotkey_flow(true);
}

#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_history_window() {
    history_window_flow(true);
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

/// Bildmodus Skizze: synthetisches „Foto“ (beleuchtete Kugel, Rahmen, Streifen) in der Vorschau.
#[test]
#[ignore = "rendert Bilder zur Sichtprüfung"]
fn render_sketch() {
    let mut h = harness(true);
    let (w, hgt) = (400u32, 300u32);
    let pixels = (0..w * hgt)
        .map(|i| {
            let (x, y) = ((i % w) as f64, (i / w) as f64);
            let (dx, dy) = ((x - 150.0) / 90.0, (y - 150.0) / 90.0);
            let r2 = dx * dx + dy * dy;
            let v = if r2 < 1.0 {
                // Kugel, Licht von oben links
                let z = (1.0 - r2).sqrt();
                0.15 + 0.8 * ((-dx - dy + z) / 3f64.sqrt()).max(0.0)
            } else if x > 280.0 && ((y / 20.0) as i32) % 2 == 0 {
                0.25
            } else if !(15.0..=385.0).contains(&x) || !(15.0..=285.0).contains(&y) {
                0.4
            } else {
                0.85
            };
            (v.clamp(0.0, 1.0) * 255.0) as u8
        })
        .collect();
    let image = vl_core::ImageObject {
        image: std::sync::Arc::new(vl_core::RasterImage::new(w, hgt, pixels)),
        width_mm: 80.0,
        height_mm: 60.0,
        adjust: Default::default(),
        clip: vec![],
    };
    let id = h.state_mut().doc.add_image("Foto", 0, image, Point::new(40.0, 40.0));
    let layer = h.state().doc.get(id).unwrap().layer;
    let s = &mut h.state_mut().doc.layers[layer].settings;
    s.mode = vl_core::LayerMode::Image;
    s.dither = vl_core::DitherMode::Sketch;
    s.interval = 0.1;
    h.state_mut().selection = vec![id];
    h.state_mut().view.request_fit(Some(vl_core::Rect::new(Point::new(35.0, 35.0), Point::new(125.0, 105.0))));
    h.run_ok();
    h.state_mut().preview.active = true;
    for _ in 0..100 {
        h.step();
        if h.state().preview.job.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    h.run_ok();
    assert!(h.state().preview.job.is_some(), "{:?}", h.state().preview.error);
    save(&mut h, "bildmodus_skizze");
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

/// Beispielinhalt für den Zufallstest: offene und geschlossene Formen, Gruppe, Bild, auf mehreren Ebenen.
fn monkey_scene(app: &mut VectorLumeApp) {
    let pixels = (0..64 * 48).map(|i| ((i * 7) % 256) as u8).collect();
    let image = vl_core::ImageObject {
        image: std::sync::Arc::new(vl_core::RasterImage::new(64, 48, pixels)),
        width_mm: 32.0,
        height_mm: 24.0,
        adjust: Default::default(),
        clip: Vec::new(),
    };
    app.doc.add_path("Rechteck", 0, VectorPath::rect(20.0, 20.0, 60.0, 40.0, 4.0));
    app.doc.add_path("Kreis", 1, VectorPath::ellipse(Point::new(120.0, 50.0), 20.0, 15.0));
    app.doc.add_path("Linie", 0, VectorPath::line(Point::new(20.0, 90.0), Point::new(140.0, 120.0)));
    let a = app.doc.add_path("Gruppe A", 2, VectorPath::rect(160.0, 20.0, 30.0, 30.0, 0.0));
    let b = app.doc.add_path("Gruppe B", 2, VectorPath::ellipse(Point::new(175.0, 35.0), 8.0, 8.0));
    app.doc.group(&[a, b]);
    app.doc.add_image("Bild", 3, image, Point::new(60.0, 140.0));
    app.doc.layers[1].settings.mode = vl_core::LayerMode::Fill;
    app.select_all();
}

/// Zufallsbedienung („Affentest“): klickt nach festem Muster auf Knöpfe, Menüeinträge, Kästchen und Reiter,
/// drückt Tastenkürzel und zieht auf der Arbeitsfläche – mit Beispielobjekten, Auswahl und verbundenem Simulator.
/// Kein Schritt darf die App abstürzen lassen. Dateidialoge, Kamera und echte Anschlüsse sind im Test gesperrt
/// (`file_dialog`, `camera`, `connect_with`), daher kann nichts außerhalb des Tests passieren.
fn monkey(seed: u64, steps: usize) {
    use egui::{Key, Modifiers, PointerButton, Pos2};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let dir = std::env::temp_dir().join(format!("vectorlume-ui-affe-{seed}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::settings::TEST_CONFIG_DIR.with(|d| *d.borrow_mut() = Some(dir.clone()));
    let mut h = harness(false);
    {
        let app = h.state_mut();
        monkey_scene(app);
        app.selected_port = vl_device::SIMULATOR_PORT.into();
        app.connect();
    }
    h.run_ok();
    // „Beenden“ würde nur das Fenster schließen; alles andere darf der Affe anklicken
    const SKIP: [&str; 1] = ["Beenden"];
    const KEYS: [(Modifiers, Key); 10] = [
        (Modifiers::NONE, Key::Escape),
        (Modifiers::NONE, Key::Escape),
        (Modifiers::NONE, Key::Delete),
        (Modifiers::NONE, Key::Enter),
        (Modifiers::COMMAND, Key::Z),
        (Modifiers::COMMAND, Key::Y),
        (Modifiers::COMMAND, Key::A),
        (Modifiers::COMMAND, Key::C),
        (Modifiers::COMMAND, Key::V),
        (Modifiers::COMMAND, Key::G),
    ];
    let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = move |n: usize| {
        rng ^= rng >> 12;
        rng ^= rng << 25;
        rng ^= rng >> 27;
        (rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as usize % n.max(1)
    };
    let screen = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 1000.0));
    let mut log: Vec<String> = Vec::new();
    for step in 0..steps {
        if step % 60 == 0 && h.state().doc.objects.len() < 3 {
            monkey_scene(h.state_mut());
            log.push("Beispielinhalt neu".into());
        }
        let action = next(100);
        if action < 70 {
            let targets: Vec<(String, egui::Rect)> = h
                .query_all_by(|n| {
                    use egui::accesskit::Role;
                    matches!(
                        n.role(),
                        Role::Button
                            | Role::CheckBox
                            | Role::RadioButton
                            | Role::MenuItem
                            | Role::MenuItemCheckBox
                            | Role::MenuItemRadio
                            | Role::Tab
                            | Role::ComboBox
                    ) && !n.is_disabled()
                })
                .map(|n| (egui_kittest::kittest::NodeT::accesskit_node(&n).label().unwrap_or_default(), n.rect()))
                .filter(|(label, rect)| screen.contains(rect.center()) && !SKIP.iter().any(|s| label.contains(s)))
                .collect();
            if targets.is_empty() {
                continue;
            }
            let (label, rect) = &targets[next(targets.len())];
            log.push(format!("{step}: Klick „{label}“"));
            let pos = rect.center();
            h.hover_at(pos);
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                });
            }
        } else if action < 80 {
            let (m, k) = KEYS[next(KEYS.len())];
            log.push(format!("{step}: Taste {m:?} {k:?}"));
            h.key_press_modifiers(m, k);
        } else {
            // Arbeitsfläche: Ziehen (Werkzeuge, Auswahlrahmen, Verschieben) oder Rechtsklick
            let p = Pos2::new(300.0 + next(800) as f32, 120.0 + next(700) as f32);
            if action < 95 {
                let q = p + egui::vec2(next(200) as f32 - 100.0, next(200) as f32 - 100.0);
                log.push(format!("{step}: Ziehen {p:?} → {q:?}"));
                h.drag_at(p);
                h.hover_at(p + (q - p) * 0.5);
                h.drop_at(q);
            } else {
                log.push(format!("{step}: Rechtsklick {p:?}"));
                h.hover_at(p);
                for pressed in [true, false] {
                    h.event(egui::Event::PointerButton {
                        pos: p,
                        button: PointerButton::Secondary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    });
                }
            }
        }
        if catch_unwind(AssertUnwindSafe(|| h.run_steps(3))).is_err() {
            let tail = log.iter().rev().take(15).rev().cloned().collect::<Vec<_>>().join("\n");
            panic!("Absturz bei Zufallsbedienung (Startwert {seed}), letzte Schritte:\n{tail}");
        }
    }
    if std::env::var_os("VL_MONKEY_LOG").is_some() {
        println!(
            "{}",
            log.join(
                "
"
            )
        );
    }
    h.state_mut().disconnect();
    crate::settings::TEST_CONFIG_DIR.with(|d| *d.borrow_mut() = None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn monkey_clicks_1() {
    monkey(1, 1500);
}

#[test]
fn monkey_clicks_2() {
    monkey(2, 1500);
}

#[test]
fn monkey_clicks_3() {
    monkey(3, 1500);
}

/// Längere Suche von Hand: `VL_MONKEY_SEEDS=10..40 cargo test -p vectorlume monkey_many -- --ignored`.
#[test]
#[ignore = "lange Zufallssuche"]
fn monkey_many() {
    let range = std::env::var("VL_MONKEY_SEEDS").unwrap_or_else(|_| "10..20".into());
    let (a, b) = range.split_once("..").expect("Bereich wie 10..20");
    for seed in a.parse::<u64>().unwrap()..b.parse::<u64>().unwrap() {
        monkey(seed, 2000);
    }
}

/// Zeitmessung: ein Bild der Oberfläche mit vielen Formen (nur von Hand, mit `--ignored --nocapture`).
#[test]
#[ignore = "Zeitmessung"]
fn perf_many_shapes() {
    let mut h = harness(false);
    {
        let app = h.state_mut();
        for i in 0..5000 {
            let (x, y) = ((i % 100) as f64 * 4.0, (i / 100) as f64 * 8.0);
            app.doc.add_path("K", i % 3, VectorPath::ellipse(Point::new(x + 2.0, y + 2.0), 1.5, 1.5));
        }
        app.selection.clear();
    }
    h.run_ok();
    let t = std::time::Instant::now();
    for _ in 0..20 {
        h.state_mut().view.zoom *= 1.001;
        h.step();
    }
    println!("5000 Kreise: {:.1} ms je Bild", t.elapsed().as_secs_f64() * 1000.0 / 20.0);
    h.state_mut().select_all();
    h.run_ok();
    let t = std::time::Instant::now();
    for _ in 0..20 {
        h.step();
    }
    println!("alle ausgewählt: {:.1} ms je Bild", t.elapsed().as_secs_f64() * 1000.0 / 20.0);
}

fn tap(h: &mut Harness<'static, VectorLumeApp>, pos: egui::Pos2) {
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(2);
    }
}

/// Echter Ablauf am Touchpad: „Job starten“ anklicken, der Zeiger bleibt liegen, der Job läuft durch. Danach steht der
/// Knopf wieder unter dem Zeiger – ein Tipp-Klick beim Weiterbewegen darf den Job nicht noch einmal starten. Erst
/// nach Wegbewegen und Zurückkehren startet ein Klick wieder.
#[test]
fn start_button_is_guarded_after_a_job() {
    let dir = std::env::temp_dir().join(format!("vectorlume-ui-startsperre-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::settings::TEST_CONFIG_DIR.with(|d| *d.borrow_mut() = Some(dir.clone()));
    let mut h = harness_sized(false, egui::vec2(1600.0, 2400.0));
    {
        let app = h.state_mut();
        app.settings.safety_acknowledged = true;
        app.settings.profile.return_to_origin = false;
        // kleines Quadrat nahe am Nullpunkt (vorne links): kurzer Job im Simulator
        let y = app.settings.profile.bed_height - 3.0;
        app.doc.add_path("R", 0, VectorPath::rect(1.0, y, 1.0, 1.0, 0.0));
        app.selected_port = vl_device::SIMULATOR_PORT.into();
        app.connect();
    }
    wait_until(&mut h, |a| a.machine.connected && a.machine.status.is_some());
    let pos = h.get_by_label_contains("Job starten").rect().center();
    h.hover_at(pos);
    h.run_steps(2);
    tap(&mut h, pos);
    wait_until(&mut h, |a| a.machine.job_running);
    for _ in 0..1500 {
        h.run_ok();
        if !h.state().machine.job_running {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!h.state().machine.job_running, "Job nicht fertig geworden");
    // Zeiger hat sich nicht bewegt; Wartezeit überspringen
    for _ in 0..3 {
        h.run_steps(2);
    }
    if let Some(g) = &mut h.state_mut().machine.start_guard {
        g.since -= crate::app::StartGuard::DELAY;
    }
    h.run_steps(3);
    assert!(h.state().machine.start_guard.is_some(), "Sperre ohne Wegbewegen aufgehoben");
    tap(&mut h, pos);
    for _ in 0..20 {
        h.run_ok();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!h.state().machine.job_running, "Tipp-Klick hat den Job erneut gestartet");
    assert!(h.state().toasts.iter().any(|t| t.text.contains("gerade erst fertig")));
    // wegbewegen und zurück: jetzt startet ein Klick wieder
    h.hover_at(pos + egui::vec2(0.0, 200.0));
    h.run_steps(2);
    assert!(h.state().machine.start_guard.is_none());
    h.hover_at(pos);
    h.run_steps(2);
    tap(&mut h, pos);
    wait_until(&mut h, |a| a.machine.job_running);
    h.state_mut().disconnect();
    crate::settings::TEST_CONFIG_DIR.with(|d| *d.borrow_mut() = None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// „Laser per Klick verfahren“: ein Klick auf die Arbeitsfläche fährt den Laser dorthin (Simulator), Esc beendet.
#[test]
fn click_moves_laser_to_position() {
    let dir = std::env::temp_dir().join(format!("vectorlume-ui-klickfahrt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::settings::TEST_CONFIG_DIR.with(|d| *d.borrow_mut() = Some(dir.clone()));
    let mut h = harness(false);
    {
        let app = h.state_mut();
        app.settings.profile.jog_speed = 30000.0;
        app.selected_port = vl_device::SIMULATOR_PORT.into();
        app.connect();
    }
    wait_until(&mut h, |a| a.machine.connected && a.machine.status.is_some());
    h.state_mut().right_tab = crate::ui_side::RightTab::Jog;
    h.run_steps(2);
    h.get_by_label_contains("Laser per Klick verfahren").click();
    h.run_steps(2);
    assert!(h.state().move_by_click);
    let bed_h = h.state().settings.profile.bed_height;
    let target = Point::new(60.0, bed_h - 40.0);
    let at = h.state().view.tf.unwrap().to_screen(target);
    h.hover_at(at);
    h.run_steps(2);
    tap(&mut h, at);
    // Ursprung vorne links: Maschine X 60, Y 40
    wait_until(&mut h, |a| {
        a.machine.head_position().is_some_and(|p| (p.x - 60.0).abs() < 0.5 && (p.y - 40.0).abs() < 0.5)
    });
    assert!(h.state().selection.is_empty(), "im Verfahrmodus wird nichts ausgewählt");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(!h.state().move_by_click);
    h.state_mut().disconnect();
    crate::settings::TEST_CONFIG_DIR.with(|d| *d.borrow_mut() = None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Endschalter-Alarm mitten in einer Füllung (Simulator): der Abbruch wird gespeichert, „Job fortsetzen“ schlägt die
/// Stelle kurz vor dem Abbruch vor, markiert sie und lasert nach der Referenzfahrt nur den Rest.
#[test]
fn aborted_job_can_be_resumed() {
    use std::sync::Arc;
    let dir = std::env::temp_dir().join(format!("vectorlume-ui-fortsetzen-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::settings::TEST_CONFIG_DIR.with(|d| *d.borrow_mut() = Some(dir.clone()));
    let mut h = harness_sized(false, egui::vec2(1600.0, 2400.0));
    let (job, clean, limit_at) = {
        let app = h.state_mut();
        app.settings.safety_acknowledged = true;
        app.settings.profile.return_to_origin = false;
        let y = app.settings.profile.bed_height - 6.0;
        app.doc.add_path("Füllung", 0, VectorPath::rect(2.0, y, 10.0, 4.0, 0.0));
        app.doc.layers[0].settings =
            vl_core::CutSettings { interval: 0.2, ..vl_core::CutSettings::default_for(vl_core::LayerMode::Fill) };
        let job = vl_cam::build_job(&app.doc, &[], &app.settings.profile, &app.settings.job).unwrap();
        // Endschalter nach der Hälfte; zum Fortsetzen dieselben Zeilen mit einer harmlosen statt der Störzeile
        let base = vl_cam::strip_for_streaming(&job.gcode).to_vec();
        let k = base.len() / 2;
        let with = |extra: &str| {
            let mut l = base.clone();
            l.insert(k, extra.into());
            vl_cam::Job { gcode: l.into(), ..job.clone() }
        };
        app.selected_port = vl_device::SIMULATOR_PORT.into();
        app.connect();
        (Arc::new(with("$SIM=LIMIT")), with("G90"), k)
    };
    wait_until(&mut h, |a| a.machine.connected && a.machine.status.is_some());
    h.state_mut().send_job(&job);
    wait_until(&mut h, |a| a.machine.job_running);
    wait_until(&mut h, |a| !a.machine.job_running);
    let acked = h.state().machine.aborted.as_ref().expect("Abbruch gespeichert").acked;
    assert!(acked <= limit_at + 1, "{acked} / {limit_at}");
    h.state_mut().machine.aborted.as_mut().unwrap().job = Arc::new(clean);
    // Fenster erklärt den Endschalter-Alarm
    h.run_steps(2);
    assert!(matches!(h.state().modal, Some(crate::app::Modal::HardLimit { .. })));
    h.get_by_label("Schließen").click();
    h.run_steps(2);
    assert!(h.state().modal.is_none());
    // Referenzfahrt nach dem Alarm
    wait_until(&mut h, |a| a.machine.status.as_ref().is_some_and(|s| s.state == vl_device::MachineState::Alarm));
    h.state_mut().home();
    wait_until(&mut h, |a| a.machine.status.as_ref().is_some_and(|s| s.state == vl_device::MachineState::Idle));
    h.get_by_label_contains("Abgebrochenen Job fortsetzen").click();
    h.run_steps(3);
    let (line, marker) = {
        let st = h.state().resume.state.as_ref().expect("Fenster mit Job");
        let p = st.point().unwrap();
        (p.line, st.marker())
    };
    assert!(line > 0 && line <= limit_at, "Stelle {line} / Abbruch {limit_at}");
    let m = marker.expect("Markierung");
    assert!(m.x >= 1.9 && m.x <= 12.1, "{m:?}");
    h.get_by_label_contains("Ab hier fortsetzen").click();
    h.run_steps(2);
    wait_until(&mut h, |a| a.machine.job_running);
    assert!(h.state().machine.job_name.contains("Fortsetzung"));
    assert!(h.state().machine.job_progress.1 < job.gcode.len(), "nur der Rest wird gesendet");
    for _ in 0..1500 {
        h.run_ok();
        if !h.state().machine.job_running {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!h.state().machine.job_running);
    assert!(h.state().machine.aborted.is_none(), "{:?}", h.state().toasts.iter().map(|t| &t.text).collect::<Vec<_>>());
    h.state_mut().disconnect();
    crate::settings::TEST_CONFIG_DIR.with(|d| *d.borrow_mut() = None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Fährt der Kopf über die Arbeitsfläche hinaus, erklärt der Startdialog wo und warum und warnt deutlich vor
/// Einklemmen und Schäden am Gerät.
#[test]
fn start_warns_when_head_leaves_the_bed() {
    let mut h = harness(false);
    let job = {
        let app = h.state_mut();
        app.settings.safety_acknowledged = true;
        app.doc.add_path("zu weit links", 0, VectorPath::rect(-3.0, 100.0, 20.0, 20.0, 0.0));
        std::sync::Arc::new(vl_cam::build_job(&app.doc, &[], &app.settings.profile, &app.settings.job).unwrap())
    };
    assert!(job.leaves_bed);
    h.state_mut().launch_job(job);
    h.run_steps(2);
    h.get_by_label_contains("Der Laserkopf würde links 3 mm");
    h.get_by_label_contains("Teile des Motivs");
    h.get_by_label_contains("Alarm 1");
    h.get_by_label_contains("Nur starten, wenn wirklich nichts passieren kann");
    h.get_by_label("Abbrechen").click();
    h.run_steps(2);
    assert!(h.state().modal.is_none());
}

/// Konturen mit fast vollständiger Kehrtwende (Spitze eines dünnen nachgezeichneten Strichs) dürfen beim Zeichnen
/// keine Nadeln quer über die Arbeitsfläche erzeugen.
#[test]
fn sharp_contour_tips_draw_without_needles() {
    use egui::epaint::{Mesh, TessellationOptions, Tessellator};
    use egui::{Pos2, Stroke};
    // schmaler Keil: 200 px lang, an der Spitze 0,05 px breit
    let pts =
        vec![Pos2::new(100.0, 100.0), Pos2::new(300.0, 99.975), Pos2::new(100.0, 100.025), Pos2::new(100.0, 140.0)];
    let bounds = |shape: egui::Shape| {
        let mut tess = Tessellator::new(1.0, TessellationOptions::default(), [1, 1], vec![]);
        let mut mesh = Mesh::default();
        tess.tessellate_shape(shape, &mut mesh);
        mesh.calc_bounds()
    };
    let b = bounds(crate::canvas::outline(pts, true, Stroke::new(1.2, egui::Color32::WHITE)));
    assert!(b.max.x < 305.0 && b.min.x > 95.0 && b.min.y > 95.0 && b.max.y < 145.0, "{b:?}");
    // doppelte Punkte und zu kurze Züge
    let b = bounds(crate::canvas::outline(
        vec![
            Pos2::new(0.0, 0.0),
            Pos2::new(0.0, 0.0),
            Pos2::new(0.0, 0.0),
            Pos2::new(10.0, 0.0),
            Pos2::new(10.0, 10.0),
        ],
        true,
        Stroke::new(1.2, egui::Color32::WHITE),
    ));
    assert!(b.min.x > -3.0 && b.max.x < 13.0 && b.max.y < 13.0, "{b:?}");
    assert!(matches!(crate::canvas::outline(vec![Pos2::ZERO, Pos2::ZERO], true, Stroke::NONE), egui::Shape::Noop));
}
