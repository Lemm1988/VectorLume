//! Materialbibliothek und Materialtest (Fenster).

use crate::app::{ToastKind, VectorLumeApp};
use crate::i18n::{tr, trs};
use crate::theme;
use egui::{Button, Color32, CornerRadius, DragValue, RichText, Sense, Stroke, Ui, pos2, vec2};
use egui_phosphor::regular as icon;
use std::path::PathBuf;
use std::sync::Arc;
use vl_cam::{Anchor, Job, JobSettings, MachineProfile, StartFrom, TestAxis, TestParam, TestPattern};
use vl_core::{Affine, LayerMode, MaterialEntry, MaterialLibrary, Point, Polyline};

fn library_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("materials.json"))
}

#[derive(Default)]
pub struct MaterialsUi {
    pub library: MaterialLibrary,
    pub open: bool,
    pub filter: String,
    pub selected: Option<usize>,
    /// Neuer Eintrag, der gerade benannt wird (aus der aktuellen Ebene oder einem Testfeld).
    pub draft: Option<MaterialEntry>,
    pub load_error: Option<String>,
}

impl MaterialsUi {
    pub fn load() -> Self {
        let mut ui = Self::default();
        if let Some(p) = library_path().filter(|p| p.exists()) {
            match MaterialLibrary::load(&p) {
                Ok(lib) => ui.library = lib,
                // Defekte Datei nicht überschreiben, damit nichts verloren geht.
                Err(e) => ui.load_error = Some(e),
            }
        }
        ui
    }

    fn save(&self) -> Result<(), String> {
        if self.load_error.is_some() {
            return Err(tr(
                "Die Materialbibliothek konnte beim Start nicht gelesen werden und wird nicht überschrieben.",
            )
            .into());
        }
        let path = library_path().ok_or(tr("Kein Ordner für Einstellungen gefunden."))?;
        self.library.save(&path)
    }
}

#[derive(Default)]
pub struct TestUi {
    pub open: bool,
    pub show_on_canvas: bool,
    pub job: Option<Arc<Job>>,
    pub error: Option<String>,
    pub selected_cell: Option<(u32, u32)>,
    pub(crate) built: Option<(TestPattern, MachineProfile, JobSettings)>,
}

/// Beschriftung mit einer serifenlosen Systemschrift (Oberkante bei y = 0).
fn label_polylines(text: &str, height: f64) -> Vec<Polyline> {
    let db = vl_io::system_fonts();
    // Achsentitel kommen deutsch aus vl-cam und werden in der Sprache der Oberfläche gelasert
    let spec = vl_io::TextSpec { text: trs(text), family: "Arial".into(), height_mm: height, ..Default::default() };
    vl_io::text_to_path(&db, &spec).map(|p| p.flatten(&Affine::IDENTITY, 0.02)).unwrap_or_default()
}

impl VectorLumeApp {
    pub fn open_material_test(&mut self) {
        self.test.open = true;
        self.test.show_on_canvas = true;
        self.test.built = None;
    }

    /// Baut den Testjob neu, wenn sich Muster, Maschine oder Job-Einstellungen geändert haben.
    fn refresh_test_job(&mut self) {
        let key = (self.settings.test_pattern.clone(), self.settings.profile.clone(), self.settings.job.clone());
        if self.test.built.as_ref() == Some(&key) {
            return;
        }
        match key.0.build(&key.1, &key.2, &label_polylines) {
            Ok(job) => {
                self.test.job = Some(Arc::new(job));
                self.test.error = None;
            }
            Err(e) => {
                self.test.job = None;
                self.test.error = Some(e);
            }
        }
        let rows = key.0.rows.count;
        let cols = key.0.columns.count;
        if self.test.selected_cell.is_some_and(|(r, c)| r >= rows || c >= cols) {
            self.test.selected_cell = None;
        }
        self.test.built = Some(key);
    }

