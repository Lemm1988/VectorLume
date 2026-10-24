//! Kamera über der Arbeitsfläche: Live-Bild (Webcam) oder Foto, über vier Passpunkte perspektivisch entzerrt und
//! halbtransparent auf der Arbeitsfläche angezeigt – zum genauen Platzieren auf dem Werkstück.

use crate::app::{ToastKind, VectorLumeApp};
use crate::i18n::{tr, trs};
use crate::theme;
use egui::{Align2, Button, Color32, DragValue, RichText, Sense, Stroke, TextureOptions, pos2, vec2};
use egui_phosphor::regular as icon;
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{ApiBackend, CameraIndex, RequestedFormat, RequestedFormatType};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use vl_core::{Document, LayerMode, Point, VectorPath};
use vl_raster::warp::{Homography, warp_to_area};

/// Längere Seite des entzerrten Bildes in Pixeln.
const OVERLAY_PX: usize = 1200;
/// Abstand der Bilder im Live-Betrieb.
const LIVE_INTERVAL: Duration = Duration::from_millis(300);
/// Größe der Ausrichtmarken (Kreuze) in mm.
const MARK_SIZE: f64 = 8.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CameraPoint {
    /// Lage im Kamerabild (0…1); `None` = noch nicht gesetzt.
    pub image: Option<[f64; 2]>,
    /// Lage auf dem Bett in Maschinenkoordinaten (mm).
    pub bed: [f64; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraSettings {
    /// Name der zuletzt verwendeten Kamera.
    pub device: String,
    pub points: [CameraPoint; 4],
    /// Entzerrtes Bild auf der Arbeitsfläche anzeigen.
    pub overlay: bool,
    pub opacity: f32,
    /// Leistung (%) und Geschwindigkeit (mm/min) der Ausrichtmarken.
    pub mark_power: f64,
    pub mark_speed: f64,
}

impl Default for CameraSettings {
    fn default() -> Self {
        Self {
            device: String::new(),
            points: Default::default(),
            overlay: true,
            opacity: 0.7,
            mark_power: 20.0,
            mark_speed: 1500.0,
        }
    }
}

impl CameraSettings {
    /// Passpunkte bei 20 % und 80 % der Arbeitsfläche, solange keine eingetragen sind.
    pub fn ensure_points(&mut self, width: f64, height: f64) {
        if self.points.iter().all(|p| p.bed == [0.0, 0.0]) {
            let (x0, x1, y0, y1) = (width * 0.2, width * 0.8, height * 0.2, height * 0.8);
            for (p, b) in self.points.iter_mut().zip([[x0, y1], [x1, y1], [x1, y0], [x0, y0]]) {
                p.bed = b;
            }
        }
    }

    pub fn calibrated(&self) -> bool {
        self.points.iter().all(|p| p.image.is_some())
    }
}

/// Kamerabild in RGB.
#[derive(Clone)]
struct Frame {
    width: usize,
    height: usize,
    rgb: Arc<Vec<u8>>,
}

/// Aufnahme in einem eigenen Thread; endet beim Verwerfen.
struct Capture {
    stop: Arc<AtomicBool>,
    shared: Arc<Mutex<Option<Result<Frame, String>>>>,
    live: bool,
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn start_capture(index: CameraIndex, live: bool, ctx: egui::Context) -> Capture {
    let stop = Arc::new(AtomicBool::new(false));
    let shared: Arc<Mutex<Option<Result<Frame, String>>>> = Arc::new(Mutex::new(None));
    let (stop2, shared2) = (stop.clone(), shared.clone());
    let spawned = std::thread::Builder::new().name("kamera".into()).spawn(move || {
        let put = |r: Result<Frame, String>| {
            *shared2.lock().unwrap() = Some(r);
            ctx.request_repaint();
        };
        let format = RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestResolution);
        let mut cam = match nokhwa::Camera::new(index, format) {
            Ok(c) => c,
            Err(e) => return put(Err(trs(&format!("Kamera konnte nicht geöffnet werden: {e}")))),
        };
        if let Err(e) = cam.open_stream() {
            return put(Err(trs(&format!(
                "Kamera konnte nicht gestartet werden (wird sie von einem anderen Programm benutzt?): {e}"
            ))));
        }
        let mut count = 0;
        while !stop2.load(Ordering::Relaxed) {
            match cam.frame() {
                Ok(buf) => {
                    count += 1;
                    // Die ersten Bilder sind oft noch nicht richtig belichtet
                    if count < 4 {
                        continue;
                    }
                    match buf.decode_image::<RgbFormat>() {
                        Ok(img) => put(Ok(Frame {
                            width: img.width() as usize,
                            height: img.height() as usize,
                            rgb: Arc::new(img.into_raw()),
                        })),
                        Err(e) => {
                            put(Err(trs(&format!("Kamerabild nicht lesbar: {e}"))));
                            break;
                        }
                    }
                    if !live {
                        break;
                    }
                    std::thread::sleep(LIVE_INTERVAL);
                }
                Err(e) => {
                    put(Err(trs(&format!("Kein Bild von der Kamera: {e}"))));
                    break;
                }
            }
        }
        let _ = cam.stop_stream();
    });
    if let Err(e) = spawned {
        *shared.lock().unwrap() = Some(Err(e.to_string()));
    }
    Capture { stop, shared, live }
}

#[derive(Default)]
pub struct CameraState {
    pub open: bool,
    devices: Option<Vec<(CameraIndex, String)>>,
    capture: Option<Capture>,
    frame: Option<Frame>,
    frame_gen: u64,
    preview: Option<(u64, egui::TextureHandle)>,
    overlay: Option<egui::TextureHandle>,
    overlay_key: String,
    /// Passpunkt, der mit dem nächsten Klick ins Bild gesetzt wird.
    picking: Option<usize>,
    error: Option<String>,
}

impl CameraState {
    pub fn is_live(&self) -> bool {
        self.capture.as_ref().is_some_and(|c| c.live)
    }

    fn set_frame(&mut self, frame: Frame) {
        self.frame = Some(frame);
        self.frame_gen += 1;
        self.error = None;
    }
}

/// Neue Kamerabilder übernehmen und das entzerrte Bild für die Arbeitsfläche aktualisieren.
pub fn tick(app: &mut VectorLumeApp, ctx: &egui::Context) {
    let cam = &mut app.camera;
    if let Some(c) = &cam.capture {
        let msg = c.shared.lock().unwrap().take();
        let live = c.live;
        match msg {
            Some(Ok(frame)) => {
                cam.set_frame(frame);
                if !live {
                    cam.capture = None;
                }
            }
            Some(Err(e)) => {
                cam.error = Some(e);
                cam.capture = None;
            }
            None => {}
        }
    }
    let cs = &app.settings.camera;
    if !cs.overlay || !cs.calibrated() {
        cam.overlay = None;
        cam.overlay_key.clear();
        return;
    }
    let Some(frame) = cam.frame.clone() else { return };
    let profile = &app.settings.profile;
    let doc_points: Vec<Point> =
        cs.points.iter().map(|p| profile.machine_to_doc(Point::new(p.bed[0], p.bed[1]))).collect();
    let key =
        format!("{} {:?} {:?} {} {}", cam.frame_gen, cs.points, doc_points, profile.bed_width, profile.bed_height);
    if key == cam.overlay_key {
        return;
    }
    cam.overlay_key = key;
    let Some(to_image) = homography(cs, profile).and_then(|h| h.inverse()) else {
        cam.overlay = None;
        return;
    };
    let w = warp_to_area(&frame.rgb, frame.width, frame.height, &to_image, profile.bed_rect(), OVERLAY_PX);
    let image = egui::ColorImage::from_rgba_unmultiplied([w.width, w.height], &w.rgba);
    match &mut cam.overlay {
        Some(t) => t.set(image, TextureOptions::LINEAR),
        None => cam.overlay = Some(ctx.load_texture("kamera-arbeitsflaeche", image, TextureOptions::LINEAR)),
    }
}

/// Abbildung Kamerabild (0…1) → Dokumentkoordinaten (mm).
fn homography(cs: &CameraSettings, profile: &vl_cam::MachineProfile) -> Option<Homography> {
    let mut from = [Point::ZERO; 4];
    let mut to = [Point::ZERO; 4];
    for (i, p) in cs.points.iter().enumerate() {
        let [u, v] = p.image?;
        from[i] = Point::new(u, v);
        to[i] = profile.machine_to_doc(Point::new(p.bed[0], p.bed[1]));
    }
    Homography::from_points(&from, &to)
}

/// Entzerrtes Kamerabild unter Raster und Objekten zeichnen.
pub fn paint_overlay(app: &VectorLumeApp, painter: &egui::Painter, bed_screen: egui::Rect) {
    if let Some(tex) = &app.camera.overlay
        && app.settings.camera.overlay
    {
        let uv = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let alpha = (app.settings.camera.opacity.clamp(0.05, 1.0) * 255.0) as u8;
        painter.image(tex.id(), bed_screen, uv, Color32::from_white_alpha(alpha));
    }
}

fn devices() -> Vec<(CameraIndex, String)> {
    nokhwa::query(ApiBackend::Auto)
        .map(|list| list.into_iter().map(|c| (c.index().clone(), c.human_name())).collect())
        .unwrap_or_default()
}

/// Ort eines Passpunktes, wie er auf der Arbeitsfläche erscheint.
fn place_label(app: &VectorLumeApp, bed: [f64; 2]) -> &'static str {
    let p = app.settings.profile.machine_to_doc(Point::new(bed[0], bed[1]));
    let (w, h) = (app.settings.profile.bed_width, app.settings.profile.bed_height);
    match (p.x < w / 2.0, p.y < h / 2.0) {
        (true, true) => tr("oben links"),
        (false, true) => tr("oben rechts"),
        (false, false) => tr("unten rechts"),
        (true, false) => tr("unten links"),
    }
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.camera.open {
        return;
    }
    let (bw, bh) = (app.settings.profile.bed_width, app.settings.profile.bed_height);
    app.settings.camera.ensure_points(bw, bh);
    if app.camera.devices.is_none() {
        app.camera.devices = Some(devices());
    }
    let before = app.settings.camera.clone();
    let mut open = true;
    let mut action: Option<&str> = None;
    egui::Window::new(trs(&format!("{} Kamera", icon::CAMERA)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(560.0)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let devices = app.camera.devices.clone().unwrap_or_default();
            // Quelle
            ui.horizontal(|ui| {
                ui.label(tr("Kamera"));
                let current = devices
                    .iter()
                    .find(|d| d.1 == app.settings.camera.device)
                    .or(devices.first())
                    .map(|d| d.1.clone())
                    .unwrap_or_else(|| tr("keine gefunden").into());
                egui::ComboBox::from_id_salt("camera_device").selected_text(&current).width(220.0).show_ui(ui, |ui| {
                    for (_, name) in &devices {
                        ui.selectable_value(&mut app.settings.camera.device, name.clone(), name);
                    }
                });
                if ui.button(icon::ARROWS_CLOCKWISE).on_hover_text(tr("Kameras neu suchen")).clicked() {
                    app.camera.devices = Some(self::devices());
                }
            });
            ui.horizontal(|ui| {
                let has_cam = !devices.is_empty();
                let busy = app.camera.capture.is_some();
                if ui.add_enabled(has_cam && !busy, Button::new(trs(&format!("{} Aufnehmen", icon::CAMERA)))).clicked() {
                    action = Some("snap");
                }
                let mut live = app.camera.is_live();
                if ui.add_enabled(has_cam, egui::Checkbox::new(&mut live, tr("Live"))).on_hover_text(tr("Bild laufend aktualisieren")).changed() {
                    action = Some(if live { "live" } else { "stop" });
                }
                if ui.button(trs(&format!("{} Foto laden…", icon::IMAGE))).on_hover_text(tr("Foto der Arbeitsfläche, z. B. vom Handy senkrecht von oben")).clicked() {
                    action = Some("photo");
                }
                if busy && !live {
                    ui.spinner();
                }
            });
            if let Some(e) = &app.camera.error {
                ui.colored_label(pal.danger, trs(e));
            }
            ui.add_space(6.0);

            // Vorschau mit Passpunkten
            if let Some(frame) = app.camera.frame.clone() {
                let generation = app.camera.frame_gen;
                if app.camera.preview.as_ref().is_none_or(|(g, _)| *g != generation) {
                    let img = egui::ColorImage::from_rgb([frame.width, frame.height], &frame.rgb);
                    match &mut app.camera.preview {
                        Some((g, t)) => {
                            t.set(img, TextureOptions::LINEAR);
                            *g = generation;
                        }
                        None => app.camera.preview = Some((generation, ctx.load_texture("kamera-vorschau", img, TextureOptions::LINEAR))),
                    }
                }
                let tex = app.camera.preview.as_ref().unwrap().1.id();
                let width = 540.0f32;
                let size = vec2(width, width * frame.height as f32 / frame.width as f32);
                let resp = ui.add(egui::Image::new((tex, size)).sense(Sense::click()));
                let r = resp.rect;
                if app.camera.picking.is_some() {
                    resp.ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                }
                if resp.clicked()
                    && let (Some(i), Some(pos)) = (app.camera.picking, resp.interact_pointer_pos())
                {
                    let u = ((pos.x - r.min.x) / r.width()).clamp(0.0, 1.0) as f64;
                    let v = ((pos.y - r.min.y) / r.height()).clamp(0.0, 1.0) as f64;
                    app.settings.camera.points[i].image = Some([u, v]);
                    app.camera.picking = (i + 1..4).find(|&j| app.settings.camera.points[j].image.is_none());
                }
                let painter = ui.painter_at(r);
                for (i, p) in app.settings.camera.points.iter().enumerate() {
                    if let Some([u, v]) = p.image {
                        let c = r.min + vec2(u as f32 * r.width(), v as f32 * r.height());
                        let col = if app.camera.picking == Some(i) { pal.warning } else { pal.accent };
                        painter.circle_stroke(c, 7.0, Stroke::new(2.0, col));
                        painter.line_segment([c - vec2(11.0, 0.0), c + vec2(11.0, 0.0)], Stroke::new(1.0, col));
                        painter.line_segment([c - vec2(0.0, 11.0), c + vec2(0.0, 11.0)], Stroke::new(1.0, col));
                        painter.text(c + vec2(10.0, -10.0), Align2::LEFT_BOTTOM, format!("{}", i + 1), egui::FontId::proportional(14.0), col);
                    }
                }
            } else {
                ui.label(RichText::new(tr("Noch kein Bild – „Aufnehmen“, „Live“ oder „Foto laden…“.")).color(pal.text_weak));
            }
            ui.add_space(8.0);

            // Ausrichten
            ui.label(RichText::new(tr("Ausrichten")).strong());
            ui.label(
                RichText::new(
                    tr("Vier Punkte, deren Lage auf dem Bett bekannt ist (z. B. gelaserte Ausrichtmarken), jeweils \
                     im Bild anklicken. Die Kamera muss danach fest montiert bleiben."),
                )
                .small()
                .color(pal.text_weak),
            );
            egui::Grid::new("camera_points").num_columns(5).spacing(vec2(10.0, 4.0)).show(ui, |ui| {
                for i in 0..4 {
                    let bed = app.settings.camera.points[i].bed;
                    ui.label(format!("{} ({})", i + 1, place_label(app, bed)));
                    let p = &mut app.settings.camera.points[i];
                    ui.add(DragValue::new(&mut p.bed[0]).speed(0.5).range(-10_000.0..=10_000.0).prefix("X ").suffix(" mm"));
                    ui.add(DragValue::new(&mut p.bed[1]).speed(0.5).range(-10_000.0..=10_000.0).prefix("Y ").suffix(" mm"));
                    let picking = app.camera.picking == Some(i);
                    let label = if picking { tr("Klick ins Bild…") } else { tr("Im Bild setzen") };
                    if ui.add_enabled(app.camera.frame.is_some(), Button::selectable(picking, label)).clicked() {
                        app.camera.picking = if picking { None } else { Some(i) };
                    }
                    let set = app.settings.camera.points[i].image.is_some();
                    ui.label(RichText::new(if set { icon::CHECK_CIRCLE } else { icon::CIRCLE_DASHED }).color(if set { pal.success } else { pal.text_weak }));
                    ui.end_row();
                }
            });
            ui.horizontal(|ui| {
                let connected = app.machine.connected && !app.machine.job_running;
                if ui
                    .add_enabled(connected, Button::new(trs(&format!("{} Ausrichtmarken lasern", icon::CROSSHAIR))))
                    .on_hover_text(tr("Lasert an den vier Punkten je ein kleines Kreuz (auf Papier oder Abfallholz)"))
                    .on_disabled_hover_text(tr("Dafür muss ein Laser verbunden sein."))
                    .clicked()
                {
                    action = Some("marks");
                }
                ui.add(DragValue::new(&mut app.settings.camera.mark_power).speed(0.5).range(1.0..=100.0).suffix(" %"));
                ui.add(DragValue::new(&mut app.settings.camera.mark_speed).speed(10.0).range(60.0..=30_000.0).suffix(" mm/min"));
                if ui.button(tr("Punkte zurücksetzen")).clicked() {
                    for p in &mut app.settings.camera.points {
                        p.image = None;
                    }
                    app.camera.picking = Some(0);
                }
            });
            let cs = &app.settings.camera;
            let status = if !cs.calibrated() {
                let missing = cs.points.iter().filter(|p| p.image.is_none()).count();
                RichText::new(trs(&format!("Noch {missing} Punkt(e) im Bild setzen."))).color(pal.text_weak)
            } else if homography(cs, &app.settings.profile).is_none() {
                RichText::new(tr("Die Punkte ergeben keine gültige Ausrichtung (liegen drei auf einer Linie?).")).color(pal.danger)
            } else {
                RichText::new(trs(&format!("{} Ausgerichtet", icon::CHECK))).color(pal.success)
            };
            ui.label(status);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut app.settings.camera.overlay, tr("Auf der Arbeitsfläche anzeigen"));
                ui.label(tr("Deckkraft"));
                ui.add(egui::Slider::new(&mut app.settings.camera.opacity, 0.1..=1.0).show_value(false));
            });
            ui.label(
                RichText::new(tr("Hinweis: Eine starke Objektivverzerrung (Fischauge) wird nicht ausgeglichen – am besten eine Kamera mit normalem Objektiv senkrecht über dem Bett verwenden."))
                    .small()
                    .color(pal.text_weak),
            );
        });

    match action {
        Some(a @ ("snap" | "live")) => {
            let devices = app.camera.devices.clone().unwrap_or_default();
            let dev = devices.iter().find(|d| d.1 == app.settings.camera.device).or(devices.first()).cloned();
            if let Some((index, name)) = dev {
                app.settings.camera.device = name;
                app.camera.capture = None;
                app.camera.capture = Some(start_capture(index, a == "live", ctx.clone()));
            }
        }
        Some("stop") => app.camera.capture = None,
        Some("photo") => load_photo(app),
        Some("marks") => laser_marks(app),
        _ => {}
    }
    if !open {
        app.camera.open = false;
        app.camera.picking = None;
        // Live-Bild nur weiterlaufen lassen, wenn es auf der Arbeitsfläche angezeigt wird
        if !app.settings.camera.overlay {
            app.camera.capture = None;
        }
    }
    if app.settings.camera != before {
        app.settings.save();
    }
}

