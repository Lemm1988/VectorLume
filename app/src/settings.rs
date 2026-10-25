//! Dauerhafte Programmeinstellungen (Maschinenprofil, Job-Optionen, Ansicht).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use vl_cam::{JobSettings, MachineProfile, TestPattern};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeChoice {
    #[default]
    Dark,
    Light,
    System,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub profile: MachineProfile,
    pub job: JobSettings,
    pub theme: ThemeChoice,
    /// Sprache der Oberfläche; beim ersten Start die des Systems.
    pub language: crate::i18n::Language,
    pub recent_files: Vec<PathBuf>,
    pub last_port: String,
    /// Adresse für „Netzwerk“, z. B. `192.168.4.1:23`.
    pub network_address: String,
    pub jog_step: f64,
    pub show_travel: bool,
    pub safety_acknowledged: bool,
    pub snap_to_grid: bool,
    pub grid_snap_mm: f64,
    pub snap_to_objects: bool,
    pub test_pattern: TestPattern,
    pub camera: crate::camera::CameraSettings,
    /// Eigene Befehlsfolgen als Knöpfe in der Konsole.
    pub macros: Vec<Macro>,
}

/// Benannte G-Code-Zeilen, die mit einem Klick gesendet werden (z. B. Luft an/aus, Parkposition).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Macro {
    pub name: String,
    pub gcode: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            profile: MachineProfile::default(),
            job: JobSettings::default(),
            theme: ThemeChoice::Dark,
            language: crate::i18n::Language::from_system(),
            recent_files: Vec::new(),
            last_port: String::new(),
            network_address: String::new(),
            jog_step: 10.0,
            show_travel: true,
            safety_acknowledged: false,
            snap_to_grid: false,
            grid_snap_mm: 1.0,
            snap_to_objects: true,
            test_pattern: TestPattern::default(),
            camera: Default::default(),
            macros: Vec::new(),
        }
    }
}

/// Ordner für Einstellungen, Autosicherung und Materialbibliothek. `VECTORLUME_CONFIG_DIR` setzt einen
/// eigenen Ordner (z. B. für Tests, ohne die echten Einstellungen zu verändern).
pub fn config_dir() -> Option<PathBuf> {
    match std::env::var_os("VECTORLUME_CONFIG_DIR") {
        Some(d) if !d.is_empty() => Some(PathBuf::from(d)),
        _ => dirs::config_dir().map(|d| d.join("VectorLume")),
    }
}

fn path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("settings.json"))
}

impl AppSettings {
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(self)
            && let Err(e) = std::fs::write(&p, text)
        {
            log::warn!("Einstellungen konnten nicht gespeichert werden: {e}");
        }
    }

    pub fn add_recent(&mut self, file: PathBuf) {
        self.recent_files.retain(|f| f != &file);
        self.recent_files.insert(0, file);
        self.recent_files.truncate(10);
    }
}
