//! Bettet unter Windows Programmsymbol und Versionsinfos in die vectorlume.exe ein.

include!("src/icon_art.rs");

fn main() {
    println!("cargo:rerun-if-changed=src/icon_art.rs");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico = out.join("vectorlume.ico");
    std::fs::write(&ico, ico_bytes(&[16, 20, 24, 32, 40, 48, 64, 128, 256])).expect("Symbol schreiben");

    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico.to_str().unwrap())
        .set("ProductName", "VectorLume")
        .set("FileDescription", "VectorLume - Laser-Software")
        .set("LegalCopyright", "Alle Rechte vorbehalten");
    if let Err(e) = res.compile() {
        // Ohne Windows-SDK (rc.exe) bleibt die .exe ohne Symbol, der Build läuft trotzdem.
        println!("cargo:warning=Programmsymbol nicht eingebettet: {e}");
    }
}

/// ICO-Datei mit unkomprimierten 32-Bit-Bildern (BGRA, von unten nach oben).
fn ico_bytes(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&s| dib(s)).collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&s, img) in sizes.iter().zip(&images) {
        let dim = if s >= 256 { 0 } else { s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend_from_slice(&img);
    }
    out
}

fn dib(size: u32) -> Vec<u8> {
    let rgba = icon_rgba(size);
    let mask_row = size.div_ceil(32) * 4;
    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&(2 * size as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]);
    for y in (0..size).rev() {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            out.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    // UND-Maske: alles 0, die Transparenz steckt im Alphakanal.
    out.extend(std::iter::repeat_n(0u8, (mask_row * size) as usize));
    out
}