fn load_photo(app: &mut VectorLumeApp) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter(tr("Bilder"), &["png", "jpg", "jpeg", "bmp", "webp", "tif", "tiff"])
        .pick_file()
    else {
        return;
    };
    match image::open(&path) {
        Ok(img) => {
            let rgb = img.to_rgb8();
            app.camera.capture = None;
            app.camera.set_frame(Frame {
                width: rgb.width() as usize,
                height: rgb.height() as usize,
                rgb: Arc::new(rgb.into_raw()),
            });
        }
        Err(e) => app.toast(ToastKind::Error, trs(&format!("Foto konnte nicht geladen werden: {e}"))),
    }
}

/// Lasert ein Kreuz an jedem Passpunkt.
fn laser_marks(app: &mut VectorLumeApp) {
    if app.settings.profile.rotary.enabled {
        app.toast(ToastKind::Error, tr("Für die Ausrichtmarken bitte zuerst die Drehachse ausschalten."));
        return;
    }
    let profile = app.settings.profile.clone();
    let mut doc = Document::new();
    let cs = &app.settings.camera;
    doc.layers[0].settings = vl_core::CutSettings {
        speed: cs.mark_speed,
        power_max: cs.mark_power,
        air_assist: false,
        ..vl_core::CutSettings::default_for(LayerMode::Line)
    };
    let h = MARK_SIZE / 2.0;
    for (i, p) in cs.points.iter().enumerate() {
        let c = profile.machine_to_doc(Point::new(p.bed[0], p.bed[1]));
        let mut cross = VectorPath::line(Point::new(c.x - h, c.y), Point::new(c.x + h, c.y));
        cross.append(VectorPath::line(Point::new(c.x, c.y - h), Point::new(c.x, c.y + h)));
        doc.add_path(trs(&format!("Marke {}", i + 1)), 0, cross);
    }
    let settings = vl_cam::JobSettings { start_from: vl_cam::StartFrom::Absolute, ..Default::default() };
    match vl_cam::build_job(&doc, &[], &profile, &settings) {
        Ok(job) => {
            if app.ready_to_start() {
                app.launch_job(Arc::new(job));
            }
        }
        Err(e) => app.toast(ToastKind::Error, e),
    }
}
