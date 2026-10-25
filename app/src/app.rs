//! Zentraler Zustand der Anwendung und alle Aktionen (Dateien, Bearbeiten, Jobs, Gerät).

use crate::canvas::{GeomCache, TextureCache, View};
use crate::history::History;
use crate::i18n::{tr, trs};
use crate::settings::{AppSettings, ThemeChoice};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use vl_cam::{Job, StartFrom, strip_for_streaming};
use vl_core::{Affine, Document, Object, ObjectId, Point, Rect, Shape};
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
    Barcode,
    Node,
    Measure,
    Trim,
    Tabs,
}

impl Tool {
    pub const ALL: [Tool; 12] = [
        Tool::Select,
        Tool::Node,
        Tool::Trim,
        Tool::Tabs,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Line,
        Tool::Polygon,
        Tool::Star,
        Tool::Text,
        Tool::Barcode,
        Tool::Measure,
    ];

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
            Tool::Barcode => i::QR_CODE,
            Tool::Node => i::BEZIER_CURVE,
            Tool::Measure => i::RULER,
            Tool::Trim => i::SCISSORS,
            Tool::Tabs => i::LINK_BREAK,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => tr("Auswählen"),
            Tool::Rect => tr("Rechteck"),
            Tool::Ellipse => tr("Ellipse"),
            Tool::Line => tr("Linie"),
            Tool::Polygon => tr("Vieleck"),
            Tool::Star => tr("Stern"),
            Tool::Text => tr("Text"),
            Tool::Barcode => tr("QR-/Barcode"),
            Tool::Node => tr("Knoten bearbeiten"),
            Tool::Measure => tr("Messen"),
            Tool::Trim => tr("Trimmen"),
            Tool::Tabs => tr("Laschen setzen"),
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
    /// Rahmen fahren; `true` = konvexe Hülle statt Rechteck.
    Frame(bool),
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
    /// Der laufende Job wurde aus dem Dokument erzeugt.
    pub document_job: bool,
    /// Schalter „Rahmen wiederholen“: der Rahmen läuft immer wieder, bis gestoppt oder ausgeschaltet.
    pub frame_loop: bool,
    /// Zeilen des laufenden Wiederhol-Rahmens, nach jedem Durchgang erneut gesendet.
    pub frame_repeat: Option<vl_core::LineBuf>,
    /// Durchgänge des laufenden Rahmens.
    pub frame_passes: u32,
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
    ConfirmStart {
        job: Arc<Job>,
    },
    LaserModeOff,
    Safety {
        job: Arc<Job>,
    },
    /// Fremde G-Code-Datei senden (Inhalt ungeprüft).
    SendFile {
        name: String,
        lines: Arc<vl_core::LineBuf>,
    },
    Unsaved(AfterDiscard),
    Message {
        title: String,
        text: String,
    },
    Recover {
        path: PathBuf,
        meta: crate::autosave::Meta,
    },
    /// Fehlerbericht: aus der letzten Sitzung oder (`running`) gerade eben in einem Hintergrund-Thread.
    Crash {
        path: PathBuf,
        running: bool,
    },
}

