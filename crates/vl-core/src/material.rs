//! Materialbibliothek: bewährte Ebeneneinstellungen je Material, Stärke und Bearbeitung.

use crate::layer::CutSettings;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const LIBRARY_FORMAT: &str = "vectorlume-materials";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaterialEntry {
    /// z. B. „Pappelsperrholz“
    pub material: String,
    /// Materialstärke in mm, 0 = ohne Angabe (z. B. Gravur auf beliebiger Stärke)
    pub thickness: f64,
    /// z. B. „Schneiden“, „Gravieren hell“
    pub title: String,
    pub settings: CutSettings,
    pub notes: String,
}

impl Default for MaterialEntry {
    fn default() -> Self {
        Self {
            material: String::new(),
            thickness: 0.0,
            title: String::new(),
            settings: CutSettings::default(),
            notes: String::new(),
        }
    }
}

impl MaterialEntry {
    pub fn thickness_label(&self) -> String {
        if self.thickness > 0.0 { format!("{} mm", trim_float(self.thickness)) } else { "beliebig".into() }
    }

    fn same_slot(&self, other: &MaterialEntry) -> bool {
        self.material.trim().eq_ignore_ascii_case(other.material.trim())
            && (self.thickness - other.thickness).abs() < 1e-6
            && self.title.trim().eq_ignore_ascii_case(other.title.trim())
    }
}

fn trim_float(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MaterialLibrary {
    pub entries: Vec<MaterialEntry>,
}

#[derive(Serialize, Deserialize)]
struct LibraryFile {
    format: String,
    version: u32,
    entries: Vec<MaterialEntry>,
}

impl MaterialLibrary {
    /// Fügt einen Eintrag hinzu oder ersetzt den mit gleichem Material, gleicher Stärke und Bezeichnung.
    /// Gibt `true` zurück, wenn ein vorhandener Eintrag ersetzt wurde.
    pub fn upsert(&mut self, entry: MaterialEntry) -> bool {
        let replaced = if let Some(e) = self.entries.iter_mut().find(|e| e.same_slot(&entry)) {
            *e = entry;
            true
        } else {
            self.entries.push(entry);
            false
        };
        self.sort();
        replaced
    }

    pub fn sort(&mut self) {
        self.entries.sort_by(|a, b| {
            a.material
                .to_lowercase()
                .cmp(&b.material.to_lowercase())
                .then(a.thickness.total_cmp(&b.thickness))
                .then(a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        });
    }

    /// Materialnamen in Anzeigereihenfolge (ohne Doppelte).
    pub fn materials(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for e in &self.entries {
            if !out.iter().any(|m| m.eq_ignore_ascii_case(&e.material)) {
                out.push(e.material.clone());
            }
        }
        out
    }

    /// Übernimmt alle Einträge aus `other`; gleiche Einträge werden ersetzt. Gibt die Anzahl zurück.
    pub fn merge(&mut self, other: MaterialLibrary) -> usize {
        let n = other.entries.len();
        for e in other.entries {
            self.upsert(e);
        }
        n
    }

    pub fn to_json(&self) -> Result<String, String> {
        let file = LibraryFile { format: LIBRARY_FORMAT.into(), version: 1, entries: self.entries.clone() };
        serde_json::to_string_pretty(&file).map_err(|e| e.to_string())
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let file: LibraryFile = serde_json::from_str(text).map_err(|e| format!("Ungültige Materialdatei: {e}"))?;
        if file.format != LIBRARY_FORMAT {
            return Err("Die Datei ist keine VectorLume-Materialbibliothek.".into());
        }
        if file.version > 1 {
            return Err("Die Materialbibliothek stammt aus einer neueren VectorLume-Version.".into());
        }
        let mut lib = Self { entries: file.entries };
        for e in &mut lib.entries {
            e.settings.sanitize();
            e.thickness = if e.thickness.is_finite() { e.thickness.clamp(0.0, 1000.0) } else { 0.0 };
        }
        lib.sort();
        Ok(lib)
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::from_json(&text)
    }

    /// Speichert über eine temporäre Datei, damit bei einem Absturz keine halbe Bibliothek entsteht.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, self.to_json()?).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::LayerMode;

    fn entry(material: &str, thickness: f64, title: &str, speed: f64) -> MaterialEntry {
        let mut settings = CutSettings::default_for(LayerMode::Line);
        settings.speed = speed;
        MaterialEntry { material: material.into(), thickness, title: title.into(), settings, notes: String::new() }
    }

    #[test]
    fn upsert_replaces_same_slot_and_sorts() {
        let mut lib = MaterialLibrary::default();
        assert!(!lib.upsert(entry("Sperrholz", 3.0, "Schneiden", 300.0)));
        assert!(!lib.upsert(entry("Acryl", 3.0, "Schneiden", 200.0)));
        assert!(lib.upsert(entry("sperrholz ", 3.0, "schneiden", 250.0)));
        assert_eq!(lib.entries.len(), 2);
        assert_eq!(lib.materials(), vec!["Acryl".to_string(), "sperrholz ".to_string()]);
        assert_eq!(lib.entries[1].settings.speed, 250.0);
    }

    #[test]
    fn json_round_trip_and_format_check() {
        let mut lib = MaterialLibrary::default();
        lib.upsert(entry("Leder", 0.0, "Gravieren", 6000.0));
        let text = lib.to_json().unwrap();
        assert_eq!(MaterialLibrary::from_json(&text).unwrap(), lib);
        assert!(MaterialLibrary::from_json(r#"{"format":"x","version":1,"entries":[]}"#).is_err());
        assert_eq!(lib.entries[0].thickness_label(), "beliebig");
        assert_eq!(entry("a", 2.5, "b", 1.0).thickness_label(), "2.5 mm");
    }
}