    fn apply_to_current_layer(&mut self, settings: &vl_core::CutSettings, what: &str) {
        let l = self.current_layer;
        let tab = self.sub_tab.min(self.doc.layers[l].sub.len());
        let Some(output) = self.doc.layers[l].operation(tab).map(|c| c.output) else { return };
        let mut s = settings.clone();
        s.output = output;
        self.checkpoint();
        if let Some(op) = self.doc.layers[l].operation_mut(tab) {
            *op = s;
        }
        self.mark_changed();
        let name = self.doc.layers[l].name.clone();
        self.toast(ToastKind::Success, trs(&format!("{what} auf Ebene {name} übernommen.")));
    }
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    material_test(app, ctx);
    library_window(app, ctx);
}

// ---------------------------------------------------------------- Materialtest

fn axis_ui(ui: &mut Ui, id: &str, axis: &mut TestAxis) {
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(id).width(130.0).selected_text(tr(axis.param.label())).show_ui(ui, |ui| {
            for p in TestParam::ALL {
                if ui.selectable_value(&mut axis.param, p, tr(p.label())).changed() {
                    let (min, max) = match p {
                        TestParam::Speed => (1000.0, 6000.0),
                        TestParam::Power => (10.0, 50.0),
                        TestParam::Passes => (1.0, 5.0),
                        TestParam::Interval => (0.05, 0.25),
                        TestParam::Focus => (-2.0, 2.0),
                    };
                    axis.min = min;
                    axis.max = max;
                }
            }
        });
        let (lo, hi) = axis.param.range();
        let (speed, decimals) = match axis.param {
            TestParam::Speed => (10.0, 0),
            TestParam::Power => (0.5, 1),
            TestParam::Passes => (0.05, 0),
            TestParam::Interval => (0.005, 2),
            TestParam::Focus => (0.05, 1),
        };
        let unit = format!(" {}", axis.param.unit());
        ui.add(DragValue::new(&mut axis.min).speed(speed).range(lo..=hi).fixed_decimals(decimals).suffix(&unit));
        ui.label(tr("bis"));
        ui.add(DragValue::new(&mut axis.max).speed(speed).range(lo..=hi).fixed_decimals(decimals).suffix(&unit));
        ui.add(DragValue::new(&mut axis.count).range(1..=vl_cam::testpattern::MAX_STEPS).suffix(tr(" Stufen")));
    });
}