pub struct VectorLumeApp {
    pub ctx: egui::Context,
    pub doc: Document,
    pub history: History,
    pub selection: Vec<ObjectId>,
    pub current_layer: usize,
    /// Gewählter Reiter der Ebeneneinstellungen: 0 = Hauptbearbeitung, ab 1 Unterebenen.
    pub sub_tab: usize,
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
    /// Makro im Bearbeiten-Fenster: Index (None = neu) und Entwurf.
    pub macro_edit: Option<(Option<usize>, crate::settings::Macro)>,
    pub file_path: Option<PathBuf>,
    /// Name für Titel und Speichern-Dialog, solange das Projekt noch keine Datei hat (z. B. nach LightBurn-Import).
    pub name_hint: Option<String>,
    pub dirty: bool,
    pub revision: u64,
    pub clipboard: Vec<Object>,
    /// Versatz des letzten Einfügens; wächst mit jedem Einfügen, damit Kopien nicht übereinander liegen.
    pub paste_offset: f64,
    pub preview: PreviewState,
    pub jobs: JobWorker,
    pub toasts: Vec<Toast>,
    pub geom: GeomCache,
    pub textures: TextureCache,
    pub drag: crate::canvas::Drag,
    pub node_edit: crate::node_tool::NodeEdit,
    pub snap_hint: Option<crate::snapping::SnapHint>,
    pub snap_all: Option<(u64, Arc<crate::snapping::SnapTargets>)>,
    pub modal: Option<Modal>,
    /// Fehlerbericht, der gemeldet wird, sobald kein anderer Hinweis offen ist.
    pub pending_crash: Option<(PathBuf, bool)>,
    pub text_dialog: crate::text_tool::TextDialog,
    pub font_families: Arc<Mutex<Option<Vec<String>>>>,
    pub show_device_settings: bool,
    pub show_rotary: bool,
    pub print_cut: crate::print_cut::PrintCut,
    pub center_finder: crate::center_finder::CenterFinder,
    pub box_gen: crate::box_gen::BoxGen,
    pub hinge: crate::hinge_tool::HingeTool,
    pub puzzle: crate::puzzle_tool::PuzzleTool,
    pub kerf: crate::kerf_tool::KerfTool,
    pub gear: crate::gear_tool::GearTool,
    pub grbl: crate::grbl_params::GrblParams,
    pub manual: crate::manual::Manual,
    pub model: crate::model_tool::ModelTool,
    pub history_window: crate::history_window::HistoryWindow,
    /// Aktuelle Tastenbelegung (aus `settings.hotkeys`).
    pub keys: crate::hotkeys::Keymap,
    pub hotkey_editor: crate::hotkeys::HotkeyEditor,
    /// Assistent „Gerät einrichten“.
    pub setup: crate::setup_wizard::SetupWizard,
    /// Laufendes Verzerren der Auswahl.
    pub warp: Option<crate::warp_tool::WarpSession>,
    pub tangent: Option<crate::tangent_tool::TangentPick>,
    /// Schnittreihenfolge per Klick festlegen.
    pub cut_order: Option<crate::cut_order::OrderPick>,
    pub library: crate::library::Library,
    /// Letzte Messung des Messwerkzeugs (Start, Ende).
    pub measure: Option<(Point, Point)>,
    /// Vorschau des Trimm-Werkzeugs: Mauszeiger, Projektstand und das Stück, das ein Klick entfernt.
    pub trim_hover: Option<(Point, u64, Option<crate::canvas::TrimHit>)>,
    /// Vorschau des Laschenwerkzeugs: Mauszeiger, Projektstand und was ein Klick bewirkt.
    pub tab_hover: Option<(Point, u64, Option<crate::tab_tool::TabHit>)>,
    pub camera: crate::camera::CameraState,
    pub show_about: bool,
    pub show_notes: bool,
    pub right_tab: crate::ui_side::RightTab,
    pub lock_ratio: bool,
    pub rotate_by: f64,
    pub materials: crate::materials::MaterialsUi,
    pub test: crate::materials::TestUi,
    pub autosave: crate::autosave::Autosave,
    pub shape_dialogs: crate::shape_ops::ShapeDialogs,
    pub trace: crate::trace_ui::TraceUi,
    /// Zuletzt für den Start erzeugter Dokument-Job (Zeigeradresse zum Wiedererkennen in `send_job`).
    pub document_job: Option<usize>,
    last_edit: Option<(&'static str, Instant)>,
    /// Verschiebung der ersten importierten Leiterplattendatei; weitere Lagen desselben Boards folgen ihr.
    pcb_offset: Option<Point>,
    last_title: String,
    close_confirmed: bool,
}

impl VectorLumeApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::theme::install(&cc.egui_ctx);
        let settings = AppSettings::load();
        crate::i18n::set_language(settings.language);
        crate::units::set_units(settings.units);
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
        let keys = crate::hotkeys::Keymap::new(&settings.hotkeys);
        let mut app = Self {
            ctx,
            doc: Document::new(),
            history: History::default(),
            selection: Vec::new(),
            current_layer: 0,
            sub_tab: 0,
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
            macro_edit: None,
            file_path: None,
            name_hint: None,
            dirty: false,
            revision: 1,
            clipboard: Vec::new(),
            paste_offset: 0.0,
            preview: PreviewState { progress: 1.0, ..Default::default() },
            jobs: JobWorker { tx, rx, next_id: 1, busy: 0, preview_in_flight: false },
            toasts: Vec::new(),
            geom: GeomCache::default(),
            textures: TextureCache::default(),
            drag: crate::canvas::Drag::None,
            node_edit: Default::default(),
            snap_hint: None,
            snap_all: None,
            modal: None,
            pending_crash: None,
            text_dialog: Default::default(),
            font_families,
            show_device_settings: false,
            show_rotary: false,
            print_cut: Default::default(),
            center_finder: Default::default(),
            box_gen: Default::default(),
            hinge: Default::default(),
            puzzle: Default::default(),
            kerf: Default::default(),
            gear: Default::default(),
            grbl: Default::default(),
            manual: Default::default(),
            model: Default::default(),
            history_window: Default::default(),
            keys,
            hotkey_editor: Default::default(),
            setup: Default::default(),
            warp: None,
            tangent: None,
            cut_order: None,
            library: Default::default(),
            measure: None,
            trim_hover: None,
            tab_hover: None,
            camera: Default::default(),
            show_about: false,
            show_notes: false,
            right_tab: crate::ui_side::RightTab::Laser,
            lock_ratio: true,
            rotate_by: 90.0,
            materials: crate::materials::MaterialsUi::load(),
            test: crate::materials::TestUi::default(),
            autosave: crate::autosave::Autosave::default(),
            shape_dialogs: crate::shape_ops::ShapeDialogs::default(),
            trace: Default::default(),
            document_job: None,
            last_edit: None,
            pcb_offset: None,
            last_title: String::new(),
            close_confirmed: false,
        };
        // Vor dem Öffnen von Dateien, die sonst die Sicherung der letzten Sitzung ersetzen würden
        if let Some((path, meta)) = crate::autosave::take_previous() {
            app.modal = Some(Modal::Recover { path, meta });
        }
        app.pending_crash = crate::crash::take_previous().map(|p| (p, false));
        if app.settings.first_run {
            app.open_setup_wizard(true);
        }
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

    pub fn set_units(&mut self, u: crate::units::Units) {
        self.settings.units = u;
        crate::units::set_units(u);
        self.settings.save();
    }

    pub fn set_language(&mut self, lang: crate::i18n::Language) {
        self.settings.language = lang;
        crate::i18n::set_language(lang);
        self.settings.save();
        // Beschriftungen des Materialtests neu erzeugen
        self.test.built = None;
    }

