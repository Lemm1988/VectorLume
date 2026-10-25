//! Zugriffe auf das Betriebssystem außerhalb des Programmfensters: Dateiauswahl über `rfd` und das Starten anderer
//! Programme (Explorer, Editor). In Tests passiert nichts davon – ein echtes Fenster würde den Test blockieren oder
//! auf dem Desktop des Benutzers erscheinen; eine Dateiauswahl gilt dort als abgebrochen.

use std::path::{Path, PathBuf};

pub struct FileDialog(rfd::FileDialog);

impl FileDialog {
    pub fn new() -> Self {
        Self(rfd::FileDialog::new())
    }

    pub fn add_filter(self, name: impl Into<String>, extensions: &[impl ToString]) -> Self {
        Self(self.0.add_filter(name, extensions))
    }

    pub fn set_file_name(self, file_name: impl Into<String>) -> Self {
        Self(self.0.set_file_name(file_name))
    }

    pub fn set_title(self, title: impl Into<String>) -> Self {
        Self(self.0.set_title(title))
    }

    pub fn pick_file(self) -> Option<PathBuf> {
        if cfg!(test) { None } else { self.0.pick_file() }
    }

    pub fn pick_files(self) -> Option<Vec<PathBuf>> {
        if cfg!(test) { None } else { self.0.pick_files() }
    }

    pub fn save_file(self) -> Option<PathBuf> {
        if cfg!(test) { None } else { self.0.save_file() }
    }
}

/// Zeigt eine Datei oder einen Ordner im Explorer.
pub fn show_in_explorer(path: &Path) {
    launch("explorer", path);
}

/// Öffnet eine Textdatei im Standard-Editor.
pub fn open_in_editor(path: &Path) {
    launch(if cfg!(windows) { "notepad" } else { "xdg-open" }, path);
}

fn launch(program: &str, path: &Path) {
    if !cfg!(test) {
        let _ = std::process::Command::new(program).arg(path).spawn();
    }
}
