//! Dauerhafte Programmeinstellungen (Maschinenprofil, Job-Optionen, Ansicht).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    /// Aktives Maschinenprofil (Arbeitskopie von `devices[device_index]`).
    pub profile: MachineProfile,
    /// Alle eingerichteten Geräte; der Eintrag des aktiven wird beim Speichern und Umschalten aktualisiert.
    pub devices: Vec<MachineProfile>,
    pub device_index: usize,
    pub job: JobSettings,
    pub theme: ThemeChoice,
    /// Sprache der Oberfläche; beim ersten Start die des Systems.
    pub language: crate::i18n::Language,
    /// Maßeinheit der Anzeige (intern immer mm); beim ersten Start Zoll nur in den USA.
    pub units: crate::units::Units,
    pub recent_files: Vec<PathBuf>,
    pub last_port: String,
    /// Adresse für „Netzwerk“, z. B. `192.168.4.1:23`.
    pub network_address: String,
    pub jog_step: f64,
    pub show_travel: bool,
    /// Rechte Seitenleiste (Ebenen, Laser, Konsole, Bibliothek); ausblendbar für kleine Bildschirme.
    pub show_side_panel: bool,
    pub safety_acknowledged: bool,
    pub snap_to_grid: bool,
    pub grid_snap_mm: f64,
    pub snap_to_objects: bool,
    pub test_pattern: TestPattern,
    pub camera: crate::camera::CameraSettings,
    /// Eigene Befehlsfolgen als Knöpfe in der Konsole.
    pub macros: Vec<Macro>,
    /// Geänderte Tastenkürzel je Befehl (z. B. `"save": ["Ctrl+S"]`, leere Liste = keine Taste).
    pub hotkeys: BTreeMap<String, Vec<String>>,
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
            devices: Vec::new(),
            device_index: 0,
            job: JobSettings::default(),
            theme: ThemeChoice::Dark,
            language: crate::i18n::Language::from_system(),
            units: crate::units::Units::from_system(),
            recent_files: Vec::new(),
            last_port: String::new(),
            network_address: String::new(),
            jog_step: 10.0,
            show_travel: true,
            show_side_panel: true,
            safety_acknowledged: false,
            snap_to_grid: false,
            grid_snap_mm: 1.0,
            snap_to_objects: true,
            test_pattern: TestPattern::default(),
            camera: Default::default(),
            macros: Vec::new(),
            hotkeys: BTreeMap::new(),
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
        let mut s: Self = path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        s.normalize_devices();
        s
    }

    /// Ältere Einstellungen kennen nur ein Profil: es wird das erste Gerät.
    fn normalize_devices(&mut self) {
        if self.devices.is_empty() {
            self.devices.push(self.profile.clone());
            self.device_index = 0;
        }
        if self.device_index >= self.devices.len() {
            self.device_index = 0;
            self.profile = self.devices[0].clone();
        }
    }

    fn sync_device(&mut self) {
        self.normalize_devices();
        self.devices[self.device_index] = self.profile.clone();
    }

    /// Namen aller Geräte in Listenreihenfolge (das aktive mit seinem aktuellen Namen).
    pub fn device_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.devices.iter().map(|d| d.name.clone()).collect();
        match names.get_mut(self.device_index) {
            Some(n) => *n = self.profile.name.clone(),
            None => names.push(self.profile.name.clone()),
        }
        names
    }

    pub fn switch_device(&mut self, index: usize) {
        self.sync_device();
        if index < self.devices.len() {
            self.device_index = index;
            self.profile = self.devices[index].clone();
        }
    }

    /// Fügt ein Gerät hinzu und macht es aktiv.
    pub fn add_device(&mut self, profile: MachineProfile) {
        self.sync_device();
        self.devices.push(profile);
        self.switch_device(self.devices.len() - 1);
    }

    /// Entfernt das aktive Gerät (das letzte bleibt immer erhalten).
    pub fn remove_device(&mut self) -> bool {
        self.sync_device();
        if self.devices.len() < 2 {
            return false;
        }
        self.devices.remove(self.device_index);
        self.device_index = self.device_index.min(self.devices.len() - 1);
        self.profile = self.devices[self.device_index].clone();
        true
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut copy = self.clone();
        copy.sync_device();
        if let Ok(text) = serde_json::to_string_pretty(&copy)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> MachineProfile {
        MachineProfile { name: name.into(), ..Default::default() }
    }

    #[test]
    fn devices_switch_add_remove() {
        let mut s = AppSettings { profile: named("A"), ..Default::default() };
        s.normalize_devices();
        assert_eq!(s.device_names(), ["A"]);
        // Änderungen am aktiven Profil bleiben beim Umschalten erhalten
        s.profile.bed_width = 123.0;
        s.add_device(named("B"));
        assert_eq!((s.device_index, s.profile.name.as_str()), (1, "B"));
        s.profile.name = "B2".into();
        assert_eq!(s.device_names(), ["A", "B2"]);
        s.switch_device(0);
        assert_eq!((s.profile.name.as_str(), s.profile.bed_width), ("A", 123.0));
        assert_eq!(s.devices[1].name, "B2");
        assert!(s.remove_device());
        assert_eq!((s.device_index, s.profile.name.as_str()), (0, "B2"));
        assert!(!s.remove_device(), "das letzte Gerät bleibt");
        // Einstellungen ohne Geräteliste (ältere Version)
        let mut old: AppSettings = serde_json::from_str(r#"{"profile":{"name":"Alt"}}"#).unwrap();
        old.normalize_devices();
        assert_eq!(old.device_names(), ["Alt"]);
    }
}