    pub fn toast(&mut self, kind: ToastKind, text: impl Into<String>) {
        let secs = if kind == ToastKind::Error { 7 } else { 4 };
        // Meldungen aus den Bibliotheken kommen deutsch an; übersetzt wird hier zentral
        self.toasts.push(Toast { text: trs(&text.into()), kind, until: Instant::now() + Duration::from_secs(secs) });
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

    // Notizen gehören nicht zum Rückgängig-Verlauf: sie bleiben beim Springen erhalten.
    pub fn undo(&mut self) {
        let notes = self.doc.notes.clone();
        if self.history.undo(&mut self.doc) {
            self.doc.notes = notes;
            self.after_history_jump();
        }
    }

    pub fn redo(&mut self) {
        let notes = self.doc.notes.clone();
        if self.history.redo(&mut self.doc) {
            self.doc.notes = notes;
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
        if self.dirty && (!self.doc.objects.is_empty() || !self.doc.notes.is_empty()) {
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
        self.autosave.clear();
        self.history.clear();
        self.selection.clear();
        self.geom.clear();
        self.pcb_offset = None;
        self.file_path = path;
        self.name_hint = None;
        self.dirty = false;
        self.revision += 1;
        self.current_layer = self.doc.used_layers().first().copied().unwrap_or(0);
        self.show_notes = self.doc.notes.show_on_open && !self.doc.notes.is_empty();
        if !self.show_notes && !self.doc.notes.is_empty() {
            self.toast(ToastKind::Info, tr("Das Projekt enthält Notizen (Ansicht → Projektnotizen)."));
        }
    }

    pub fn open_dialog(&mut self, force: bool, path: Option<PathBuf>) {
        if !force && !self.confirm_discard(AfterDiscard::Open(path.clone())) {
            return;
        }
        let path = path.or_else(|| {
            rfd::FileDialog::new()
                .set_title(tr("Projekt öffnen"))
                .add_filter(tr("Alle Projekte"), &[vl_core::project::PROJECT_EXTENSION, "lbrn2"])
                .add_filter(tr("VectorLume-Projekt"), &[vl_core::project::PROJECT_EXTENSION])
                .add_filter(tr("LightBurn-Projekt"), &["lbrn2"])
                .pick_file()
        });
        if let Some(p) = path {
            self.open_project(&p);
        }
    }

    /// Lädt eine automatische Sicherung; das Projekt gilt danach als ungespeichert.
    pub fn restore_autosave(&mut self, path: &Path, original: Option<PathBuf>) {
        let loaded = vl_core::project::load_project(path);
        crate::autosave::discard_previous(path);
        match loaded {
            Ok(doc) => {
                self.doc = doc;
                self.reset_after_load(original);
                self.dirty = true;
                self.zoom_to_fit();
                self.toast(ToastKind::Success, tr("Sicherung wiederhergestellt. Bitte das Projekt speichern."));
            }
            Err(e) => {
                self.autosave.clear();
                self.toast(ToastKind::Error, trs(&format!("Sicherung unbrauchbar: {e}")));
            }
        }
    }

    pub fn open_project(&mut self, path: &Path) {
        if is_lightburn(path) {
            self.open_lightburn(path);
            return;
        }
        match vl_core::project::load_project(path) {
            Ok(doc) => {
                self.doc = doc;
                self.reset_after_load(Some(path.to_path_buf()));
                self.settings.add_recent(path.to_path_buf());
                self.settings.save();
                self.zoom_to_fit();
                self.toast(ToastKind::Success, trs(&format!("„{}“ geöffnet.", file_name(path))));
            }
            Err(e) => self.toast(ToastKind::Error, e),
        }
    }

    /// LightBurn-Projekt als neues, noch ungespeichertes VectorLume-Projekt übernehmen.
    fn open_lightburn(&mut self, path: &Path) {
        let bed_height = self.settings.profile.bed_height;
        let result = std::fs::read(path)
            .map_err(|e| trs(&format!("Datei konnte nicht gelesen werden: {e}")))
            .and_then(|data| vl_io::import_lightburn(&data, bed_height, path.parent()));
        match result {
            Ok(im) => {
                self.doc = im.doc;
                self.reset_after_load(None);
                self.name_hint = path.file_stem().map(|s| s.to_string_lossy().into_owned());
                self.dirty = true;
                self.settings.add_recent(path.to_path_buf());
                self.settings.save();
                self.zoom_to_fit();
                self.toast(
                    ToastKind::Success,
                    trs(&format!(
                        "LightBurn-Projekt „{}“ übernommen. Gespeichert wird als VectorLume-Projekt.",
                        file_name(path)
                    )),
                );
                for w in im.warnings {
                    self.toast(ToastKind::Info, w);
                }
            }
            Err(e) => self.toast(ToastKind::Error, format!("{}: {e}", file_name(path))),
        }
    }

    pub fn save(&mut self, save_as: bool) -> bool {
        let path = match (&self.file_path, save_as) {
            (Some(p), false) => Some(p.clone()),
            _ => rfd::FileDialog::new()
                .set_title(tr("Projekt speichern"))
                .set_file_name(format!(
                    "{}.{}",
                    self.name_hint.as_deref().unwrap_or(tr("Projekt")),
                    vl_core::project::PROJECT_EXTENSION
                ))
                .add_filter(tr("VectorLume-Projekt"), &[vl_core::project::PROJECT_EXTENSION])
                .save_file(),
        };
        let Some(mut path) = path else { return false };
        if path.extension().is_none() {
            path.set_extension(vl_core::project::PROJECT_EXTENSION);
        }
        match vl_core::project::save_project(&self.doc, &path) {
            Ok(()) => {
                self.dirty = false;
                self.autosave.clear();
                self.settings.add_recent(path.clone());
                self.settings.save();
                self.toast(ToastKind::Success, trs(&format!("Gespeichert: {}", file_name(&path))));
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
        all.extend_from_slice(vl_io::MODEL_EXTENSIONS);
        let pcb = vl_io::pcb_extensions();
        all.extend_from_slice(&pcb);
        if let Some(files) = rfd::FileDialog::new()
            .set_title(tr("Importieren"))
            .add_filter(tr("Alle unterstützten Dateien"), &all)
            .add_filter(tr("Vektorgrafik (SVG, DXF, PDF, AI, EPS, HPGL)"), vl_io::VECTOR_EXTENSIONS)
            .add_filter(tr("Bilder"), vl_io::IMAGE_EXTENSIONS)
            .add_filter(tr("3D-Modelle (STL, OBJ) als Tiefenbild"), vl_io::MODEL_EXTENSIONS)
            .add_filter(tr("Leiterplatten (Gerber, Excellon-Bohrdateien)"), &pcb)
            .pick_files()
        {
            for f in files {
                self.import_path(&f);
            }
        }
    }

    pub fn import_path(&mut self, path: &Path) {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == vl_core::project::PROJECT_EXTENSION || is_lightburn(path) {
            self.open_dialog(false, Some(path.to_path_buf()));
            return;
        }
        if vl_io::MODEL_EXTENSIONS.contains(&ext.as_str()) {
            self.open_model(path);
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
                    if vl_io::is_pcb_extension(&ext) {
                        self.place_pcb(&ids);
                    } else {
                        self.place_imported(&ids);
                    }
                    self.selection = ids;
                    self.mark_changed();
                    self.toast(ToastKind::Success, trs(&format!("„{}“ importiert.", file_name(path))));
                }
                for w in warnings {
                    self.toast(ToastKind::Info, w);
                }
            }
            Err(e) => self.toast(ToastKind::Error, e),
        }
    }

    /// Leiterplattendateien desselben Boards (Kupfer, Bohrungen, Umriss) um denselben Betrag verschieben, damit sie
    /// deckungsgleich bleiben: die erste mit 10 mm Rand in die linke obere Ecke, alle weiteren genauso.
    fn place_pcb(&mut self, ids: &[ObjectId]) {
        let offset = *self.pcb_offset.get_or_insert_with(|| {
            let b = self.doc.bounds_of(ids);
            let bed = self.settings.profile.bed_rect();
            Point::new(bed.min.x + 10.0 - b.min.x, bed.min.y + 10.0 - b.min.y)
        });
        self.doc.transform_objects(ids, &Affine::translate(offset.x, offset.y));
    }

    /// Importiertes in die Arbeitsfläche legen, falls es (teilweise) außerhalb liegt.
    pub(crate) fn place_imported(&mut self, ids: &[ObjectId]) {
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
        let name = self
            .file_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or(tr("Job"))
            .to_string();
        if let Some(path) = rfd::FileDialog::new()
            .set_title(tr("G-Code speichern"))
            .set_file_name(format!("{name}.gcode"))
            .add_filter(tr("G-Code"), &["gcode", "nc", "gc"])
            .save_file()
        {
            self.request_job(JobPurpose::Export(path));
        }
    }

    pub fn export_svg_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title(tr("Als SVG exportieren"))
            .set_file_name(tr("Export.svg"))
            .add_filter("SVG", &["svg"])
            .save_file()
        {
            let p = &self.settings.profile;
            let svg = vl_io::svg::export_svg(&self.doc, p.bed_width, p.bed_height);
            match std::fs::write(&path, svg) {
                Ok(()) => self.toast(ToastKind::Success, trs(&format!("SVG gespeichert: {}", file_name(&path)))),
                Err(e) => self.toast(ToastKind::Error, trs(&format!("SVG konnte nicht gespeichert werden: {e}"))),
            }
        }
    }

    /// Speichert das ausgewählte Bild so, wie es graviert wird (Zeilenabstand, Bildanpassung, Bildmodus, Negativ
    /// der Ebene): schwarz = volle Leistung, ein Pixel je Rasterpunkt.
    pub fn export_processed_image(&mut self) {
        let Some(o) = self.selection.iter().filter_map(|id| self.doc.get(*id)).find(|o| o.is_image()) else { return };
        let vl_core::Shape::Image(img) = &o.shape else { return };
        let layer = &self.doc.layers[o.layer];
        let Some(cs) = layer.operation(layer.image_operation()) else { return };
        let r = match vl_raster::engrave_image(img, &o.transform, cs) {
            Ok(r) => r,
            Err(e) => {
                self.toast(ToastKind::Error, e.to_string());
                return;
            }
        };
        let dpi = (25.4 / cs.interval.max(1e-3)).round();
        let png = vl_core::RasterImage::new(r.cols as u32, r.rows as u32, r.preview_pixels()).to_png();
        let name = format!("{}_{dpi}dpi.png", o.name.replace(['\\', '/', ':', '*', '?', '"', '<', '>', '|'], "_"));
        let Some(path) = rfd::FileDialog::new()
            .set_title(tr("Bearbeitetes Bild speichern"))
            .set_file_name(name)
            .add_filter("PNG", &["png"])
            .save_file()
        else {
            return;
        };
        match png.and_then(|data| std::fs::write(&path, data).map_err(|e| e.to_string())) {
            Ok(()) => self.toast(
                ToastKind::Success,
                trs(&format!("Gespeichert: {} ({} × {} Punkte, {dpi} dpi)", file_name(&path), r.cols, r.rows)),
            ),
            Err(e) => self.toast(ToastKind::Error, trs(&format!("Bild konnte nicht gespeichert werden: {e}"))),
        }
    }

    /// PDF im Maßstab 1:1; die Seite umfasst den Entwurf (sichtbare Ebenen) plus 5 mm Rand, zum Drucken in 100 %.
    pub fn export_pdf_dialog(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(tr("Als PDF exportieren"))
            .set_file_name(tr("Export.pdf"))
            .add_filter("PDF", &["pdf"])
            .save_file()
        else {
            return;
        };
        let visible = self
            .doc
            .objects
            .iter()
            .filter(|o| self.doc.layers.get(o.layer).is_some_and(|l| l.visible))
            .fold(vl_core::Rect::EMPTY, |r, o| r.union(&o.bounds()));
        let p = &self.settings.profile;
        let page = if visible.is_empty() {
            vl_core::Rect::from_xywh(0.0, 0.0, p.bed_width, p.bed_height)
        } else {
            visible.expanded(5.0)
        };
        let title = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        match vl_io::pdf_export::export_pdf(&self.doc, page, &title)
            .and_then(|data| std::fs::write(&path, data).map_err(|e| e.to_string()))
        {
            Ok(()) => self.toast(
                ToastKind::Success,
                trs(&format!(
                    "PDF gespeichert: {} ({} × {}, in 100 % drucken)",
                    file_name(&path),
                    crate::units::num(page.width(), 0),
                    crate::units::len(page.height(), 0)
                )),
            ),
            Err(e) => self.toast(ToastKind::Error, trs(&format!("PDF konnte nicht gespeichert werden: {e}"))),
        }
    }

    pub fn export_dxf_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title(tr("Als DXF exportieren"))
            .set_file_name(tr("Export.dxf"))
            .add_filter("DXF", &["dxf"])
            .save_file()
        {
            let out = vl_io::dxf::export_dxf(&self.doc, self.settings.profile.bed_height);
            match std::fs::write(&path, out.text) {
                Ok(()) => {
                    self.toast(ToastKind::Success, trs(&format!("DXF gespeichert: {}", file_name(&path))));
                    if out.skipped_images > 0 {
                        self.toast(
                            ToastKind::Info,
                            trs(&format!("{} Bild(er) sind im DXF nicht enthalten.", out.skipped_images)),
                        );
                    }
                }
                Err(e) => self.toast(ToastKind::Error, trs(&format!("DXF konnte nicht gespeichert werden: {e}"))),
            }
        }
    }

    // ---------------------------------------------------------------- Bearbeiten

    pub fn select_all(&mut self) {
        self.selection =
            self.doc.objects.iter().filter(|o| self.doc.layers[o.layer].selectable()).map(|o| o.id).collect();
    }

    pub fn invert_selection(&mut self) {
        let old = std::mem::take(&mut self.selection);
        self.selection = self
            .doc
            .objects
            .iter()
            .filter(|o| self.doc.layers[o.layer].selectable() && !old.contains(&o.id))
            .map(|o| o.id)
            .collect();
    }

    pub fn select_layer(&mut self, layer: usize) {
        self.selection = self.doc.objects_on_layer(layer).map(|o| o.id).collect();
    }

    /// Wählt Pfade mit offenen Enden – sie werden beim Füllen übergangen und beim Schneiden nicht geschlossen.
    pub fn select_open_shapes(&mut self) {
        self.selection = self
            .doc
            .objects
            .iter()
            .filter(|o| self.doc.layers[o.layer].selectable() && o.text.is_none())
            .filter(|o| matches!(&o.shape, Shape::Path(p) if p.subpaths.iter().any(|s| !s.closed)))
            .map(|o| o.id)
            .collect();
        let n = self.selection.len();
        if n == 0 {
            self.toast(ToastKind::Info, tr("Keine offenen Formen gefunden."));
        } else {
            self.toast(ToastKind::Info, trs(&format!("{n} offene Formen ausgewählt.")));
        }
    }

    /// Genau ein Bild und mindestens eine Vektorform ausgewählt.
    pub fn can_clip_image(&self) -> bool {
        let objs: Vec<&Object> = self.selection.iter().filter_map(|id| self.doc.get(*id)).collect();
        objs.iter().filter(|o| o.is_image()).count() == 1 && objs.iter().any(|o| !o.is_image())
    }

    /// Ausgewähltes Bild mit Zuschnitt.
    pub fn selected_clipped_image(&self) -> Option<ObjectId> {
        self.selection
            .iter()
            .copied()
            .find(|id| self.doc.get(*id).is_some_and(|o| matches!(&o.shape, Shape::Image(img) if !img.clip.is_empty())))
    }

    /// Schneidet das ausgewählte Bild auf die geschlossenen Formen der Auswahl zu; die Formen werden entfernt.
    pub fn clip_image_to_shapes(&mut self) {
        if !self.can_clip_image() {
            self.toast(ToastKind::Info, tr("Bitte ein Bild und mindestens eine geschlossene Form auswählen."));
            return;
        }
        let objs: Vec<&Object> = self.selection.iter().filter_map(|id| self.doc.get(*id)).collect();
        let Some(image) = objs.iter().find(|o| o.is_image()) else { return };
        let (image_id, inv) = (image.id, image.transform.inverse());
        let Some(inv) = inv else { return };
        let shapes: Vec<ObjectId> = objs.iter().filter(|o| !o.is_image()).map(|o| o.id).collect();
        let rings: Vec<Vec<Point>> = objs
            .iter()
            .filter(|o| !o.is_image())
            .flat_map(|o| o.polylines(0.02))
            .filter(|p| p.closed && p.points.len() >= 3)
            .map(|p| p.points.iter().map(|q| inv.apply(*q)).collect())
            .collect();
        if rings.is_empty() {
            self.toast(ToastKind::Info, tr("Die Form zum Zuschneiden muss geschlossen sein."));
            return;
        }
        self.checkpoint();
        if let Some(o) = self.doc.get_mut(image_id)
            && let Shape::Image(img) = &mut o.shape
        {
            img.clip = rings;
        }
        self.doc.remove(&shapes);
        self.selection = vec![image_id];
        self.mark_changed();
    }

    pub fn remove_image_clip(&mut self) {
        let Some(id) = self.selected_clipped_image() else { return };
        self.checkpoint();
        if let Some(o) = self.doc.get_mut(id)
            && let Shape::Image(img) = &mut o.shape
        {
            img.clip.clear();
        }
        self.mark_changed();
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

    pub(crate) fn selected_objects(&self) -> Vec<Object> {
        self.doc.objects_in(&self.selection).cloned().collect()
    }

    pub fn copy(&mut self) {
        self.clipboard = self.selected_objects();
        self.paste_offset = 0.0;
    }

    pub fn cut(&mut self) {
        self.copy();
        self.delete_selection();
    }

    /// Einfügen, jedes Mal um weitere 5 mm versetzt.
    pub fn paste(&mut self) {
        self.paste_shifted(true);
    }

    /// Einfügen an der Stelle des Originals.
    pub fn paste_in_place(&mut self) {
        self.paste_shifted(false);
    }

    fn paste_shifted(&mut self, shifted: bool) {
        if self.clipboard.is_empty() {
            return;
        }
        self.checkpoint();
        if shifted {
            self.paste_offset += 5.0;
        }
        let d = if shifted { self.paste_offset } else { 0.0 };
        let objs = self.clipboard.clone();
        self.selection = self.doc.duplicate(&objs, Point::new(d, d));
        self.mark_changed();
    }

    /// Verschiebt die Auswahl in der Objektreihenfolge (Zeichen- und, ohne Optimierung, Schnittreihenfolge).
    pub fn reorder_selection(&mut self, how: vl_core::ZOrder) {
        if self.selection.is_empty() {
            return;
        }
        self.checkpoint();
        if self.doc.reorder(&self.selection, how) {
            self.mark_changed();
        } else {
            self.history.discard_if_unchanged(&self.doc);
        }
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

    /// Formen, die ineinander liegen, gruppieren (Auswahl; ohne Auswahl alle auswählbaren Objekte).
    pub fn auto_group(&mut self) {
        let ids: Vec<ObjectId> = if self.selection.is_empty() {
            self.doc.objects.iter().filter(|o| self.doc.layers[o.layer].selectable()).map(|o| o.id).collect()
        } else {
            self.selection.clone()
        };
        self.checkpoint();
        let n = self.doc.auto_group(&ids);
        if n == 0 {
            self.history.discard_if_unchanged(&self.doc);
            self.toast(ToastKind::Info, tr("Keine ineinanderliegenden Formen gefunden."));
            return;
        }
        self.mark_changed();
        let msg = if n == 1 { tr("1 Gruppe gebildet.").to_string() } else { trs(&format!("{n} Gruppen gebildet.")) };
        self.toast(ToastKind::Success, msg);
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

    /// Spiegelt die Auswahl an der Linie `a`–`b` (Messlinie); mit `copy` bleibt das Original stehen.
    pub fn mirror_across(&mut self, a: Point, b: Point, copy: bool) {
        let Some(t) = Affine::reflect_across(a, b) else { return };
        if self.selection.is_empty() {
            return;
        }
        self.checkpoint();
        if copy {
            let objs = self.selected_objects();
            self.selection = self.doc.duplicate(&objs, Point::ZERO);
        }
        self.doc.transform_objects(&self.selection, &t);
        self.mark_changed();
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
    pub(crate) fn selection_units(&self) -> Vec<Vec<ObjectId>> {
        let mut units: Vec<Vec<ObjectId>> = Vec::new();
        let mut groups: Vec<(u64, usize)> = Vec::new();
        for o in self.doc.objects_in(&self.selection) {
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
        self.move_to_bed(0.5, 0.5);
    }

    /// Schiebt die Auswahl an eine Stelle der Arbeitsfläche: `ax`/`ay` 0 = links/oben, 0,5 = Mitte, 1 = rechts/unten.
    pub fn move_to_bed(&mut self, ax: f64, ay: f64) {
        let b = self.selection_bounds();
        if b.is_empty() {
            return;
        }
        let bed = self.settings.profile.bed_rect();
        let target =
            Point::new(bed.min.x + (bed.width() - b.width()) * ax, bed.min.y + (bed.height() - b.height()) * ay);
        let d = target - b.min;
        self.transform_selection(&Affine::translate(d.x, d.y));
    }

    pub fn assign_layer(&mut self, layer: usize) {
        self.current_layer = layer;
        if self.selection.is_empty() {
            return;
        }
        self.checkpoint();
        let images_only = self.doc.objects_in(&self.selection).all(|o| o.is_image());
        if images_only && self.doc.objects_on_layer(layer).next().is_none() {
            self.doc.layers[layer].settings = vl_core::CutSettings::default_for(vl_core::LayerMode::Image);
        }
        for o in self.doc.objects_in_mut(&self.selection) {
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
        doc.layers[0].name = tr("Schneiden").into();
        let frame = doc.add_path(tr("Rahmen"), 0, VectorPath::rect(30.0, 30.0, 170.0, 120.0, 8.0));
        let hole = doc.add_path(tr("Loch"), 0, VectorPath::ellipse(Point::new(180.0, 50.0), 8.0, 8.0));
        doc.group(&[frame, hole]);
        doc.layers[1].settings = CutSettings { fill_angle: 45.0, ..CutSettings::default_for(LayerMode::Fill) };
        doc.layers[1].name = tr("Gravur").into();
        doc.add_path(tr("Stern"), 1, VectorPath::star(Point::new(80.0, 80.0), 36.0, 15.0, 5));
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
                let id = doc.add(tr("Text „VectorLume“"), 1, Affine::translate(45.0, 126.0), vl_core::Shape::Path(p));
                if let Some(o) = doc.get_mut(id) {
                    let rendered = spec.text.clone();
                    o.text = Some(Box::new(vl_core::TextData { spec, rendered, ..Default::default() }));
                }
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
            clip: Vec::new(),
        };
        doc.add_image(tr("Verlauf"), 3, img, Point::new(220.0, 40.0));
        doc.layers[3].settings.dither = DitherMode::Jarvis;
        doc.layers[3].name = tr("Foto").into();
        self.doc = doc;
        self.reset_after_load(None);
        self.dirty = true;
        self.zoom_to_fit();
    }

    // ---------------------------------------------------------------- Jobs

    pub fn request_job(&mut self, purpose: JobPurpose) {
        // Variable Texte (Zähler, Datum, Uhrzeit) vor jeder Ausgabe aktualisieren
        self.refresh_variable_texts();
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
                        self.document_job = Some(Arc::as_ptr(&job) as usize);
                        self.preview.job = Some(job.clone());
                        self.preview.built_revision = self.revision;
                        self.preview.texture_generation += 1;
                        self.launch_job(job);
                    }
                    Err(e) => self.toast(ToastKind::Error, e),
                },
                JobPurpose::Frame(hull) => match result {
                    Ok(job) => self.run_frame(&job, hull),
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

    /// G-Code-Datei aus einem anderen Programm direkt an den Laser senden.
    pub fn send_gcode_file_dialog(&mut self) {
        if !self.ready_to_start() {
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .set_title(tr("G-Code-Datei senden"))
            .add_filter(tr("G-Code"), &["gcode", "nc", "gc", "ngc", "tap", "txt"])
            .pick_file()
        else {
            return;
        };
        match std::fs::read(&path) {
            Ok(bytes) => {
                let lines = vl_cam::lines_from_file(&String::from_utf8_lossy(&bytes));
                if lines.is_empty() {
                    self.toast(ToastKind::Error, tr("Die Datei enthält keine G-Code-Befehle."));
                } else {
                    self.modal = Some(Modal::SendFile { name: file_name(&path), lines: Arc::new(lines) });
                }
            }
            Err(e) => self.toast(ToastKind::Error, trs(&format!("Datei konnte nicht gelesen werden: {e}"))),
        }
    }

    pub fn send_gcode_lines(&mut self, name: String, lines: &vl_core::LineBuf) {
        if !self.ready_to_start() {
            return;
        }
        self.machine.document_job = false;
        // Keine Schätzung: die Restzeit wird aus dem Fortschritt hochgerechnet
        self.machine.job_estimate = 0.0;
        self.device.send(Command::StartJob { name, lines: lines.clone() });
    }

    pub fn run_frame(&mut self, job: &Job, hull: bool) {
        let lines = strip_for_streaming(&vl_cam::frame_gcode(job, &self.settings.profile, hull));
        self.machine.frame_passes = 1;
        self.machine.frame_repeat = self.machine.frame_loop.then(|| lines.clone());
        self.device.send(Command::StartJob { name: tr("Rahmen").into(), lines });
    }

    pub fn write_gcode(&mut self, job: &Job, path: &Path) {
        // ohne Kopie des (bei Fotos Hunderte MB großen) Textes schreiben
        let written = std::fs::File::create(path).and_then(|mut f| {
            use std::io::Write;
            f.write_all(job.gcode.text().as_bytes())?;
            f.write_all(b"\n")
        });
        match written {
            Ok(()) => {
                let mut msg = trs(&format!(
                    "G-Code gespeichert: {} (geschätzt {})",
                    file_name(path),
                    job.estimate.format_duration()
                ));
                if !job.warnings.is_empty() {
                    msg.push_str(tr(" – mit Warnungen"));
                }
                self.toast(ToastKind::Success, msg);
                for w in &job.warnings {
                    self.toast(ToastKind::Info, w.clone());
                }
            }
            Err(e) => self.toast(ToastKind::Error, trs(&format!("Speichern fehlgeschlagen: {e}"))),
        }
    }

    /// Prüft, ob ein Job gestartet werden darf, und meldet sonst den Grund.
    pub fn ready_to_start(&mut self) -> bool {
        if !self.machine.connected {
            self.toast(ToastKind::Error, tr("Kein Laser verbunden."));
            return false;
        }
        if self.machine.job_running {
            self.toast(ToastKind::Info, tr("Es läuft bereits ein Job."));
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
            self.toast(ToastKind::Error, tr("Kein Laser verbunden."));
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

    pub fn frame_job(&mut self, hull: bool) {
        if self.ready_to_frame() {
            self.request_job(JobPurpose::Frame(hull));
        }
    }

    pub fn send_job(&mut self, job: &Job) {
        // Nur Jobs aus dem Dokument (nicht Materialtest/Rahmen) schalten Zähler weiter
        self.machine.document_job = self.document_job.take() == Some(job as *const Job as usize);
        let lines = strip_for_streaming(&job.gcode);
        self.machine.job_estimate = job.estimate.seconds;
        let name = self.file_path.as_ref().map(|p| file_name(p)).unwrap_or_else(|| tr("Job").into());
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
        self.connect_with(None);
    }

    /// Verbinden mit Steuerung und Baudrate eines noch nicht gespeicherten Geräts (Einrichtungsassistent).
    pub fn connect_as(&mut self, profile: &vl_cam::MachineProfile) {
        self.connect_with(Some(profile));
    }

    fn connect_with(&mut self, profile: Option<&vl_cam::MachineProfile>) {
        let port = self.selected_port.clone();
        if port.is_empty() {
            self.toast(ToastKind::Error, tr("Kein Anschluss ausgewählt."));
            return;
        }
        self.settings.last_port = port.clone();
        self.settings.save();
        let port = if port == vl_device::NETWORK_PORT {
            let addr = self.settings.network_address.trim();
            if addr.is_empty() {
                self.toast(
                    ToastKind::Error,
                    tr("Bitte die Netzwerkadresse der Steuerung eingeben (z. B. 192.168.4.1)."),
                );
                return;
            }
            format!("{}{}", vl_device::TCP_PREFIX, vl_device::with_default_port(addr))
        } else {
            port
        };
        let p = profile.unwrap_or(&self.settings.profile);
        self.machine.settings.clear();
        self.device.send(Command::Connect { port, baud: p.baud, rx_buffer: p.rx_buffer, firmware: p.firmware });
    }

    pub fn disconnect(&mut self) {
        self.device.send(Command::Disconnect);
    }

    pub fn send_line(&mut self, line: impl Into<String>) {
        self.device.send(Command::Line(line.into()));
    }

    pub fn firmware(&self) -> vl_core::Firmware {
        self.settings.profile.firmware
    }

    pub fn home(&mut self) {
        self.send_line(self.firmware().home_command());
    }

    pub fn unlock(&mut self) {
        self.send_line(self.firmware().unlock_command());
    }

    /// Verfahren ohne Laser; `axes` z. B. `"X10 Y-5"`.
    pub fn send_jog(&mut self, relative: bool, axes: &str) {
        let lines = self.firmware().jog_lines(relative, axes, self.settings.profile.jog_speed);
        for l in lines {
            self.send_line(l);
        }
    }

    pub fn jog(&mut self, screen_dir: Point) {
        let profile = &self.settings.profile;
        let mut d = profile.screen_to_machine_delta(screen_dir * self.settings.jog_step);
        // Mit Drehachse bewegt Y die Oberfläche des Werkstücks um die Schrittweite.
        let mut y_axis = 'Y';
        if let Some(k) = profile.rotary.active_scale() {
            d.y *= k;
            y_axis = profile.rotary.axis.letter();
        }
        let mut axes = Vec::new();
        if d.x != 0.0 {
            axes.push(format!("X{}", vl_cam::gcode::fmt_mm(d.x)));
        }
        if d.y != 0.0 {
            axes.push(format!("{y_axis}{}", vl_cam::gcode::fmt_mm(d.y)));
        }
        self.send_jog(true, &axes.join(" "));
    }

    /// Dreht die Drehachse um `travel` (Achsweg) – zum Einrichten.
    pub fn jog_rotary(&mut self, travel: f64) {
        let axis = self.settings.profile.rotary.axis.letter();
        self.send_jog(true, &format!("{axis}{}", vl_cam::gcode::fmt_mm(travel)));
    }

    /// Fährt den Laser auf einen Punkt des Dokuments (absolute Koordinaten).
    pub fn jog_to_doc(&mut self, p: Point) {
        let m = self.settings.profile.doc_to_machine(p);
        self.send_jog(false, &format!("X{} Y{}", vl_cam::gcode::fmt_mm(m.x), vl_cam::gcode::fmt_mm(m.y)));
    }

    /// Laser kann zu Dokumentpunkten fahren: verbunden, kein Job, absolute Koordinaten, keine Drehachse.
    pub fn can_jog_to_doc(&self) -> bool {
        self.machine.connected
            && !self.machine.job_running
            && self.settings.job.start_from == StartFrom::Absolute
            && self.settings.profile.rotary.active_scale().is_none()
    }

    /// Fährt den Laser zum Job-Ankerpunkt der Auswahl (ohne Auswahl: des ganzen Entwurfs).
    pub fn move_laser_to_selection(&mut self) {
        let b = if self.selection.is_empty() { self.doc.bounds() } else { self.selection_bounds() };
        if !b.is_empty() {
            self.jog_to_doc(self.settings.job.anchor.point_in(&b));
        }
    }

    pub fn set_user_origin_here(&mut self) {
        match self.machine.head_position() {
            Some(p) => self.set_user_origin(p),
            None => self.toast(ToastKind::Error, tr("Position unbekannt – ist der Laser verbunden?")),
        }
    }

    /// Setzt den Benutzer-Ursprung (Arbeitskoordinaten) und startet Jobs künftig dort.
    pub fn set_user_origin(&mut self, p: Point) {
        self.settings.job.user_origin = p;
        self.settings.job.start_from = StartFrom::UserOrigin;
        self.settings.save();
        self.toast(ToastKind::Success, trs(&format!("Benutzer-Ursprung gesetzt: X {:.2} Y {:.2}", p.x, p.y)));
    }

    /// Wechselt das aktive Gerät (nur getrennt und ohne laufenden Job).
    pub fn switch_device(&mut self, index: usize) {
        if self.machine.connected || index == self.settings.device_index {
            return;
        }
        self.settings.switch_device(index);
        self.after_device_change();
    }

    pub fn after_device_change(&mut self) {
        self.settings.save();
        self.revision += 1;
        self.view.request_fit(None);
        self.toast(ToastKind::Info, trs(&format!("Gerät „{}“ ist aktiv.", self.settings.profile.name)));
    }

    pub fn export_device_dialog(&mut self) {
        let p = &self.settings.profile;
        if let Some(path) = rfd::FileDialog::new()
            .set_title(tr("Gerät exportieren"))
            .add_filter(tr("VectorLume-Gerät"), &["vldevice"])
            .set_file_name(format!("{}.vldevice", p.name))
            .save_file()
        {
            let result = serde_json::to_string_pretty(p)
                .map_err(|e| e.to_string())
                .and_then(|text| std::fs::write(&path, text).map_err(|e| e.to_string()));
            match result {
                Ok(()) => self.toast(ToastKind::Success, tr("Gerät exportiert.")),
                Err(e) => self.toast(ToastKind::Error, trs(&format!("Export fehlgeschlagen: {e}"))),
            }
        }
    }

    pub fn import_device_dialog(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(tr("Gerät importieren"))
            .add_filter(tr("VectorLume-Gerät"), &["vldevice", "json"])
            .pick_file()
        else {
            return;
        };
        let profile = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| serde_json::from_str::<vl_cam::MachineProfile>(&text).map_err(|e| e.to_string()));
        match profile {
            Ok(p) => {
                self.settings.add_device(p);
                self.after_device_change();
            }
            Err(e) => self.toast(ToastKind::Error, trs(&format!("Gerät konnte nicht gelesen werden: {e}"))),
        }
    }

    /// Übernimmt Werte aus `$$` ins Maschinenprofil.
    pub fn apply_device_settings(&mut self) {
        let m = &self.machine;
        let p = &mut self.settings.profile;
        let mut changed = Vec::new();
        if let Some(v) = m.setting_f64(30) {
            p.s_max = v;
            changed.push(tr("S-Max"));
        }
        if let (Some(x), Some(y)) = (m.setting_f64(130), m.setting_f64(131))
            && x > 0.0
            && y > 0.0
        {
            p.bed_width = x;
            p.bed_height = y;
            changed.push(tr("Arbeitsfläche"));
        }
        if let Some(v) = m.setting_f64(110) {
            p.rapid_rate = v;
            changed.push(tr("Eilgang"));
        }
        if let Some(v) = m.setting_f64(120) {
            p.acceleration = v;
            changed.push(tr("Beschleunigung"));
        }
        self.settings.save();
        self.view.request_fit(None);
        if changed.is_empty() {
            self.toast(ToastKind::Info, tr("Keine Werte vom Gerät verfügbar – bitte verbinden."));
        } else {
            self.toast(ToastKind::Success, trs(&format!("Übernommen: {}", changed.join(", "))));
        }
    }

    fn poll_device(&mut self) {
        for ev in self.device.poll() {
            match ev {
                Event::Connected { port } => {
                    self.machine.connected = true;
                    self.console_push(Direction::Info, trs(&format!("Verbunden mit {port}")));
                    self.toast(ToastKind::Success, trs(&format!("Verbunden mit {port}")));
                    self.machine.port = Some(port);
                }
                Event::Disconnected { reason } => {
                    let was = self.machine.connected;
                    self.machine.connected = false;
                    self.machine.status = None;
                    self.machine.job_running = false;
                    self.machine.frame_repeat = None;
                    match reason {
                        Some(r) => {
                            self.console_push(Direction::Info, r.clone());
                            self.toast(ToastKind::Error, r);
                        }
                        None if was => self.toast(ToastKind::Info, tr("Verbindung getrennt.")),
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
                        self.toast(ToastKind::Error, tr("Achtung: GRBL-Laser-Modus ist aus ($32=0)."));
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
                    if let Some(lines) = self.machine.frame_repeat.as_ref().filter(|_| ok && self.machine.frame_loop) {
                        // nächster Durchgang ohne Meldung; der Job gilt dazwischen weiter als laufend
                        self.machine.frame_passes += 1;
                        self.device.send(Command::StartJob { name: tr("Rahmen").into(), lines: lines.clone() });
                        continue;
                    }
                    self.machine.frame_repeat = None;
                    self.machine.job_running = false;
                    if std::mem::take(&mut self.machine.document_job) && ok {
                        self.advance_text_counters();
                    }
                    let text = format!("{message} ({})", vl_cam::format_duration(seconds));
                    self.toast(if ok { ToastKind::Success } else { ToastKind::Error }, text);
                }
                Event::Alarm(code, text) => self.toast(ToastKind::Error, trs(&format!("Alarm {code}: {text}"))),
            }
        }
    }

    // ---------------------------------------------------------------- Rahmen pro Frame

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.close_confirmed {
            if self.machine.job_running {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.modal = Some(Modal::Message {
                    title: tr("Job läuft").into(),
                    text: tr("Bitte den laufenden Job erst stoppen oder abwarten, bevor VectorLume beendet wird.")
                        .into(),
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
        let name = self
            .file_path
            .as_ref()
            .map(|p| file_name(p))
            .or_else(|| self.name_hint.clone())
            .unwrap_or_else(|| tr("Unbenannt").into());
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

fn is_lightburn(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("lbrn2"))
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
        crate::camera::tick(self, &ctx);
        self.handle_close_request(&ctx);
        crate::ui_main::shortcuts(self, &ctx);
        if let Some(path) = crate::crash::take_new() {
            self.pending_crash = Some((path, true));
        }
        if self.modal.is_none()
            && let Some((path, running)) = self.pending_crash.take()
        {
            self.modal = Some(Modal::Crash { path, running });
        }

        crate::ui_main::top_bar(self, ui);
        crate::ui_main::bottom_bar(self, ui);
        crate::ui_main::tool_bar(self, ui);
        crate::ui_side::right_panel(self, ui);
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.canvas_ui(ui));

        crate::dialogs::show(self, &ctx);
        crate::materials::show(self, &ctx);
        crate::shape_ops::show(self, &ctx);
        crate::trace_ui::show(self, &ctx);
        crate::camera::show(self, &ctx);
        self.update_preview();
        self.update_title(&ctx);
        self.autosave.tick(&self.doc, self.revision, self.dirty, self.file_path.as_deref());
        if self.machine.job_running || self.jobs.busy > 0 || !self.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self) {
        self.settings.save();
        self.autosave.clear();
    }
}
