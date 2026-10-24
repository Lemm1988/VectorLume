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
    pub recent_files: Vec<PathBuf>,
    pub last_port: String,
    pub jog_step: f64,
    pub show_travel: bool,
    pub safety_acknowledged: bool,
    pub snap_to_grid: bool,
    pub grid_snap_mm: f64,
    pub test_pattern: TestPattern,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            profile: MachineProfile::default(),
            job: JobSettings::default(),
            theme: ThemeChoice::Dark,
            recent_files: Vec::new(),
            last_port: String::new(),
            jog_step: 10.0,
            show_travel: true,
            safety_acknowledged: false,
            snap_to_grid: false,
            grid_snap_mm: 1.0,
            test_pattern: TestPattern::default(),
        }
    }
}

fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("VectorLume").join("settings.json"))
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