fn material_test(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.test.open {
        return;
    }
    app.refresh_test_job();
    let mut open = true;
    let mut t = app.settings.test_pattern.clone();
    let mut js = app.settings.job.clone();
    let mut action: Option<&str> = None;
    let mut cell_click: Option<(u32, u32)> = None;
    let z_axis = app.settings.profile.z_axis;
    egui::Window::new(trs(&format!("{} Materialtest", icon::GRID_FOUR)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(860.0)
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            ui.label(
                RichText::new(tr(
                    "Brennt ein Raster aus Feldern mit gestaffelten Werten. Danach das beste Feld anklicken und als \
                     Material speichern.",
                ))
                .color(pal.text_weak),
            );
            ui.add_space(6.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(470.0);
                    egui::Grid::new("test_grid").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                        ui.label(tr("Modus"));
                        ui.horizontal(|ui| {
                            for m in [LayerMode::Fill, LayerMode::Line] {
                                if ui.selectable_label(t.base.mode == m, tr(m.label())).clicked() && t.base.mode != m {
                                    let keep = (t.base.air_assist, t.base.interval);
                                    t.base = vl_core::CutSettings::default_for(m);
                                    (t.base.air_assist, t.base.interval) = keep;
                                }
                            }
                        });
                        ui.end_row();
                        ui.label(tr("Spalten"));
                        axis_ui(ui, "test_cols", &mut t.columns);
                        ui.end_row();
                        ui.label(tr("Zeilen"));
                        axis_ui(ui, "test_rows", &mut t.rows);
                        ui.end_row();
                        if !z_axis && (t.columns.param == TestParam::Focus || t.rows.param == TestParam::Focus) {
                            ui.label("");
                            ui.label(
                                RichText::new(trs(&format!(
                                    "{} Der Fokustest braucht eine Z-Achse (Laser → Maschineneinstellungen).",
                                    icon::WARNING
                                )))
                                .color(pal.warning),
                            );
                            ui.end_row();
                        }

                        let fixed = |p: TestParam| t.columns.param == p || t.rows.param == p;
                        let (fs, fp, fpa, fi) = (
                            fixed(TestParam::Speed),
                            fixed(TestParam::Power),
                            fixed(TestParam::Passes),
                            fixed(TestParam::Interval),
                        );
                        ui.label(tr("Feste Werte"));
                        ui.horizontal(|ui| {
                            ui.add_enabled(
                                !fs,
                                DragValue::new(&mut t.base.speed).speed(10.0).range(1.0..=100000.0).suffix(" mm/min"),
                            );
                            ui.add_enabled(
                                !fp,
                                DragValue::new(&mut t.base.power_max).speed(0.5).range(0.0..=100.0).suffix(" %"),
                            );
                            ui.add_enabled(!fpa, DragValue::new(&mut t.base.passes).range(1..=50).suffix("×"));
                            if t.base.mode == LayerMode::Fill {
                                ui.add_enabled(
                                    !fi,
                                    DragValue::new(&mut t.base.interval)
                                        .speed(0.005)
                                        .range(0.02..=2.0)
                                        .fixed_decimals(2)
                                        .suffix(" mm"),
                                );
                            }
                        });
                        ui.end_row();
                        if t.base.mode == LayerMode::Fill {
                            ui.label(tr("Füllen"));
                            ui.horizontal(|ui| {
                                ui.add(
                                    DragValue::new(&mut t.base.fill_angle).speed(1.0).range(-180.0..=180.0).suffix("°"),
                                );
                                ui.checkbox(&mut t.base.crosshatch, tr("Kreuz"));
                                ui.checkbox(&mut t.base.bidirectional, tr("Bidirektional"));
                                ui.add(
                                    DragValue::new(&mut t.base.overscan)
                                        .speed(0.1)
                                        .range(0.0..=20.0)
                                        .prefix(tr("Overscan "))
                                        .suffix(" mm"),
                                );
                            });
                            ui.end_row();
                        }
                        ui.label(tr("Luftunterstützung"));
                        ui.checkbox(&mut t.base.air_assist, "");
                        ui.end_row();
                        ui.label(tr("Felder"));
                        ui.horizontal(|ui| {
                            ui.add(DragValue::new(&mut t.cell_width).speed(0.1).range(1.0..=100.0).suffix(" mm"));
                            ui.label("×");
                            ui.add(DragValue::new(&mut t.cell_height).speed(0.1).range(1.0..=100.0).suffix(" mm"));
                            ui.label(tr("Abstand"));
                            ui.add(DragValue::new(&mut t.gap).speed(0.1).range(0.0..=50.0).suffix(" mm"));
                        });
                        ui.end_row();
                        ui.label(tr("Beschriftung"));
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut t.labels, "");
                            ui.add_enabled_ui(t.labels, |ui| {
                                ui.add(
                                    DragValue::new(&mut t.label_height)
                                        .speed(0.1)
                                        .range(1.0..=20.0)
                                        .prefix(tr("Höhe "))
                                        .suffix(" mm"),
                                );
                                ui.add(
                                    DragValue::new(&mut t.label_speed)
                                        .speed(10.0)
                                        .range(1.0..=100000.0)
                                        .suffix(" mm/min"),
                                );
                                ui.add(DragValue::new(&mut t.label_power).speed(0.5).range(0.0..=100.0).suffix(" %"));
                            });
                        });
                        ui.end_row();
                        ui.label(tr("Start von"));
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt("test_start")
                                .selected_text(tr(js.start_from.label()))
                                .show_ui(ui, |ui| {
                                    for s in StartFrom::ALL {
                                        ui.selectable_value(&mut js.start_from, s, tr(s.label()));
                                    }
                                });
                            if js.start_from != StartFrom::Absolute {
                                egui::ComboBox::from_id_salt("test_anchor")
                                    .selected_text(tr(js.anchor.label()))
                                    .show_ui(ui, |ui| {
                                        for &a in Anchor::GRID.iter().flatten() {
                                            ui.selectable_value(&mut js.anchor, a, tr(a.label()));
                                        }
                                    });
                            }
                        });
                        ui.end_row();
                        if js.start_from == StartFrom::Absolute {
                            ui.label(tr("Position"));
                            ui.horizontal(|ui| {
                                ui.add(DragValue::new(&mut t.position.x).speed(0.5).prefix("X ").suffix(" mm"));
                                ui.add(DragValue::new(&mut t.position.y).speed(0.5).prefix("Y ").suffix(" mm"));
                                ui.label(RichText::new(tr("linke untere Ecke")).small().color(pal.text_weak));
                            });
                            ui.end_row();
                        }
                    });
                });
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    ui.set_width(340.0);
                    if let Some(job) = app.test.job.clone() {
                        if let Some(c) = test_preview(ui, &t, app.test.selected_cell, &app.settings.profile) {
                            cell_click = Some(c);
                        }
                        let b = job.design_bounds;
                        ui.label(trs(&format!(
                            "{} Felder · {:.0} × {:.0} mm · ca. {}",
                            t.cell_count(),
                            b.width(),
                            b.height(),
                            job.estimate.format_duration()
                        )));
                        for w in &job.warnings {
                            ui.label(RichText::new(format!("{} {w}", icon::WARNING)).color(pal.warning));
                        }
                    } else if let Some(e) = &app.test.error {
                        ui.label(RichText::new(format!("{} {e}", icon::WARNING)).color(pal.danger));
                    }
                    ui.checkbox(&mut app.test.show_on_canvas, tr("Auf der Arbeitsfläche anzeigen"));
                    ui.add_space(4.0);
                    if let Some((r, c)) = app.test.selected_cell {
                        let cs = t.cell_settings(r, c);
                        crate::ui_side::card(ui, |ui| {
                            ui.label(RichText::new(trs(&format!("Feld Zeile {}, Spalte {}", r + 1, c + 1))).strong());
                            ui.label(trs(&cs.summary()));
                            if cs.mode == LayerMode::Fill {
                                ui.label(trs(&format!("Zeilenabstand {:.2} mm", cs.interval)));
                            }
                            ui.horizontal(|ui| {
                                if ui
                                    .button(trs(&format!("{} Als Material speichern…", icon::BOOKMARK_SIMPLE)))
                                    .clicked()
                                {
                                    action = Some("save_cell");
                                }
                                if ui.button(tr("Auf aktuelle Ebene")).clicked() {
                                    action = Some("apply_cell");
                                }
                            });
                        });
                    } else {
                        ui.label(
                            RichText::new(tr("Feld anklicken, um seine Werte zu übernehmen."))
                                .small()
                                .color(pal.text_weak),
                        );
                    }
                });
            });
            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                let ready = app.test.job.is_some();
                let connected = app.machine.connected && !app.machine.job_running;
                if ui
                    .add_enabled(ready && connected, Button::new(trs(&format!("{} Rahmen", icon::BOUNDING_BOX))))
                    .clicked()
                {
                    action = Some("frame");
                }
                if ui
                    .add_enabled(
                        ready && connected,
                        Button::new(RichText::new(trs(&format!("{} Test starten", icon::PLAY))).color(Color32::WHITE))
                            .fill(pal.success),
                    )
                    .clicked()
                {
                    action = Some("start");
                }
                if ui
                    .add_enabled(ready, Button::new(trs(&format!("{} G-Code speichern…", icon::FLOPPY_DISK))))
                    .clicked()
                {
                    action = Some("export");
                }
                if !app.machine.connected {
                    ui.label(RichText::new(tr("Kein Laser verbunden")).small().color(pal.text_weak));
                }
            });
        });
    if t != app.settings.test_pattern || js != app.settings.job {
        app.settings.test_pattern = t;
        app.settings.job = js;
        app.settings.save();
        app.revision += 1;
    }
    if let Some(c) = cell_click {
        app.test.selected_cell = if app.test.selected_cell == Some(c) { None } else { Some(c) };
    }
    let cell = app.test.selected_cell.map(|(r, c)| app.settings.test_pattern.cell_settings(r, c));
    match (action, app.test.job.clone()) {
        (Some("frame"), Some(job)) => {
            if app.ready_to_frame() {
                app.run_frame(&job, false);
            }
        }
        (Some("start"), Some(job)) => {
            if app.ready_to_start() {
                app.launch_job(job);
            }
        }
        (Some("export"), Some(job)) => {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter(tr("G-Code"), &["gcode", "nc"])
                .set_file_name(tr("Materialtest.gcode"))
                .save_file()
            {
                app.write_gcode(&job, &path);
            }
        }
        (Some("save_cell"), _) => {
            if let Some(cs) = cell {
                let title = if cs.mode == LayerMode::Line { tr("Schneiden") } else { tr("Gravieren") };
                app.materials.draft = Some(MaterialEntry { title: title.into(), settings: cs, ..Default::default() });
                app.materials.open = true;
            }
        }
        (Some("apply_cell"), _) => {
            if let Some(cs) = cell {
                app.apply_to_current_layer(&cs, tr("Testfeld"));
            }
        }
        _ => {}
    }
    if !open {
        app.test.open = false;
    }
}

