//! EPS-Import über ein installiertes Ghostscript: die Datei wird in ein PDF umgewandelt (Texte als Umrisse)
//! und dann wie ein PDF gelesen. Ghostscript wird nur als eigenständiges Programm aufgerufen.

use crate::Imported;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Längste Rechenzeit für Ghostscript.
const TIMEOUT: Duration = Duration::from_secs(60);

pub fn import_eps(data: &[u8]) -> Result<Imported, String> {
    let ps = postscript_part(data)?;
    let gs = find_ghostscript().ok_or(
        "EPS-Dateien benötigen Ghostscript (kostenlos, ghostscript.com). Alternativ die Datei im \
         Zeichenprogramm als PDF oder SVG speichern.",
    )?;
    let dir = std::env::temp_dir().join(format!("vectorlume-eps-{}-{}", std::process::id(), unique()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = convert(&gs, &dir, ps).and_then(|pdf| crate::pdf::import_pdf(&pdf));
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

fn convert(gs: &Path, dir: &Path, ps: &[u8]) -> Result<Vec<u8>, String> {
    let input = dir.join("eingabe.eps");
    let output = dir.join("ausgabe.pdf");
    std::fs::write(&input, ps).map_err(|e| e.to_string())?;
    let mut cmd = Command::new(gs);
    cmd.args(["-dNOPAUSE", "-dBATCH", "-dSAFER", "-dQUIET", "-sDEVICE=pdfwrite", "-dEPSCrop", "-dNoOutputFonts"])
        .arg(format!("-sOutputFile={}", output.display()))
        .arg(&input)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| format!("Ghostscript konnte nicht gestartet werden: {e}"))?;
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Ghostscript hat zu lange gebraucht – die EPS-Datei ist zu komplex oder fehlerhaft.".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if !status.success() {
        let mut msg = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = std::io::Read::read_to_string(&mut e, &mut msg);
        }
        let first = msg.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("unbekannter Fehler");
        return Err(format!("Die EPS-Datei konnte nicht gelesen werden ({first})."));
    }
    std::fs::read(&output).map_err(|e| format!("Ghostscript hat kein PDF erzeugt: {e}"))
}

/// PostScript-Teil der Datei; „DOS-EPS“ mit Binärkopf und Vorschaubild wird zerlegt.
fn postscript_part(data: &[u8]) -> Result<&[u8], String> {
    if data.starts_with(&[0xC5, 0xD0, 0xD3, 0xC6]) && data.len() >= 12 {
        let word = |i: usize| u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
        let (offset, len) = (word(4), word(8));
        return data.get(offset..offset.saturating_add(len)).ok_or_else(|| "EPS-Datei ist beschädigt.".to_string());
    }
    if data.starts_with(b"%!") { Ok(data) } else { Err("Die Datei ist keine EPS-Datei.".into()) }
}

/// Sucht Ghostscript: `VECTORLUME_GHOSTSCRIPT`, Suchpfad, Standardinstallation, PDF24.
pub fn find_ghostscript() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("VECTORLUME_GHOSTSCRIPT").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let names: &[&str] = if cfg!(windows) { &["gswin64c.exe", "gswin32c.exe"] } else { &["gs"] };
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            for n in names {
                let p = dir.join(n);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    let mut roots: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    roots.dedup();
    for root in &roots {
        // Neueste Version zuerst (gs10.04.0 vor gs9.56.1)
        let mut versions: Vec<PathBuf> =
            std::fs::read_dir(root.join("gs")).into_iter().flatten().flatten().map(|e| e.path()).collect();
        versions.sort_by_key(|p| std::cmp::Reverse(version_key(p)));
        let pdf24 = root.join("PDF24").join("gs");
        for dir in versions.iter().chain(std::iter::once(&pdf24)) {
            for n in names {
                let p = dir.join("bin").join(n);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn version_key(p: &Path) -> Vec<u32> {
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.trim_start_matches("gs").split('.').map(|s| s.parse().unwrap_or(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ImportedItem;

    const EPS: &str = "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 60\n\
        newpath 10 10 moveto 60 10 lineto 60 50 lineto 10 50 lineto closepath 0 0 1 setrgbcolor stroke\n\
        newpath 80 30 15 0 360 arc closepath 1 0 0 setrgbcolor fill\n\
        /Helvetica findfont 12 scalefont setfont 0 setgray 15 25 moveto (VL) show\nshowpage\n";

    #[test]
    fn dos_eps_header_is_stripped() {
        let mut data = vec![0xC5, 0xD0, 0xD3, 0xC6];
        data.extend_from_slice(&32u32.to_le_bytes());
        data.extend_from_slice(&5u32.to_le_bytes());
        data.resize(32, 0);
        data.extend_from_slice(b"%!PS-rest");
        assert_eq!(postscript_part(&data).unwrap(), b"%!PS-");
        assert!(postscript_part(b"GIF89a").is_err());
        assert_eq!(version_key(Path::new("C:/gs/gs10.04.0")), vec![10, 4, 0]);
    }

    /// Läuft nur, wenn Ghostscript installiert ist.
    #[test]
    fn converts_with_ghostscript_if_installed() {
        if find_ghostscript().is_none() {
            eprintln!("Ghostscript nicht gefunden – Test übersprungen");
            return;
        }
        let im = import_eps(EPS.as_bytes()).unwrap();
        let paths: Vec<_> = im
            .items
            .iter()
            .filter_map(|i| if let ImportedItem::Path { path, layer, .. } = i { Some((path, *layer)) } else { None })
            .collect();
        // Rechteck, Kreis und Text (als Umrisse, zwei Buchstaben)
        assert!(paths.len() >= 3, "{} Pfade", paths.len());
        let blue = crate::layer_for_color([0, 0, 255]);
        let red = crate::layer_for_color([255, 0, 0]);
        assert_ne!(blue, red);
        assert!(paths.iter().any(|(_, l)| *l == blue), "blaue Kontur");
        assert!(paths.iter().any(|(_, l)| *l == red), "roter Kreis");
        assert!(paths.iter().any(|(_, l)| l.is_none()), "schwarzer Text auf der aktuellen Ebene");
    }
}
