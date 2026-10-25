//! GRBL-Parameter (`$n=Wert`): Beschreibungen der Standardparameter von GRBL 1.1, Sicherungsdatei lesen und
//! schreiben, geänderte Werte ermitteln.

use std::collections::BTreeMap;

/// Beschreibung und Einheit eines Standardparameters.
pub fn describe(n: u32) -> Option<(&'static str, &'static str)> {
    Some(match n {
        0 => ("Schrittimpuls-Dauer", "µs"),
        1 => ("Motoren halten nach Bewegung (255 = immer)", "ms"),
        2 => ("Schrittimpuls invertieren", "Maske"),
        3 => ("Drehrichtung invertieren", "Maske"),
        4 => ("Motor-Freigabe invertieren", "0/1"),
        5 => ("Endschalter invertieren", "0/1"),
        6 => ("Tastsonde invertieren", "0/1"),
        10 => ("Inhalt der Statusmeldung", "Maske"),
        11 => ("Eckenabweichung", "mm"),
        12 => ("Bogengenauigkeit", "mm"),
        13 => ("Meldungen in Zoll", "0/1"),
        20 => ("Software-Endschalter", "0/1"),
        21 => ("Hardware-Endschalter", "0/1"),
        22 => ("Referenzfahrt", "0/1"),
        23 => ("Richtung der Referenzfahrt invertieren", "Maske"),
        24 => ("Referenzfahrt langsam", "mm/min"),
        25 => ("Referenzfahrt schnell", "mm/min"),
        26 => ("Entprellzeit der Endschalter", "ms"),
        27 => ("Rückzug nach der Referenzfahrt", "mm"),
        30 => ("Höchstwert S (volle Leistung)", "S"),
        31 => ("Kleinster Wert S", "S"),
        32 => ("Laser-Modus", "0/1"),
        100 => ("Schritte je mm X", "Schritte/mm"),
        101 => ("Schritte je mm Y", "Schritte/mm"),
        102 => ("Schritte je mm Z", "Schritte/mm"),
        110 => ("Höchstgeschwindigkeit X", "mm/min"),
        111 => ("Höchstgeschwindigkeit Y", "mm/min"),
        112 => ("Höchstgeschwindigkeit Z", "mm/min"),
        120 => ("Beschleunigung X", "mm/s²"),
        121 => ("Beschleunigung Y", "mm/s²"),
        122 => ("Beschleunigung Z", "mm/s²"),
        130 => ("Verfahrweg X", "mm"),
        131 => ("Verfahrweg Y", "mm"),
        132 => ("Verfahrweg Z", "mm"),
        _ => return None,
    })
}

/// Ob `v` ein gültiger Parameterwert ist (GRBL kennt nur Zahlen).
pub fn is_valid(v: &str) -> bool {
    v.trim().parse::<f64>().is_ok_and(f64::is_finite)
}

/// Zwei Werte gelten als gleich, wenn sie dieselbe Zahl darstellen (`8` und `8.000`).
fn same(a: &str, b: &str) -> bool {
    match (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
        (Ok(x), Ok(y)) => (x - y).abs() <= 1e-9 * x.abs().max(1.0),
        _ => a.trim() == b.trim(),
    }
}

/// Werte aus `edited`, die sich von `device` unterscheiden und gültig sind, als Befehle `$n=Wert`.
pub fn changes(device: &BTreeMap<u32, String>, edited: &BTreeMap<u32, String>) -> Vec<String> {
    edited
        .iter()
        .filter(|(n, v)| is_valid(v) && device.get(n).is_none_or(|d| !same(d, v)))
        .map(|(n, v)| format!("${n}={}", v.trim()))
        .collect()
}

/// Ob der bearbeitete Wert von dem der Steuerung abweicht.
pub fn differs(device: &BTreeMap<u32, String>, n: u32, v: &str) -> bool {
    device.get(&n).is_none_or(|d| !same(d, v))
}

/// Sicherungsdatei: eine Zeile `$n=Wert` je Parameter, Kommentarzeile vorweg.
pub fn to_file(values: &BTreeMap<u32, String>) -> String {
    let mut s = String::from("; GRBL-Parameter, gesichert mit VectorLume\n");
    for (n, v) in values {
        s.push_str(&format!("${n}={}\n", v.trim()));
    }
    s
}

/// Liest eine Sicherung bzw. eine kopierte `$$`-Ausgabe. Kommentare (`;` oder `(…)`), Leerzeilen und `ok` werden
/// übergangen; ungültige Zeilen führen zu einem Fehler mit Zeilennummer.
pub fn from_file(text: &str) -> Result<BTreeMap<u32, String>, String> {
    let mut out = BTreeMap::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.split(';').next().unwrap_or("");
        let line = line.split('(').next().unwrap_or("").trim();
        if line.is_empty() || line.eq_ignore_ascii_case("ok") {
            continue;
        }
        let parsed = line
            .strip_prefix('$')
            .and_then(|r| r.split_once('='))
            .and_then(|(n, v)| Some((n.trim().parse::<u32>().ok()?, v.trim())))
            .filter(|(_, v)| is_valid(v));
        match parsed {
            Some((n, v)) => {
                out.insert(n, v.to_string());
            }
            None => return Err(format!("Zeile {}: „{}“ ist kein GRBL-Parameter.", i + 1, raw.trim())),
        }
    }
    if out.is_empty() {
        return Err("Die Datei enthält keine GRBL-Parameter.".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(items: &[(u32, &str)]) -> BTreeMap<u32, String> {
        items.iter().map(|(n, v)| (*n, v.to_string())).collect()
    }

    #[test]
    fn only_real_changes_are_written() {
        let device = map(&[(30, "1000"), (32, "1"), (130, "400.000")]);
        let edited = map(&[(30, "1000.0"), (32, "1"), (130, "410"), (131, "x"), (110, "6000")]);
        assert_eq!(changes(&device, &edited), vec!["$110=6000", "$130=410"]);
        assert!(!differs(&device, 130, "400"));
        assert!(differs(&device, 130, "401"));
    }

    #[test]
    fn backup_round_trip_and_dollar_output() {
        let values = map(&[(0, "10"), (32, "1"), (130, "400.000")]);
        assert_eq!(from_file(&to_file(&values)).unwrap(), values);
        // Ausgabe von `$$` (ältere GRBL-Versionen mit Beschreibung in Klammern) samt „ok“
        let dump = "$0=10 (step pulse, usec)\r\n$32=1\r\nok\r\n";
        assert_eq!(from_file(dump).unwrap(), map(&[(0, "10"), (32, "1")]));
        assert!(from_file("G1 X10\n").unwrap_err().starts_with("Zeile 1:"));
        assert!(from_file("; nur Kommentar\n").is_err());
        assert_eq!(describe(130), Some(("Verfahrweg X", "mm")));
        assert_eq!(describe(7), None);
    }
}