/// Verkleinerte Ansicht des Testmusters; Felder sind nach erwarteter Schwärzung eingefärbt. Gibt ein angeklicktes Feld zurück.
fn test_preview(
    ui: &mut Ui,
    t: &TestPattern,
    selected: Option<(u32, u32)>,
    profile: &MachineProfile,
) -> Option<(u32, u32)> {
    let pal = theme::of(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(340.0, 300.0), Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(8), pal.canvas);
    let layout = t.layout(profile, &label_polylines);
    let b = layout.bounds;
    if b.is_empty() {
        return None;
    }
    let scale = ((rect.width() - 24.0) as f64 / b.width()).min((rect.height() - 24.0) as f64 / b.height());
    let off = rect.center() - vec2((b.width() * scale) as f32 / 2.0, (b.height() * scale) as f32 / 2.0);
    let to_screen = |p: Point| pos2(off.x + ((p.x - b.min.x) * scale) as f32, off.y + ((p.y - b.min.y) * scale) as f32);

    // Relative Energie je Fläche: Leistung × Durchgänge / (Geschwindigkeit × Zeilenabstand)
    let energy = |r: u32, c: u32| {
        let cs = t.cell_settings(r, c);
        let interval = if cs.mode == LayerMode::Fill { cs.interval.max(0.01) } else { 0.1 };
        cs.power_max * cs.passes as f64 / (cs.speed.max(1.0) * interval)
    };
    let max_e = layout.cells.iter().map(|&(r, c, _)| energy(r, c)).fold(0.0, f64::max).max(1e-9);
    let burn = Color32::from_rgb(70, 40, 20);
    let wood = Color32::from_rgb(222, 196, 150);
    let mut clicked = None;
    let pointer = resp.interact_pointer_pos();
    for &(r, c, cell) in &layout.cells {
        let sr = egui::Rect::from_two_pos(to_screen(cell.min), to_screen(cell.max));
        let k = (energy(r, c) / max_e).powf(0.6) as f32;
        let fill = lerp_color(wood, burn, k);
        if t.base.mode == LayerMode::Fill {
            painter.rect_filled(sr, 0.0, fill);
        } else {
            painter.rect_filled(sr, 0.0, wood.gamma_multiply(0.25));
            painter.rect_stroke(sr, 0.0, Stroke::new(1.0 + 2.0 * k, fill), egui::StrokeKind::Inside);
        }
        if selected == Some((r, c)) {
            painter.rect_stroke(sr.expand(2.0), 2.0, Stroke::new(2.0, pal.accent), egui::StrokeKind::Outside);
        }
        if resp.clicked() && pointer.is_some_and(|p| sr.expand(1.0).contains(p)) {
            clicked = Some((r, c));
        }
    }
    for poly in &layout.labels {
        let pts: Vec<egui::Pos2> = poly.points.iter().map(|&p| to_screen(p)).collect();
        if pts.len() >= 2 {
            painter.add(egui::Shape::closed_line(pts, Stroke::new(0.8, pal.text_weak)));
        }
    }
    clicked
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

// ---------------------------------------------------------------- Bibliothek

fn library_window(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.materials.open {
        return;
    }
    let mut open = true;
    let z_axis = app.settings.profile.z_axis;
    let mut apply: Option<usize> = None;
    let mut delete: Option<usize> = None;
    let mut save_draft = false;
    let mut cancel_draft = false;
    let mut from_layer = false;
    let mut import = false;
    let mut export = false;
    let mut changed = false;
    let layer_name = app.doc.layers[app.current_layer].name.clone();
    egui::Window::new(trs(&format!("{} Materialbibliothek", icon::BOOKS)))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(560.0)
        .default_height(520.0)
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let m = &mut app.materials;
            if let Some(e) = &m.load_error {
                ui.label(RichText::new(trs(&format!("{} materials.json ist beschädigt: {e}", icon::WARNING))).color(pal.danger));
            }
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut m.filter).hint_text(trs(&format!("{} Suchen…", icon::MAGNIFYING_GLASS))).desired_width(180.0));
                if ui.button(trs(&format!("{} Ebene {layer_name} speichern…", icon::PLUS))).clicked() {
                    from_layer = true;
                }
                if ui.button(trs(&format!("{} Import", icon::DOWNLOAD_SIMPLE))).clicked() {
                    import = true;
                }
                if ui.add_enabled(!m.library.entries.is_empty(), Button::new(trs(&format!("{} Export", icon::EXPORT)))).clicked() {
                    export = true;
                }
            });
            ui.add_space(6.0);

            if let Some(d) = &mut m.draft {
                crate::ui_side::card(ui, |ui| {
                    ui.label(RichText::new(tr("Neuer Eintrag")).strong());
                    egui::Grid::new("draft").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                        ui.label(tr("Material"));
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut d.material).hint_text(tr("z. B. Pappelsperrholz")).desired_width(200.0));
                            let known = m.library.materials();
                            if !known.is_empty() {
                                egui::ComboBox::from_id_salt("known").selected_text("").width(24.0).show_ui(ui, |ui| {
                                    for k in known {
                                        if ui.selectable_label(false, &k).clicked() {
                                            d.material = k;
                                        }
                                    }
                                });
                            }
                        });
                        ui.end_row();
                        ui.label(tr("Stärke"));
                        ui.add(DragValue::new(&mut d.thickness).speed(0.1).range(0.0..=100.0).suffix(" mm"))
                            .on_hover_text(tr("0 = beliebige Stärke (z. B. Gravur)"));
                        ui.end_row();
                        ui.label(tr("Bezeichnung"));
                        ui.add(egui::TextEdit::singleline(&mut d.title).hint_text(tr("z. B. Schneiden, Gravieren dunkel")).desired_width(200.0));
                        ui.end_row();
                        ui.label(tr("Notiz"));
                        ui.add(egui::TextEdit::singleline(&mut d.notes).desired_width(260.0));
                        ui.end_row();
                        ui.label(tr("Werte"));
                        ui.label(trs(&d.settings.summary()));
                        ui.end_row();
                    });
                    ui.horizontal(|ui| {
                        let ok = !d.material.trim().is_empty() && !d.title.trim().is_empty();
                        if ui.add_enabled(ok, Button::new(RichText::new(tr("Speichern")).color(Color32::WHITE)).fill(pal.accent)).clicked() {
                            save_draft = true;
                        }
                        if ui.button(tr("Abbrechen")).clicked() {
                            cancel_draft = true;
                        }
                    });
                });
                ui.add_space(6.0);
            }

            if m.library.entries.is_empty() {
                ui.label(RichText::new(
                    tr("Noch keine Materialien. Tipp: Mit Laser → Materialtest die passenden Werte ermitteln und das beste \
                     Feld hier speichern."),
                ).color(pal.text_weak));
                return;
            }
            let filter = m.filter.to_lowercase();
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(280.0).show(ui, |ui| {
                for material in m.library.materials() {
                    let idx: Vec<usize> = (0..m.library.entries.len())
                        .filter(|&i| m.library.entries[i].material.eq_ignore_ascii_case(&material))
                        .filter(|&i| {
                            let e = &m.library.entries[i];
                            filter.is_empty()
                                || format!("{} {} {}", e.material, e.title, e.notes).to_lowercase().contains(&filter)
                        })
                        .collect();
                    if idx.is_empty() {
                        continue;
                    }
                    egui::CollapsingHeader::new(RichText::new(&material).strong()).default_open(true).id_salt(&material).show(ui, |ui| {
                        for i in idx {
                            let e = &m.library.entries[i];
                            let text = format!("{} · {}  —  {}", trs(&e.thickness_label()), e.title, trs(&e.settings.summary()));
                            let resp = ui.selectable_label(m.selected == Some(i), text);
                            if resp.clicked() {
                                m.selected = Some(i);
                            }
                            if resp.double_clicked() {
                                apply = Some(i);
                            }
                        }
                    });
                }
            });
            if let Some(i) = m.selected.filter(|&i| i < m.library.entries.len()) {
                ui.separator();
                let e = &mut m.library.entries[i];
                ui.label(RichText::new(format!("{} {} · {}", e.material, trs(&e.thickness_label()), e.title)).strong());
                let before = e.clone();
                ui.horizontal(|ui| {
                    ui.label(tr("Notiz"));
                    ui.add(egui::TextEdit::singleline(&mut e.notes).desired_width(320.0));
                });
                crate::ui_side::cut_settings_ui(ui, &mut e.settings, false, z_axis);
                if *e != before {
                    changed = true;
                }
                ui.horizontal(|ui| {
                    if ui.add(Button::new(RichText::new(trs(&format!("Auf Ebene {layer_name} anwenden"))).color(Color32::WHITE)).fill(pal.accent)).clicked() {
                        apply = Some(i);
                    }
                    if ui.button(trs(&format!("{} Löschen", icon::TRASH))).clicked() {
                        delete = Some(i);
                    }
                });
            }
        });

    if from_layer {
        let layer = &app.doc.layers[app.current_layer];
        let s = layer.operation(app.sub_tab).unwrap_or(&layer.settings).clone();
        let title = if s.mode == LayerMode::Line { tr("Schneiden") } else { tr("Gravieren") };
        app.materials.draft = Some(MaterialEntry { title: title.into(), settings: s, ..Default::default() });
    }
    if cancel_draft {
        app.materials.draft = None;
    }
    if save_draft && let Some(mut d) = app.materials.draft.take() {
        d.material = d.material.trim().to_string();
        d.title = d.title.trim().to_string();
        let label = format!("{} {} · {}", d.material, trs(&d.thickness_label()), d.title);
        let replaced = app.materials.library.upsert(d);
        app.materials.selected = None;
        changed = true;
        app.toast(
            ToastKind::Success,
            if replaced { trs(&format!("{label} aktualisiert.")) } else { trs(&format!("{label} gespeichert.")) },
        );
    }
    if let Some(i) = delete {
        app.materials.library.entries.remove(i);
        app.materials.selected = None;
        changed = true;
    }
    if let Some(i) = apply {
        let e = app.materials.library.entries[i].clone();
        app.apply_to_current_layer(&e.settings, &format!("{} {}", e.material, e.title));
    }
    if import
        && let Some(path) =
            rfd::FileDialog::new().add_filter(tr("VectorLume-Materialien"), &["vlmat", "json"]).pick_file()
    {
        match MaterialLibrary::load(&path) {
            Ok(lib) => {
                let n = app.materials.library.merge(lib);
                changed = true;
                app.toast(ToastKind::Success, trs(&format!("{n} Einträge importiert.")));
            }
            Err(e) => app.toast(ToastKind::Error, e),
        }
    }
    if export
        && let Some(path) = rfd::FileDialog::new()
            .add_filter(tr("VectorLume-Materialien"), &["vlmat"])
            .set_file_name(tr("Materialien.vlmat"))
            .save_file()
    {
        match app.materials.library.save(&path) {
            Ok(()) => {
                app.toast(ToastKind::Success, trs(&format!("Bibliothek exportiert: {}", crate::app::file_name(&path))))
            }
            Err(e) => app.toast(ToastKind::Error, e),
        }
    }
    if changed && let Err(e) = app.materials.save() {
        app.toast(ToastKind::Error, trs(&format!("Materialbibliothek nicht gespeichert: {e}")));
    }
    if !open {
        app.materials.open = false;
        app.materials.draft = None;
    }
}
