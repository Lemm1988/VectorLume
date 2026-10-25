//! Fehlerbericht: Bei einem Programmfehler (Panic) werden Version, System, Meldung, Ort und Aufrufliste an
//! `crash.log` im Einstellungsordner angehängt. Beim nächsten Start weist das Programm darauf hin; ein Fehler in einem
//! Hintergrund-Thread wird sofort gemeldet, weil das Programm dabei weiterläuft.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

const FILE: &str = "crash.log";
/// Hierhin wandert der Bericht, sobald er gemeldet wurde.
const SEEN: &str = "crash-letzter.log";

static NEW_REPORT: AtomicBool = AtomicBool::new(false);

fn report_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join(FILE))
}

/// Text des Berichts für einen Fehler.
fn report(message: &str, location: &str, thread: &str, backtrace: &str) -> String {
    format!(
        "=== VectorLume {} – {} ===\nSystem: {} {}\nThread: {thread}\nFehler: {message}\nOrt: {location}\nAufrufliste:\n{backtrace}\n",
        env!("CARGO_PKG_VERSION"),
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S %z"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}

/// Richtet den Bericht ein; die bisherige Ausgabe auf der Konsole bleibt erhalten.
pub fn install() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(unbekannt)".into());
        let location = info.location().map(|l| l.to_string()).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        if let Some(path) = report_path() {
            let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                let _ = f.write_all(report(&message, &location, &thread, &backtrace).as_bytes());
            }
        }
        NEW_REPORT.store(true, Ordering::Relaxed);
        default(info);
    }));
}

/// Ob seit der letzten Abfrage ein Fehler aufgetreten ist (Programm läuft noch, z. B. Fehler im Hintergrund).
pub fn take_new() -> Option<PathBuf> {
    NEW_REPORT.swap(false, Ordering::Relaxed).then(report_path).flatten()
}

/// Bericht aus einer früheren Sitzung: wird als gemeldet abgelegt und sein neuer Pfad geliefert.
pub fn take_previous() -> Option<PathBuf> {
    let path = report_path().filter(|p| p.is_file())?;
    let seen = path.with_file_name(SEEN);
    std::fs::rename(&path, &seen).ok().map(|_| seen)
}

/// Öffnet den Bericht im Standard-Editor.
pub fn open(path: &std::path::Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("notepad").arg(path).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_names_version_place_and_message() {
        let r = report("index out of bounds", "app/src/canvas.rs:10:5", "main", "0: main");
        assert!(r.starts_with(&format!("=== VectorLume {} – ", env!("CARGO_PKG_VERSION"))));
        assert!(r.contains("Fehler: index out of bounds\nOrt: app/src/canvas.rs:10:5\n"));
        assert!(r.contains("Thread: main\n") && r.ends_with("0: main\n"));
    }
}
