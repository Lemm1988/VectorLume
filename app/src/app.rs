//! Zentraler Zustand der Anwendung und alle Aktionen (Dateien, Bearbeiten, Jobs, Gerät).

use crate::canvas::{GeomCache, TextureCache, View};
use crate::history::History;
use crate::settings::{AppSettings, ThemeChoice};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use vl_cam::{Job, StartFrom, strip_for_streaming};
use vl_core::{Affine, Document, Object, ObjectId, Point, Rect};
use vl_device::{Command, Device, Direction, Event, Status};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Rect,
    Ellipse,
    Line,
    Polygon,
    Star,
    Text,
}

impl Tool {
    pub const ALL: [Tool; 7] =
        [Tool::Select, Tool::Rect, Tool::Ellipse, Tool::Line, Tool::Polygon, Tool::Star, Tool::Text];

    pub fn icon(self) -> &'static str {
        use egui_phosphor::regular as i;
        match self {
            Tool::Select => i::CURSOR,
            Tool::Rect => i::SQUARE,
            Tool::Ellipse => i::CIRCLE,
            Tool::Line => i::LINE_SEGMENT,
            Tool::Polygon => i::POLYGON,
            Tool::Star => i::STAR,
            Tool::Text => i::TEXT_T,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Auswählen (V)",
            Tool::Rect => "Rechteck (R)",
            Tool::Ellipse => "Ellipse (E)",
            Tool::Line => "Linie (L)",
            Tool::Polygon => "Vieleck",
            Tool::Star => "Stern",
            Tool::Text => "Text (T)",
        }
    }
}

pub struct ToolOptions {
    pub corner_radius: f64,
    pub polygon_sides: usize,
    pub star_points: usize,
    pub star_inner: f64,
}

impl Default for ToolOptions {
    fn default() -> Self {
        Self { corner_radius: 0.0, polygon_sides: 6, star_points: 5, star_inner: 0.45 }
    }
}

#[derive(Clone, Debug)]
pub enum JobPurpose {
    Preview,
    Start,
    Frame,
    Export(PathBuf),
}

pub struct JobWorker {
    tx: Sender<(u64, JobPurpose, Result<Job, String>)>,
    rx: Receiver<(u64, JobPurpose, Result<Job, String>)>,
    next_id: u64,
    pub busy: usize,
    preview_in_flight: bool,
}

#[derive(Default)]
pub struct PreviewState {
    pub active: bool,
    pub job: Option<Arc<Job>>,
    pub built_revision: u64,
    pub requested_revision: Option<u64>,
    pub error: Option<String>,
    /// Abspiel-Fortschritt 0…1.
    pub progress: f32,
    pub texture_generation: u64,
}

#[derive(Default)]
pub struct MachineInfo {
    pub connected: bool,
    pub port: Option<String>,
    pub status: Option<Status>,
    pub last_wco: Option<[f64; 3]>,
    pub settings: BTreeMap<u32, String>,
    pub job_running: bool,
    pub job_name: String,
    pub job_progress: (usize, usize),
    pub job_started: Option<Instant>,
    pub job_estimate: f64,
}

impl MachineInfo {
    /// Aktuelle Kopfposition in Arbeitskoordinaten.
    pub fn head_position(&self) -> Option<Point> {
        let st = self.status.as_ref()?;
        let p = st.work_position(self.last_wco)?;
        Some(Point::new(p[0], p[1]))
    }

    pub fn setting_f64(&self, n: u32) -> Option<f64> {
        self.settings.get(&n).and_then(|v| v.trim().parse().ok())
    }

    pub fn laser_mode(&self) -> Option<bool> {
        self.setting_f64(32).map(|v| v == 1.0)
    }
}

pub struct ConsoleLine {
    pub dir: Direction,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub until: Instant,
}

/// Was nach dem Bestätigen „Änderungen verwerfen?“ passieren soll.
#[derive(Clone, Debug)]
pub enum AfterDiscard {
    New,
    Demo,
    Open(Option<PathBuf>),
    Close,
}

pub enum Modal {
    ConfirmStart { job: Arc<Job> },
    LaserModeOff,
    Safety { job: Arc<Job> },
    Unsaved(AfterDiscard),
    Message { title: String, text: String },
}

pub struct TextDialog {
    pub open: bool,
    pub spec: vl_io::TextSpec,
    pub at: Point,
    pub filter: String,
    pub error: Option<String>,
}

