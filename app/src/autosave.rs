//! Automatische Sicherung: ungespeicherte Änderungen werden regelmäßig gesichert und nach einem Absturz angeboten.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use vl_core::Document;

/// Abstand zwischen zwei Sicherungen (für Tests über `VECTORLUME_AUTOSAVE_SECS` änderbar).
fn interval() -> Duration {
    let secs = std::env::var("VECTORLUME_AUTOSAVE_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(90);
    Duration::from_secs(secs)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Meta {
    /// Projektdatei, zu der die Sicherung gehört (None = unbenannt).
    pub original: Option<PathBuf>,
    /// Zeitpunkt der Sicherung (Sekunden seit 1970).
    pub saved_at: u64,
}

impl Meta {
    /// „vor 3 Minuten“ o. Ä.
    pub fn age_label(&self) -> String {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let mins = now.saturating_sub(self.saved_at) / 60;
        match mins {
            0 => "vor weniger als einer Minute".into(),
            1 => "vor 1 Minute".into(),
            m if m < 120 => format!("vor {m} Minuten"),
            m if m < 48 * 60 => format!("vor {} Stunden", m / 60),
            m => format!("vor {} Tagen", m / 60 / 24),
        }
    }
}

fn dir() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("autosave"))
}

fn files() -> Option<(PathBuf, PathBuf)> {
    let d = dir()?;
    Some((d.join("autosave.vlume"), d.join("autosave.json")))
}

/// Vorhandene Sicherung aus einer früheren Sitzung.
pub fn find() -> Option<(PathBuf, Meta)> {
    let (project, meta) = files()?;
    if !project.exists() {
        return None;
    }
    let meta: Meta = std::fs::read_to_string(meta)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Meta { original: None, saved_at: 0 });
    Some((project, meta))
}

pub struct Autosave {
    last_revision: u64,
    last_time: Instant,
    busy: Arc<AtomicBool>,
}

impl Default for Autosave {
    fn default() -> Self {
        Self { last_revision: 0, last_time: Instant::now(), busy: Arc::new(AtomicBool::new(false)) }
    }
}

impl Autosave {
    /// Sichert im Hintergrund, wenn es ungespeicherte Änderungen gibt und das Intervall abgelaufen ist.
    pub fn tick(&mut self, doc: &Document, revision: u64, dirty: bool, original: Option<&Path>) {
        if !dirty
            || doc.objects.is_empty()
            || revision == self.last_revision
            || self.last_time.elapsed() < interval()
            || self.busy.load(Ordering::Acquire)
        {
            return;
        }
        let Some((project, meta_path)) = files() else { return };
        self.last_revision = revision;
        self.last_time = Instant::now();
        self.busy.store(true, Ordering::Release);
        let busy = self.busy.clone();
        let doc = doc.clone();
        let meta = Meta {
            original: original.map(Path::to_path_buf),
            saved_at: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        };
        std::thread::spawn(move || {
            let result = (|| -> Result<(), String> {
                if let Some(d) = project.parent() {
                    std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
                }
                vl_core::project::save_project(&doc, &project)?;
                let text = serde_json::to_string(&meta).map_err(|e| e.to_string())?;
                std::fs::write(&meta_path, text).map_err(|e| e.to_string())
            })();
            if let Err(e) = result {
                log::warn!("Automatische Sicherung fehlgeschlagen: {e}");
            }
            busy.store(false, Ordering::Release);
        });
    }

    /// Löscht die Sicherung (nach dem Speichern, beim Verwerfen und beim regulären Beenden).
    pub fn clear(&mut self) {
        self.last_time = Instant::now();
        // Eine laufende Sicherung abwarten, sonst würde sie die Datei gleich wieder anlegen.
        let start = Instant::now();
        while self.busy.load(Ordering::Acquire) && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        if let Some((project, meta)) = files() {
            let _ = std::fs::remove_file(project);
            let _ = std::fs::remove_file(meta);
        }
    }
}
