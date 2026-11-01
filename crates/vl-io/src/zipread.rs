//! Kleiner ZIP-Leser für Projektdateien (z. B. xTool Studio `.xs`): nur Lesen, nur „gespeichert“ und „Deflate“,
//! mit Prüfsumme und Größengrenzen, damit beschädigte oder präparierte Dateien weder abstürzen noch den Speicher
//! füllen.

use std::io::Read;

/// Größte entpackte Datei im Archiv.
const MAX_ENTRY: u64 = 256 << 20;
/// Größte Summe aller entpackten Dateien, die gelesen werden.
const MAX_TOTAL: u64 = 1 << 30;

pub struct Archive<'a> {
    data: &'a [u8],
    entries: Vec<Entry>,
    read_total: std::cell::Cell<u64>,
}

struct Entry {
    name: String,
    method: u16,
    crc: u32,
    compressed: u64,
    size: u64,
    header: u64,
}

pub fn is_zip(data: &[u8]) -> bool {
    data.starts_with(b"PK\x03\x04")
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    d.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

impl<'a> Archive<'a> {
    pub fn open(data: &'a [u8]) -> Result<Self, String> {
        let bad = || "Das ZIP-Archiv ist beschädigt.".to_string();
        // Ende des Inhaltsverzeichnisses: höchstens 64 KiB Kommentar dahinter
        let from = data.len().saturating_sub(22 + 0xFFFF);
        let eocd = (from..data.len().saturating_sub(21))
            .rev()
            .find(|&i| u32_at(data, i) == Some(0x0605_4b50))
            .ok_or_else(bad)?;
        let count = u16_at(data, eocd + 10).ok_or_else(bad)? as usize;
        let cd_offset = u32_at(data, eocd + 16).ok_or_else(bad)?;
        if cd_offset == u32::MAX || count == 0xFFFF {
            return Err("ZIP64-Archive werden nicht unterstützt.".into());
        }
        let mut entries = Vec::with_capacity(count.min(4096));
        let mut at = cd_offset as usize;
        for _ in 0..count {
            if u32_at(data, at) != Some(0x0201_4b50) {
                return Err(bad());
            }
            let flags = u16_at(data, at + 8).ok_or_else(bad)?;
            let method = u16_at(data, at + 10).ok_or_else(bad)?;
            let crc = u32_at(data, at + 16).ok_or_else(bad)?;
            let compressed = u32_at(data, at + 20).ok_or_else(bad)? as u64;
            let size = u32_at(data, at + 24).ok_or_else(bad)? as u64;
            let name_len = u16_at(data, at + 28).ok_or_else(bad)? as usize;
            let extra_len = u16_at(data, at + 30).ok_or_else(bad)? as usize;
            let comment_len = u16_at(data, at + 32).ok_or_else(bad)? as usize;
            let header = u32_at(data, at + 42).ok_or_else(bad)? as u64;
            let name = data.get(at + 46..at + 46 + name_len).ok_or_else(bad)?;
            if flags & 1 == 0 {
                entries.push(Entry {
                    name: String::from_utf8_lossy(name).replace('\\', "/"),
                    method,
                    crc,
                    compressed,
                    size,
                    header,
                });
            }
            at += 46 + name_len + extra_len + comment_len;
        }
        Ok(Self { data, entries, read_total: std::cell::Cell::new(0) })
    }

    /// Namen aller (unverschlüsselten) Dateien im Archiv.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.iter().any(|e| e.name == name)
    }

    /// Entpackt eine Datei; prüft Größe und Prüfsumme.
    pub fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        let e = self.entries.iter().find(|e| e.name == name).ok_or_else(|| format!("„{name}“ fehlt im Archiv."))?;
        let bad = || format!("„{name}“ im Archiv ist beschädigt.");
        if e.size > MAX_ENTRY || self.read_total.get() + e.size > MAX_TOTAL {
            return Err(format!("„{name}“ ist zu groß."));
        }
        let h = usize::try_from(e.header).map_err(|_| bad())?;
        if u32_at(self.data, h) != Some(0x0403_4b50) {
            return Err(bad());
        }
        let start = h
            + 30
            + u16_at(self.data, h + 26).ok_or_else(bad)? as usize
            + u16_at(self.data, h + 28).ok_or_else(bad)? as usize;
        let end = start.checked_add(usize::try_from(e.compressed).map_err(|_| bad())?).ok_or_else(bad)?;
        let raw = self.data.get(start..end).ok_or_else(bad)?;
        let out = match e.method {
            0 => raw.to_vec(),
            8 => {
                let mut out = Vec::with_capacity(e.size as usize);
                flate2::read::DeflateDecoder::new(raw).take(e.size + 1).read_to_end(&mut out).map_err(|_| bad())?;
                out
            }
            m => return Err(format!("„{name}“: Packverfahren {m} wird nicht unterstützt.")),
        };
        if out.len() as u64 != e.size {
            return Err(bad());
        }
        let mut crc = flate2::Crc::new();
        crc.update(&out);
        if crc.sum() != e.crc {
            return Err(bad());
        }
        self.read_total.set(self.read_total.get() + e.size);
        Ok(out)
    }
}

/// Schreibt ein Archiv (nur für Tests); `deflate` packt die Dateien.
#[doc(hidden)]
pub fn write(files: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    use std::io::Write;
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, content) in files {
        let mut crc = flate2::Crc::new();
        crc.update(content);
        let packed = if deflate {
            let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            enc.write_all(content).unwrap();
            enc.finish().unwrap()
        } else {
            content.to_vec()
        };
        let method: u16 = if deflate { 8 } else { 0 };
        let offset = out.len() as u32;
        let common = |v: &mut Vec<u8>| {
            v.extend_from_slice(&20u16.to_le_bytes());
            v.extend_from_slice(&0u16.to_le_bytes());
            v.extend_from_slice(&method.to_le_bytes());
            v.extend_from_slice(&[0, 0, 0, 0]);
            v.extend_from_slice(&crc.sum().to_le_bytes());
            v.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            v.extend_from_slice(&(content.len() as u32).to_le_bytes());
            v.extend_from_slice(&(name.len() as u16).to_le_bytes());
            v.extend_from_slice(&0u16.to_le_bytes());
        };
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        common(&mut out);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&packed);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        common(&mut central);
        // Kommentarlänge, Datenträger, interne und externe Attribute
        central.extend_from_slice(&[0; 10]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_stored_and_deflated_files() {
        for deflate in [false, true] {
            let zip = write(&[("a.json", b"{\"x\":1}"), ("dir/b.txt", &[7u8; 5000])], deflate);
            assert!(is_zip(&zip));
            let a = Archive::open(&zip).unwrap();
            assert_eq!(a.names().collect::<Vec<_>>(), ["a.json", "dir/b.txt"]);
            assert_eq!(a.read("a.json").unwrap(), b"{\"x\":1}");
            assert_eq!(a.read("dir/b.txt").unwrap(), vec![7u8; 5000]);
            assert!(a.read("fehlt").is_err());
        }
    }

    #[test]
    fn damaged_archives_are_rejected() {
        let zip = write(&[("a.txt", b"Hallo Welt")], false);
        // Inhalt verfälscht: Prüfsumme stimmt nicht mehr
        let mut bad = zip.clone();
        let pos = bad.windows(5).position(|w| w == b"Hallo").unwrap();
        bad[pos] = b'X';
        assert!(Archive::open(&bad).unwrap().read("a.txt").is_err());
        assert!(Archive::open(&zip[..zip.len() - 5]).is_err());
        assert!(Archive::open(b"PK\x03\x04").is_err());
    }
}