pub struct VectorLumeApp {
    pub ctx: egui::Context,
    pub doc: Document,
    pub history: History,
    pub selection: Vec<ObjectId>,
    pub current_layer: usize,
    pub tool: Tool,
    pub tool_options: ToolOptions,
    pub view: View,
    pub settings: AppSettings,
    pub device: Device,
    pub machine: MachineInfo,
    pub ports: Vec<vl_device::PortInfo>,
    pub selected_port: String,
    pub console: Vec<ConsoleLine>,
    pub console_input: String,
    pub file_path: Option<PathBuf>,
    pub dirty: bool,
    pub revision: u64,
    pub clipboard: Vec<Object>,
    pub preview: PreviewState,
    pub jobs: JobWorker,
    pub toasts: Vec<Toast>,
    pub geom: GeomCache,
    pub textures: TextureCache,
    pub drag: crate::canvas::Drag,
    pub modal: Option<Modal>,
    pub text_dialog: TextDialog,
    pub font_families: Arc<Mutex<Option<Vec<String>>>>,
    pub show_device_settings: bool,
    pub show_about: bool,
    pub right_tab: crate::ui_side::RightTab,
    pub lock_ratio: bool,
    pub rotate_by: f64,
    pub materials: crate::materials::MaterialsUi,
    pub test: crate::materials::TestUi,
    last_edit: Option<(&'static str, Instant)>,
    last_title: String,
    close_confirmed: bool,
}

impl VectorLumeApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::theme::install(&cc.egui_ctx);
        let settings = AppSettings::load();
        apply_theme(&cc.egui_ctx, settings.theme);
        let ctx = cc.egui_ctx.clone();
        let wake = ctx.clone();
        let device = Device::spawn(move || wake.request_repaint());
        let ports = vl_device::available_ports();
        let selected_port = if ports.iter().any(|p| p.name == settings.last_port) {
            settings.last_port.clone()
        } else {
            ports.first().map(|p| p.name.clone()).unwrap_or_default()
        };
        // Systemschriften im Hintergrund laden (für Text-Werkzeug und SVG-Text).
        let font_families = Arc::new(Mutex::new(None));
        {
            let fams = font_families.clone();
            let wake = ctx.clone();
            std::thread::spawn(move || {
                let db = vl_io::system_fonts();
                *fams.lock().unwrap() = Some(vl_io::font_families(&db));
                wake.request_repaint();
            });
        }
        let (tx, rx) = channel();
        let mut app = Self {
            ctx,
            doc: Document::new(),
            history: History::default(),
            selection: Vec::new(),
            current_layer: 0,
            tool: Tool::Select,
            tool_options: ToolOptions::default(),
            view: View::default(),
            settings,
            device,
            machine: MachineInfo::default(),
            ports,
            selected_port,
            console: Vec::new(),
            console_input: String::new(),
            file_path: None,
            dirty: false,
            revision: 1,
            clipboard: Vec::new(),
            preview: PreviewState { progress: 1.0, ..Default::default() },
            jobs: JobWorker { tx, rx, next_id: 1, busy: 0, preview_in_flight: false },
            toasts: Vec::new(),
            geom: GeomCache::default(),
            textures: TextureCache::default(),
            drag: crate::canvas::Drag::None,
            modal: None,
            text_dialog: TextDialog {
                open: false,
                spec: vl_io::TextSpec::default(),
                at: Point::new(20.0, 20.0),
                filter: String::new(),
                error: None,
            },
            font_families,
            show_device_settings: false,
            show_about: false,
            right_tab: crate::ui_side::RightTab::Laser,
            lock_ratio: true,
            rotate_by: 90.0,
            materials: crate::materials::MaterialsUi::load(),
            test: crate::materials::TestUi::default(),
            last_edit: None,
            last_title: String::new(),
            close_confirmed: false,
        };
        // Dateien aus der Befehlszeile öffnen bzw. importieren („Öffnen mit …“).
        for arg in std::env::args_os().skip(1) {
            let path = PathBuf::from(arg);
            if path.is_file() {
                app.import_path(&path);
            }
        }
        if !app.doc.objects.is_empty() {
            app.zoom_to_fit();
        }
        // Schalter für Demos und automatische Oberflächentests.
        if std::env::var_os("VECTORLUME_DEMO").is_some() {
            app.load_demo();
        }
        if std::env::var_os("VECTORLUME_PREVIEW").is_some() {
            app.preview.active = true;
        }
        if std::env::var_os("VECTORLUME_SIMULATOR").is_some() {
            app.selected_port = vl_device::SIMULATOR_PORT.into();
            app.connect();
        }
        match std::env::var("VECTORLUME_WINDOW").as_deref() {
            Ok("materialtest") => app.open_material_test(),
            Ok("materials") => app.materials.open = true,
            _ => {}
        }
        app
    }

    // ---------------------------------------------------------------- Meldungen

    pub fn toast(&mut self, kind: ToastKind, text: impl Into<String>) {
        let secs = if kind == ToastKind::Error { 7 } else { 4 };
        self.toasts.push(Toast { text: text.into(), kind, until: Instant::now() + Duration::from_secs(secs) });
        if self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
    }

    fn console_push(&mut self, dir: Direction, text: String) {
        self.console.push(ConsoleLine { dir, text });
        if self.console.len() > 3000 {
            self.console.drain(..500);
        }
    }

    // ---------------------------------------------------------------- Änderungen & Verlauf

    /// Vor jeder Änderung am Dokument aufrufen.
    pub fn checkpoint(&mut self) {
        self.history.checkpoint(&self.doc);
        self.last_edit = None;
    }

    /// Wie `checkpoint`, fasst aber schnelle Folgeänderungen am selben Feld zu einem Schritt zusammen.
    pub fn checkpoint_coalesced(&mut self, key: &'static str) {
        let now = Instant::now();
        let merge =
            matches!(self.last_edit, Some((k, t)) if k == key && now.duration_since(t) < Duration::from_millis(1200));
        if !merge {
            self.history.checkpoint(&self.doc);
        }
        self.last_edit = Some((key, now));
    }

    pub fn mark_changed(&mut self) {
        self.dirty = true;
        self.revision += 1;
    }

    pub fn undo(&mut self) {
        if self.history.undo(&mut self.doc) {
            self.after_history_jump();
        }
    }

    pub fn redo(&mut self) {
        if self.history.redo(&mut self.doc) {
            self.after_history_jump();
        }
    }

    fn after_history_jump(&mut self) {
        self.selection.retain(|id| self.doc.get(*id).is_some());
        self.geom.clear();
        self.last_edit = None;
        self.mark_changed();
    }

    // ---------------------------------------------------------------- Dateien

    pub fn confirm_discard(&mut self, then: AfterDiscard) -> bool {
        if self.dirty && !self.doc.objects.is_empty() {
            self.modal = Some(Modal::Unsaved(then));
            return false;
        }
        true
    }

    pub fn new_document(&mut self, force: bool) {
        if !force && !self.confirm_discard(AfterDiscard::New) {
            return;
        }
        self.doc = Document::new();
        self.reset_after_load(None);
    }

    fn reset_after_load(&mut self, path: Option<PathBuf>) {
        self.history.clear();
        self.selection.clear();
        self.geom.clear();
        self.file_path = path;
        self.dirty = false;
        self.revision += 1;
        self.current_layer = self.doc.used_layers().first().copied().unwrap_or(0);
    }

    pub fn open_dialog(&mut self, force: bool, path: Option<PathBuf>) {
        if !force && !self.confirm_discard(AfterDiscard::Open(path.clone())) {
            return;
        }
        let path = path.or_else(|| {
            rfd::FileDialog::new()
                .set_title("Projekt öffnen")
                .add_filter("VectorLume-Projekt", &[vl_core::project::PROJECT_EXTENSION])
                .pick_file()
        });
        if let Some(p) = path {
            self.open_project(&p);
        }
    }

    pub fn open_project(&mut self, path: &Path) {
        match vl_core::project::load_project(path) {
            Ok(doc) => {
                self.doc = doc;
                self.reset_after_load(Some(path.to_path_buf()));
                self.settings.add_recent(path.to_path_buf());
                self.settings.save();
                self.zoom_to_fit();
                self.toast(ToastKind::Success, format!("„{}“ geöffnet.", file_name(path)));
            }
            Err(e) => self.toast(ToastKind::Error, e),
        }
    }

    pub fn save(&mut self, save_as: bool) -> bool {
        let path = match (&self.file_path, save_as) {
            (Some(p), false) => Some(p.clone()),
            _ => rfd::FileDialog::new()
                .set_title("Projekt speichern")
                .set_file_name("Projekt.vlume")
                .add_filter("VectorLume-Projekt", &[vl_core::project::PROJECT_EXTENSION])
                .save_file(),
        };
        let Some(mut path) = path else { return false };
        if path.extension().is_none() {
            path.set_extension(vl_core::project::PROJECT_EXTENSION);
        }
        match vl_core::project::save_project(&self.doc, &path) {
            Ok(()) => {
                self.dirty = false;
                self.settings.add_recent(path.clone());
                self.settings.save();
                self.toast(ToastKind::Success, format!("Gespeichert: {}", file_name(&path)));
                self.file_path = Some(path);
                true
            }
            Err(e) => {
                self.toast(ToastKind::Error, e);
                false
            }
        }
    }

    pub fn import_dialog(&mut self) {
        let mut all: Vec<&str> = vl_io::VECTOR_EXTENSIONS.to_vec();
        all.extend_from_slice(vl_io::IMAGE_EXTENSIONS);
        if let Some(files) = rfd::FileDialog::new()
            .set_title("Importieren")
            .add_filter("Alle unterstützten Dateien", &all)
            .add_filter("Vektorgrafik (SVG, DXF)", vl_io::VECTOR_EXTENSIONS)
            .add_filter("Bilder", vl_io::IMAGE_EXTENSIONS)
            .pick_files()
        {
            for f in files {
                self.import_path(&f);
            }
        }
    }

    pub fn import_path(&mut self, path: &Path) {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == vl_core::project::PROJECT_EXTENSION {
            self.open_dialog(false, Some(path.to_path_buf()));
            return;
        }
        let fonts = (ext == "svg" || ext == "svgz").then(vl_io::system_fonts);
        match vl_io::import_file(path, fonts) {
            Ok(imported) => {
                let warnings = imported.warnings.clone();
                self.checkpoint();
                let ids = vl_io::add_to_document(&mut self.doc, imported, self.current_layer);
                if ids.is_empty() {
                    self.history.discard_if_unchanged(&self.doc);
                } else {
                    self.place_imported(&ids);
                    self.selection = ids;
                    self.mark_changed();
                    self.toast(ToastKind::Success, format!("„{}“ importiert.", file_name(path)));
                }
                for w in warnings {
                    self.toast(ToastKind::Info, w);
                }
            }
            Err(e) => self.toast(ToastKind::Error, e),
        }
    }

    /// Importiertes in die Arbeitsfläche legen, falls es (teilweise) außerhalb liegt.
    fn place_imported(&mut self, ids: &[ObjectId]) {
        let b = self.doc.bounds_of(ids);
        let bed = self.settings.profile.bed_rect();
        if b.is_empty() || bed.contains_rect(&b) {
            return;
        }
        let target = if b.width() <= bed.width() && b.height() <= bed.height() {
            // Möglichst nah an der ursprünglichen Lage, aber vollständig in der Fläche.
            Point::new(b.min.x.clamp(0.0, bed.width() - b.width()), b.min.y.clamp(0.0, bed.height() - b.height()))
        } else {
            bed.center() - Point::new(b.width() / 2.0, b.height() / 2.0)
        };
        self.doc.transform_objects(ids, &Affine::translate(target.x - b.min.x, target.y - b.min.y));
    }

    pub fn export_gcode_dialog(&mut self) {
        let name =
            self.file_path.as_ref().and_then(|p| p.file_stem()).and_then(|s| s.to_str()).unwrap_or("Job").to_string();
        if let Some(path) = rfd::FileDialog::new()
            .set_title("G-Code speichern")
            .set_file_name(format!("{name}.gcode"))
            .add_filter("G-Code", &["gcode", "nc", "gc"])
            .save_file()
        {
            self.request_job(JobPurpose::Export(path));
        }
    }

    pub fn export_svg_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Als SVG exportieren")
            .set_file_name("Export.svg")
            .add_filter("SVG", &["svg"])
            .save_file()
        {
            let p = &self.settings.profile;
            let svg = vl_io::svg::export_svg(&self.doc, p.bed_width, p.bed_height);
            match std::fs::write(&path, svg) {
                Ok(()) => self.toast(ToastKind::Success, format!("SVG gespeichert: {}", file_name(&path))),
                Err(e) => self.toast(ToastKind::Error, format!("SVG konnte nicht gespeichert werden: {e}")),
            }
        }
    }

    // ---------------------------------------------------------------- Bearbeiten

    pub fn select_all(&mut self) {
        self.selection = self.doc.objects.iter().map(|o| o.id).collect();
    }

    pub fn delete_selection(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        self.checkpoint();
        let sel = std::mem::take(&mut self.selection);
        self.doc.remove(&sel);
        self.mark_changed();
    }

    fn selected_objects(&self) -> Vec<Object> {
        self.doc.objects.iter().filter(|o| self.selection.contains(&o.id)).cloned().collect()
    }

    pub fn copy(&mut self) {
        self.clipboard = self.selected_objects();
    }

    pub fn cut(&mut self) {
        self.copy();
        self.delete_selection();
    }

    pub fn paste(&mut self) {
        if self.clipboard.is_empty() {
            return;
        }
        self.checkpoint();
        let objs = self.clipboard.clone();
        self.selection = self.doc.duplicate(&objs, Point::new(5.0, 5.0));
        // Beim nächsten Einfügen weiter versetzen.
        for o in &mut self.clipboard {
            o.transform = o.transform.then(&Affine::translate(5.0, 5.0));
        }
        self.mark_changed();
    }

    pub fn duplicate(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        self.checkpoint();
        let objs = self.selected_objects();
        self.selection = self.doc.duplicate(&objs, Point::new(5.0, 5.0));
        self.mark_changed();
    }

    pub fn group(&mut self) {
        if self.selection.len() < 2 {
            return;
        }
        self.checkpoint();
        self.doc.group(&self.selection);
        self.mark_changed();
    }

    pub fn ungroup(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        self.checkpoint();
        self.doc.ungroup(&self.selection);
        self.mark_changed();
    }

    pub fn selection_bounds(&self) -> Rect {
        self.doc.bounds_of(&self.selection)
    }

    pub fn transform_selection(&mut self, t: &Affine) {
        if self.selection.is_empty() {
            return;
        }
        self.checkpoint();
        self.doc.transform_objects(&self.selection, t);
        self.mark_changed();
    }

    pub fn mirror(&mut self, horizontal: bool) {
        let c = self.selection_bounds().center();
        let (sx, sy) = if horizontal { (-1.0, 1.0) } else { (1.0, -1.0) };
        self.transform_selection(&Affine::scale_about(sx, sy, c));
    }

    pub fn rotate_selection(&mut self, degrees: f64) {
        let c = self.selection_bounds().center();
        self.transform_selection(&Affine::rotate_about(degrees.to_radians(), c));
    }

    pub fn nudge(&mut self, dx: f64, dy: f64) {
        if self.selection.is_empty() {
            return;
        }
        self.checkpoint_coalesced("nudge");
        self.doc.transform_objects(&self.selection, &Affine::translate(dx, dy));
        self.mark_changed();
    }

    /// Richtet jedes ausgewählte Objekt an der Gesamtumrandung aus (`mode`: 0 links/oben, 1 Mitte, 2 rechts/unten).
    pub fn align(&mut self, horizontal: bool, mode: u8) {
        if self.selection.len() < 2 {
            return;
        }
        let all = self.selection_bounds();
        self.checkpoint();
        let units = self.selection_units();
        for unit in units {
            let b = self.doc.bounds_of(&unit);
            let d = if horizontal {
                match mode {
                    0 => all.min.x - b.min.x,
                    1 => all.center().x - b.center().x,
                    _ => all.max.x - b.max.x,
                }
            } else {
                match mode {
                    0 => all.min.y - b.min.y,
                    1 => all.center().y - b.center().y,
                    _ => all.max.y - b.max.y,
                }
            };
            let t = if horizontal { Affine::translate(d, 0.0) } else { Affine::translate(0.0, d) };
            self.doc.transform_objects(&unit, &t);
        }
        self.mark_changed();
    }

    /// Verteilt die Auswahl gleichmäßig (Mittelpunkte).
    pub fn distribute(&mut self, horizontal: bool) {
        let mut units: Vec<(f64, Vec<ObjectId>)> = self
            .selection_units()
            .into_iter()
            .map(|u| {
                let c = self.doc.bounds_of(&u).center();
                (if horizontal { c.x } else { c.y }, u)
            })
            .collect();
        if units.len() < 3 {
            return;
        }
        units.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (first, last) = (units[0].0, units[units.len() - 1].0);
        let step = (last - first) / (units.len() - 1) as f64;
        self.checkpoint();
        for (i, (pos, ids)) in units.iter().enumerate() {
            let d = first + step * i as f64 - pos;
            let t = if horizontal { Affine::translate(d, 0.0) } else { Affine::translate(0.0, d) };
            self.doc.transform_objects(ids, &t);
        }
        self.mark_changed();
    }

    /// Auswahl aufgeteilt in Gruppen bzw. Einzelobjekte.
    fn selection_units(&self) -> Vec<Vec<ObjectId>> {
        let mut units: Vec<Vec<ObjectId>> = Vec::new();
        let mut groups: Vec<(u64, usize)> = Vec::new();
        for o in self.doc.objects.iter().filter(|o| self.selection.contains(&o.id)) {
            match o.group {
                Some(g) => match groups.iter().find(|(gid, _)| *gid == g) {
                    Some(&(_, idx)) => units[idx].push(o.id),
                    None => {
                        groups.push((g, units.len()));
                        units.push(vec![o.id]);
                    }
                },
                None => units.push(vec![o.id]),
            }
        }
        units
    }

    pub fn center_on_bed(&mut self) {
        let b = self.selection_bounds();
        if b.is_empty() {
            return;
        }
        let c = self.settings.profile.bed_rect().center();
        let d = c - b.center();
        self.transform_selection(&Affine::translate(d.x, d.y));
    }

    pub fn assign_layer(&mut self, layer: usize) {
        self.current_layer = layer;
        if self.selection.is_empty() {
            return;
        }
        self.checkpoint();
        let images_only = self.doc.objects.iter().filter(|o| self.selection.contains(&o.id)).all(|o| o.is_image());
        if images_only && self.doc.objects_on_layer(layer).next().is_none() {
            self.doc.layers[layer].settings = vl_core::CutSettings::default_for(vl_core::LayerMode::Image);
        }
        for o in self.doc.objects.iter_mut().filter(|o| self.selection.contains(&o.id)) {
            o.layer = layer;
        }
        self.mark_changed();
    }

    pub fn zoom_to_fit(&mut self) {
        self.view.request_fit(None);
    }

    /// Beispielprojekt mit allen Ebenenarten (Schneiden, Füllen, Text, Bild).
    pub fn load_demo(&mut self) {
        use vl_core::{CutSettings, DitherMode, ImageAdjust, ImageObject, LayerMode, RasterImage, VectorPath};
        let mut doc = Document::new();
        doc.layers[0].settings =
            CutSettings { speed: 600.0, power_max: 85.0, ..CutSettings::default_for(LayerMode::Line) };
        doc.layers[0].name = "Schneiden".into();
        let frame = doc.add_path("Rahmen", 0, VectorPath::rect(30.0, 30.0, 170.0, 120.0, 8.0));
        let hole = doc.add_path("Loch", 0, VectorPath::ellipse(Point::new(180.0, 50.0), 8.0, 8.0));
        doc.group(&[frame, hole]);
        doc.layers[1].settings = CutSettings { fill_angle: 45.0, ..CutSettings::default_for(LayerMode::Fill) };
        doc.layers[1].name = "Gravur".into();
        doc.add_path("Stern", 1, VectorPath::star(Point::new(80.0, 80.0), 36.0, 15.0, 5));
        let db = vl_io::system_fonts();
        let family = ["Segoe UI", "Arial"]
            .iter()
            .map(|s| s.to_string())
            .find(|f| vl_io::font_families(&db).contains(f))
            .or_else(|| vl_io::text::any_family(&db));
        if let Some(family) = family {
            let spec = vl_io::TextSpec {
                text: "VectorLume".into(),
                family,
                bold: true,
                height_mm: 11.0,
                ..Default::default()
            };
            if let Ok(p) = vl_io::text_to_path(&db, &spec) {
                doc.add_path("Text „VectorLume“", 1, p.transformed(&Affine::translate(45.0, 126.0)));
            }
        }
        // Verlaufsbild erzeugen (kreisförmiger Verlauf)
        let (w, h) = (360u32, 240u32);
        let px = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| {
                let (dx, dy) = (x as f64 - 180.0, y as f64 - 120.0);
                let r = (dx * dx + dy * dy).sqrt() / 150.0;
                let wave = 0.5 + 0.5 * (x as f64 / 18.0).sin() * (y as f64 / 22.0).cos();
                ((r.min(1.0) * 0.7 + wave * 0.3) * 255.0) as u8
            })
            .collect();
        let img = ImageObject {
            image: Arc::new(RasterImage::new(w, h, px)),
            width_mm: 72.0,
            height_mm: 48.0,
            adjust: ImageAdjust::default(),
        };
        doc.add_image("Verlauf", 3, img, Point::new(220.0, 40.0));
        doc.layers[3].settings.dither = DitherMode::Jarvis;
        doc.layers[3].name = "Foto".into();
        self.doc = doc;
        self.reset_after_load(None);
        self.dirty = true;
        self.zoom_to_fit();
    }

    // ---------------------------------------------------------------- Jobs

    pub fn request_job(&mut self, purpose: JobPurpose) {
        let doc = self.doc.clone();
        let sel = self.selection.clone();
        let profile = self.settings.profile.clone();
        let js = self.settings.job.clone();
        let id = self.jobs.next_id;
        self.jobs.next_id += 1;
        if matches!(purpose, JobPurpose::Preview) {
            self.jobs.preview_in_flight = true;
            self.preview.requested_revision = Some(self.revision);
        }
        self.jobs.busy += 1;
        let tx = self.jobs.tx.clone();
        let wake = self.ctx.clone();
        std::thread::spawn(move || {
            let result = vl_cam::build_job(&doc, &sel, &profile, &js);
            let _ = tx.send((id, purpose, result));
            wake.request_repaint();
        });
    }

    fn poll_jobs(&mut self) {
        while let Ok((_id, purpose, result)) = self.jobs.rx.try_recv() {
            self.jobs.busy = self.jobs.busy.saturating_sub(1);
            match purpose {
                JobPurpose::Preview => {
                    self.jobs.preview_in_flight = false;
                    self.preview.built_revision = self.preview.requested_revision.unwrap_or(self.revision);
                    self.preview.requested_revision = None;
                    match result {
                        Ok(job) => {
                            self.preview.job = Some(Arc::new(job));
                            self.preview.error = None;
                        }
                        Err(e) => {
                            self.preview.job = None;
                            self.preview.error = Some(e);
                        }
                    }
                    self.preview.texture_generation += 1;
                }
                JobPurpose::Start => match result {
                    Ok(job) => {
                        let job = Arc::new(job);
                        self.preview.job = Some(job.clone());
                        self.preview.built_revision = self.revision;
                        self.preview.texture_generation += 1;
                        self.launch_job(job);
                    }
                    Err(e) => self.toast(ToastKind::Error, e),
                },
                JobPurpose::Frame => match result {
                    Ok(job) => self.run_frame(&job),
                    Err(e) => self.toast(ToastKind::Error, e),
                },
                JobPurpose::Export(path) => match result {
                    Ok(job) => self.write_gcode(&job, &path),
                    Err(e) => self.toast(ToastKind::Error, e),
                },
            }
        }
    }

    /// Startet einen fertigen Job nach Sicherheitshinweis bzw. Bestätigung der Warnungen.
    pub fn launch_job(&mut self, job: Arc<Job>) {
        if !self.settings.safety_acknowledged {
            self.modal = Some(Modal::Safety { job });
        } else if !job.warnings.is_empty() {
            self.modal = Some(Modal::ConfirmStart { job });
        } else {
            self.send_job(&job);
        }
    }

    pub fn run_frame(&mut self, job: &Job) {
        let lines = strip_for_streaming(&vl_cam::frame_gcode(job, &self.settings.profile));
        self.device.send(Command::StartJob { name: "Rahmen".into(), lines });
    }

    pub fn write_gcode(&mut self, job: &Job, path: &Path) {
        let mut text = job.gcode.join("\n");
        text.push('\n');
        match std::fs::write(path, text) {
            Ok(()) => {
                let mut msg =
                    format!("G-Code gespeichert: {} (geschätzt {})", file_name(path), job.estimate.format_duration());
                if !job.warnings.is_empty() {
                    msg.push_str(" – mit Warnungen");
                }
                self.toast(ToastKind::Success, msg);
                for w in &job.warnings {
                    self.toast(ToastKind::Info, w.clone());
                }
            }
            Err(e) => self.toast(ToastKind::Error, format!("Speichern fehlgeschlagen: {e}")),
        }
    }

    /// Prüft, ob ein Job gestartet werden darf, und meldet sonst den Grund.
    pub fn ready_to_start(&mut self) -> bool {
        if !self.machine.connected {
            self.toast(ToastKind::Error, "Kein Laser verbunden.");
            return false;
        }
        if self.machine.job_running {
            self.toast(ToastKind::Info, "Es läuft bereits ein Job.");
            return false;
        }
        if self.machine.laser_mode() == Some(false) {
            self.modal = Some(Modal::LaserModeOff);
            return false;
        }
        true
    }

    pub fn ready_to_frame(&mut self) -> bool {
        if !self.machine.connected {
            self.toast(ToastKind::Error, "Kein Laser verbunden.");
            return false;
        }
        if self.machine.job_running {
            return false;
        }
        if self.settings.profile.frame_power > 0.0 && self.machine.laser_mode() == Some(false) {
            self.modal = Some(Modal::LaserModeOff);
            return false;
        }
        true
    }

    pub fn start_job(&mut self) {
        if self.ready_to_start() {
            self.request_job(JobPurpose::Start);
        }
    }

    pub fn frame_job(&mut self) {
        if self.ready_to_frame() {
            self.request_job(JobPurpose::Frame);
        }
    }

    pub fn send_job(&mut self, job: &Job) {
        let lines = strip_for_streaming(&job.gcode);
        self.machine.job_estimate = job.estimate.seconds;
        let name = self.file_path.as_ref().map(|p| file_name(p)).unwrap_or_else(|| "Job".into());
        self.device.send(Command::StartJob { name, lines });
    }

    pub fn update_preview(&mut self) {
        if !self.preview.active {
            return;
        }
        // Es läuft höchstens eine Vorschau-Berechnung; ist sie veraltet, folgt danach die nächste.
        let stale = self.preview.built_revision != self.revision;
        if stale && self.drag.is_idle() && !self.jobs.preview_in_flight {
            self.request_job(JobPurpose::Preview);
        }
    }

    // ---------------------------------------------------------------- Gerät

    pub fn connect(&mut self) {
        let port = self.selected_port.clone();
        if port.is_empty() {
            self.toast(ToastKind::Error, "Kein Anschluss ausgewählt.");
            return;
        }
        self.settings.last_port = port.clone();
        self.settings.save();
        let p = &self.settings.profile;
        self.machine.settings.clear();
        self.device.send(Command::Connect { port, baud: p.baud, rx_buffer: p.rx_buffer });
    }

    pub fn disconnect(&mut self) {
        self.device.send(Command::Disconnect);
    }

    pub fn send_line(&mut self, line: impl Into<String>) {
        self.device.send(Command::Line(line.into()));
    }

    pub fn jog(&mut self, screen_dir: Point) {
        let d = self.settings.profile.screen_to_machine_delta(screen_dir * self.settings.jog_step);
        let f = self.settings.profile.jog_speed.round();
        let mut cmd = String::from("$J=G91 G21");
        if d.x != 0.0 {
            cmd.push_str(&format!(" X{}", vl_cam::gcode::fmt_mm(d.x)));
        }
        if d.y != 0.0 {
            cmd.push_str(&format!(" Y{}", vl_cam::gcode::fmt_mm(d.y)));
        }
        cmd.push_str(&format!(" F{f}"));
        self.send_line(cmd);
    }

    pub fn set_user_origin_here(&mut self) {
        match self.machine.head_position() {
            Some(p) => {
                self.settings.job.user_origin = p;
                self.settings.job.start_from = StartFrom::UserOrigin;
                self.settings.save();
                self.toast(ToastKind::Success, format!("Benutzer-Ursprung gesetzt: X {:.2} Y {:.2}", p.x, p.y));
            }
            None => self.toast(ToastKind::Error, "Position unbekannt – ist der Laser verbunden?"),
        }
    }

    /// Übernimmt Werte aus `$$` ins Maschinenprofil.
    pub fn apply_device_settings(&mut self) {
        let m = &self.machine;
        let p = &mut self.settings.profile;
        let mut changed = Vec::new();
        if let Some(v) = m.setting_f64(30) {
            p.s_max = v;
            changed.push("S-Max");
        }
        if let (Some(x), Some(y)) = (m.setting_f64(130), m.setting_f64(131)) {
            if x > 0.0 && y > 0.0 {
                p.bed_width = x;
                p.bed_height = y;
                changed.push("Arbeitsfläche");
            }
        }
        if let Some(v) = m.setting_f64(110) {
            p.rapid_rate = v;
            changed.push("Eilgang");
        }
        if let Some(v) = m.setting_f64(120) {
            p.acceleration = v;
            changed.push("Beschleunigung");
        }
        self.settings.save();
        self.view.request_fit(None);
        if changed.is_empty() {
            self.toast(ToastKind::Info, "Keine Werte vom Gerät verfügbar – bitte verbinden.");
        } else {
            self.toast(ToastKind::Success, format!("Übernommen: {}", changed.join(", ")));
        }
    }

    fn poll_device(&mut self) {
        for ev in self.device.poll() {
            match ev {
                Event::Connected { port } => {
                    self.machine.connected = true;
                    self.console_push(Direction::Info, format!("Verbunden mit {port}"));
                    self.toast(ToastKind::Success, format!("Verbunden mit {port}"));
                    self.machine.port = Some(port);
                }
                Event::Disconnected { reason } => {
                    let was = self.machine.connected;
                    self.machine.connected = false;
                    self.machine.status = None;
                    self.machine.job_running = false;
                    match reason {
                        Some(r) => {
                            self.console_push(Direction::Info, r.clone());
                            self.toast(ToastKind::Error, r);
                        }
                        None if was => self.toast(ToastKind::Info, "Verbindung getrennt."),
                        None => {}
                    }
                }
                Event::Console { dir, text } => self.console_push(dir, text),
                Event::Status(st) => {
                    if st.wco.is_some() {
                        self.machine.last_wco = st.wco;
                    }
                    self.machine.status = Some(st);
                }
                Event::Setting(n, v) => {
                    let first_laser_mode = n == 32 && !self.machine.settings.contains_key(&32);
                    self.machine.settings.insert(n, v);
                    if first_laser_mode && self.machine.laser_mode() == Some(false) {
                        self.toast(ToastKind::Error, "Achtung: GRBL-Laser-Modus ist aus ($32=0).");
                    }
                }
                Event::JobStarted { name, total } => {
                    self.machine.job_running = true;
                    self.machine.job_name = name;
                    self.machine.job_progress = (0, total);
                    self.machine.job_started = Some(Instant::now());
                }
                Event::JobProgress { acked, total } => self.machine.job_progress = (acked, total),
                Event::JobFinished { ok, message, seconds } => {
                    self.machine.job_running = false;
                    let text = format!("{message} ({})", vl_cam::format_duration(seconds));
                    self.toast(if ok { ToastKind::Success } else { ToastKind::Error }, text);
                }
                Event::Alarm(code, text) => self.toast(ToastKind::Error, format!("Alarm {code}: {text}")),
            }
        }
    }

    // ---------------------------------------------------------------- Rahmen pro Frame

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.close_confirmed {
            if self.machine.job_running {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.modal = Some(Modal::Message {
                    title: "Job läuft".into(),
                    text: "Bitte den laufenden Job erst stoppen oder abwarten, bevor VectorLume beendet wird.".into(),
                });
            } else if self.dirty && !self.doc.objects.is_empty() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.modal = Some(Modal::Unsaved(AfterDiscard::Close));
            } else {
                self.settings.save();
            }
        }
    }

    pub fn after_discard(&mut self, then: AfterDiscard) {
        match then {
            AfterDiscard::New => self.new_document(true),
            AfterDiscard::Demo => self.load_demo(),
            AfterDiscard::Open(p) => self.open_dialog(true, p),
            AfterDiscard::Close => {
                self.close_confirmed = true;
                self.settings.save();
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for p in dropped {
            self.import_path(&p);
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let name = self.file_path.as_ref().map(|p| file_name(p)).unwrap_or_else(|| "Unbenannt".into());
        let title = format!("{}{} – VectorLume", name, if self.dirty { " •" } else { "" });
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }

    pub fn set_theme(&mut self, theme: ThemeChoice) {
        self.settings.theme = theme;
        apply_theme(&self.ctx, theme);
        self.settings.save();
        self.preview.texture_generation += 1;
    }
}

pub fn apply_theme(ctx: &egui::Context, theme: ThemeChoice) {
    ctx.set_theme(match theme {
        ThemeChoice::Dark => egui::ThemePreference::Dark,
        ThemeChoice::Light => egui::ThemePreference::Light,
        ThemeChoice::System => egui::ThemePreference::System,
    });
}

pub fn file_name(p: &Path) -> String {
    p.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string()
}

impl eframe::App for VectorLumeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_device();
        self.poll_jobs();
        self.handle_dropped_files(&ctx);
        self.handle_close_request(&ctx);
        crate::ui_main::shortcuts(self, &ctx);

        crate::ui_main::top_bar(self, ui);
        crate::ui_main::bottom_bar(self, ui);
        crate::ui_main::tool_bar(self, ui);
        crate::ui_side::right_panel(self, ui);
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.canvas_ui(ui));

        crate::dialogs::show(self, &ctx);
        crate::materials::show(self, &ctx);
        self.update_preview();
        self.update_title(&ctx);
        if self.machine.job_running || self.jobs.busy > 0 || !self.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self) {
        self.settings.save();
    }
}
