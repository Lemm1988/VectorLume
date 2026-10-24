//! CSV-Dateien für Serienbriefe lesen: Trennzeichen `,` `;` oder Tabulator, Felder in Anführungszeichen
//! (auch mit Zeilenumbrüchen), UTF-8 mit/ohne BOM oder Windows-1252 (übliche Excel-Ausgabe).

use vl_core::DataList;

/// Liest eine CSV-Datei; die erste Zeile enthält die Spaltennamen.
pub fn parse_csv(bytes: &[u8], source: &str) -> Result<DataList, String> {
    let text = decode(bytes);
    let sep = detect_separator(&text);
    let mut records = parse_records(&text, sep);
    records.retain(|r| r.iter().any(|f| !f.trim().is_empty()));
    if records.is_empty() {
        return Err("Die Datei ist leer.".into());
    }
    let header_row = records.remove(0);
    let width = records.iter().map(Vec::len).max().unwrap_or(0).max(header_row.len());
    let mut headers: Vec<String> = Vec::with_capacity(width);
    for i in 0..width {
        let h = header_row.get(i).map(|h| h.trim()).unwrap_or("");
        let mut name = if h.is_empty() { format!("Spalte {}", i + 1) } else { h.replace(['{', '}'], "") };
        // Doppelte Namen unterscheidbar machen
        if headers.contains(&name) {
            name = format!("{name} ({})", i + 1);
        }
        headers.push(name);
    }
    if records.is_empty() {
        return Err("Die Datei enthält nur Spaltennamen, aber keine Datenzeilen.".into());
    }
    let rows = records
        .into_iter()
        .map(|mut r| {
            r.resize(width, String::new());
            r.into_iter().map(|f| f.trim().to_string()).collect()
        })
        .collect();
    Ok(DataList { source: source.to_string(), headers, rows, row: 0 })
}

fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| windows_1252(b)).collect(),
    }
}

/// Windows-1252: 0x80–0x9F weichen von Latin-1 ab, der Rest entspricht Unicode.
fn windows_1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}', '\u{90}', '‘',
        '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9F => HIGH[(b - 0x80) as usize],
        _ => b as char,
    }
}

/// Häufigstes Trennzeichen außerhalb von Anführungszeichen in der ersten Zeile.
fn detect_separator(text: &str) -> char {
    let mut counts = [(';', 0usize), (',', 0), ('\t', 0)];
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            '\n' | '\r' if !quoted => break,
            _ if !quoted => {
                for (sep, n) in &mut counts {
                    if c == *sep {
                        *n += 1;
                    }
                }
            }
            _ => {}
        }
    }
    counts.iter().max_by_key(|(_, n)| *n).filter(|(_, n)| *n > 0).map(|(c, _)| *c).unwrap_or(',')
}

fn parse_records(text: &str, sep: char) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.trim().is_empty() => {
                field.clear();
                quoted = true;
            }
            c if c == sep => record.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semicolon_quotes_and_umlauts() {
        let csv = "\u{FEFF}Name;Ort;Notiz\r\n\"Müller, Anna\";Köln;\"sagt \"\"Hallo\"\"\"\r\n\r\nBen;\"Bad\nTölz\"\r\n";
        let d = parse_csv(csv.as_bytes(), "t.csv").unwrap();
        assert_eq!(d.headers, vec!["Name", "Ort", "Notiz"]);
        assert_eq!(d.rows.len(), 2);
        assert_eq!(d.rows[0], vec!["Müller, Anna", "Köln", "sagt \"Hallo\""]);
        assert_eq!(d.rows[1], vec!["Ben", "Bad\nTölz", ""]);
    }

    #[test]
    fn windows_1252_and_missing_headers() {
        // „Größe,,Preis“ / „XL,1,9 €“ in Windows-1252
        let bytes = b"Gr\xF6\xDFe,,Preis\nXL,1,9 \x80\n";
        let d = parse_csv(bytes, "w.csv").unwrap();
        assert_eq!(d.headers, vec!["Größe", "Spalte 2", "Preis"]);
        assert_eq!(d.rows[0], vec!["XL", "1", "9 €"]);
        assert!(parse_csv(b"Nur;Kopf\n", "k.csv").is_err());
        assert!(parse_csv(b"", "e.csv").is_err());
    }
}
