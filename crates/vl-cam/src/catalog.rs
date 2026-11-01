//! Gerätekatalog: Vorlagen für verbreitete Laser mit GRBL-, Marlin- oder Smoothieware-Steuerung.
//!
//! Die Werte sind Ausgangspunkte. Beim Verbinden liest VectorLume Arbeitsfläche (`$130`/`$131`) und Leistungsbereich
//! (`$30`) aus GRBL-Steuerungen und übernimmt sie; die Nullpunkt-Ecke prüft man am besten mit „Rahmen“. Laser mit
//! eigenem, nicht offenem Protokoll stehen in [`UNSUPPORTED`].

use crate::machine::{MachineProfile, OriginCorner};
use vl_core::Firmware;

/// Hersteller der allgemeinen Einträge (immer zuerst gelistet).
pub const GENERIC: &str = "Allgemein";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Model {
    pub maker: &'static str,
    pub name: &'static str,
    pub firmware: Firmware,
    /// Arbeitsfläche in mm.
    pub width: f64,
    pub height: f64,
    pub origin: OriginCorner,
    /// Wert für volle Leistung.
    pub s_max: f64,
    /// M4 (Leistung folgt der Geschwindigkeit) statt M3.
    pub dynamic_power: bool,
    /// Kurzer Hinweis zur Einrichtung (deutsch, wird übersetzt angezeigt); leer = keiner.
    pub hint: &'static str,
}

/// Diodenlaser mit GRBL: Nullpunkt vorne links, `$30` = 1000, M4.
const fn grbl(maker: &'static str, name: &'static str, width: f64, height: f64) -> Model {
    Model {
        maker,
        name,
        firmware: Firmware::Grbl,
        width,
        height,
        origin: OriginCorner::FrontLeft,
        s_max: 1000.0,
        dynamic_power: true,
        hint: "",
    }
}

const fn with_hint(m: Model, hint: &'static str) -> Model {
    Model { hint, ..m }
}

const K40_HINT: &str = "Nur mit getauschter Steuerung (z. B. Cohesion3D, Awesome.Tech). Die Originalplatine (M2 Nano) \
                        spricht kein G-Code.";

/// Alle Vorlagen, nach Hersteller gruppiert; [`GENERIC`] zuerst.
pub const MODELS: &[Model] = &[
    with_hint(
        grbl(GENERIC, "GRBL-Laser", 400.0, 400.0),
        "Für fast alle Diodenlaser: Arbeitsfläche und Leistungsbereich werden beim Verbinden ausgelesen.",
    ),
    with_hint(
        Model { dynamic_power: false, ..grbl(GENERIC, "GRBL-Laser mit M3 (konstante Leistung)", 400.0, 400.0) },
        "Für ältere GRBL-Versionen ohne Laser-Modus und für CO₂-Laser, die gleichmäßige Leistung brauchen.",
    ),
    with_hint(grbl(GENERIC, "FluidNC / grblHAL", 400.0, 400.0), "GRBL-kompatibel, auch über WLAN (Telnet, Port 23)."),
    with_hint(
        Model {
            firmware: Firmware::Marlin,
            s_max: 255.0,
            dynamic_power: false,
            ..grbl(GENERIC, "Marlin", 220.0, 220.0)
        },
        "3D-Drucker und Plotter mit Laser-Aufsatz (Marlin 2.1 mit LASER_FEATURE).",
    ),
    with_hint(
        Model {
            firmware: Firmware::Smoothieware,
            s_max: 1.0,
            dynamic_power: false,
            ..grbl(GENERIC, "Smoothieware", 300.0, 200.0)
        },
        "Smoothieboard und kompatible Steuerungen; Leistung als S 0…1.",
    ),
    // Sculpfun
    grbl("Sculpfun", "SF-A9 40W", 400.0, 400.0),
    grbl("Sculpfun", "S9", 410.0, 420.0),
    grbl("Sculpfun", "S10", 410.0, 400.0),
    grbl("Sculpfun", "S30 / S30 Pro", 410.0, 400.0),
    grbl("Sculpfun", "S30 Pro Max", 410.0, 400.0),
    grbl("Sculpfun", "S30 Ultra", 600.0, 600.0),
    grbl("Sculpfun", "iCube", 130.0, 130.0),
    // Ortur
    grbl("Ortur", "Laser Master 2", 400.0, 430.0),
    grbl("Ortur", "Laser Master 2 Pro / S2", 400.0, 400.0),
    grbl("Ortur", "Laser Master 3", 400.0, 380.0),
    grbl("Ortur", "Laser Master H10", 400.0, 400.0),
    grbl("Ortur", "Aufero Laser 1 / 2", 390.0, 390.0),
    // Atomstack
    grbl("Atomstack", "A5 / A5 Pro", 410.0, 400.0),
    grbl("Atomstack", "A10 Pro / S10 Pro / X7 Pro", 410.0, 400.0),
    grbl("Atomstack", "A20 Pro / S20 Pro / X20 Pro", 400.0, 400.0),
    grbl("Atomstack", "A30 Pro / X30 Pro", 400.0, 400.0),
    grbl("Atomstack", "A40 Pro / S40 Pro / X40 Pro", 400.0, 400.0),
    // xTool (nur die GRBL-Modelle; die übrigen siehe UNSUPPORTED)
    grbl("xTool", "D1", 432.0, 406.0),
    grbl("xTool", "D1 Pro", 430.0, 390.0),
    // Creality
    grbl("Creality", "Falcon (CR-Laser Falcon)", 400.0, 415.0),
    grbl("Creality", "Falcon2 / Falcon2 Pro", 400.0, 415.0),
    // Two Trees
    grbl("Two Trees", "TTS-55 / TTS-10 Pro", 300.0, 300.0),
    grbl("Two Trees", "TS2", 450.0, 450.0),
    grbl("Two Trees", "Totem S", 300.0, 300.0),
    // Longer
    grbl("Longer", "Ray5", 400.0, 400.0),
    grbl("Longer", "B1", 400.0, 400.0),
    // NEJE
    grbl("NEJE", "3 Max", 460.0, 810.0),
    // Comgrow
    grbl("Comgrow", "Z1", 400.0, 410.0),
    // K40 (CO₂) mit getauschter Steuerung
    with_hint(
        Model { origin: OriginCorner::RearLeft, ..grbl("K40", "mit GRBL (z. B. grbl-LPC)", 300.0, 200.0) },
        K40_HINT,
    ),
    with_hint(
        Model {
            firmware: Firmware::Smoothieware,
            origin: OriginCorner::RearLeft,
            s_max: 1.0,
            dynamic_power: false,
            ..grbl("K40", "mit Smoothieware (z. B. Cohesion3D Mini)", 300.0, 200.0)
        },
        K40_HINT,
    ),
];

/// Laser, die VectorLume nicht ansteuern kann (geschlossene Protokolle), mit Begründung.
pub const UNSUPPORTED: &[(&str, &str)] = &[
    ("Ruida (die meisten CO₂-Laser)", "eigenes Binärprotokoll"),
    ("Trocen, TopWisdom, Leetro (CO₂-Steuerungen)", "eigenes Binärprotokoll"),
    ("Galvo- und Faserlaser mit EZCAD (BJJCZ)", "Galvo-Steuerung, kein G-Code"),
    ("xTool S1, F1, P2, M1 und neuere xTool-Geräte", "eigenes Protokoll"),
    ("Glowforge", "nur über den Cloud-Dienst des Herstellers"),
    ("K40 mit Originalplatine (M2 Nano)", "eigenes Protokoll"),
];

impl Model {
    /// Anzeigename, z. B. „Sculpfun S30 Ultra“ bzw. „GRBL-Laser“ für allgemeine Einträge.
    pub fn title(&self) -> String {
        if self.maker == GENERIC { self.name.to_string() } else { format!("{} {}", self.maker, self.name) }
    }

    /// Fertiges Geräteprofil mit den übrigen Werten aus der allgemeinen Vorlage.
    pub fn profile(&self) -> MachineProfile {
        MachineProfile {
            name: self.title(),
            firmware: self.firmware,
            bed_width: self.width,
            bed_height: self.height,
            origin: self.origin,
            s_max: self.s_max,
            dynamic_power: self.dynamic_power,
            // M8/M9 schaltet bei GRBL das Kühlmittel-Relais, an dem meist die Luftpumpe hängt; ohne Pumpe wirkungslos
            air_assist_supported: self.firmware == Firmware::Grbl,
            ..MachineProfile::default()
        }
    }
}

/// Hersteller in Listenreihenfolge (ohne Doppelte).
pub fn makers() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for m in MODELS {
        if !out.contains(&m.maker) {
            out.push(m.maker);
        }
    }
    out
}

/// Einträge, die zu allen Suchwörtern passen (Hersteller, Modell, Steuerung; ohne Groß-/Kleinschreibung).
pub fn search(query: &str) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    MODELS
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            let hay = format!("{} {} {}", m.maker, m.name, m.firmware.label()).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_consistent() {
        // Hersteller stehen zusammen, „Allgemein“ zuerst
        let makers = makers();
        assert_eq!(makers[0], GENERIC);
        let mut last = "";
        let mut seen = Vec::new();
        for m in MODELS {
            if m.maker != last {
                assert!(!seen.contains(&m.maker), "{} ist nicht zusammenhängend", m.maker);
                seen.push(m.maker);
                last = m.maker;
            }
            assert!(m.width >= 100.0 && m.height >= 100.0, "{}", m.title());
            assert_eq!(m.s_max, m.firmware.default_s_max(), "{}", m.title());
            let p = m.profile();
            assert_eq!(p.name, m.title());
            assert_eq!(p.bed_rect().width(), m.width);
        }
        let titles: Vec<String> = MODELS.iter().map(Model::title).collect();
        for (i, t) in titles.iter().enumerate() {
            assert!(!titles[..i].contains(t), "doppelt: {t}");
        }
    }

    #[test]
    fn search_matches_all_words() {
        let hits = search("sculpfun s30");
        assert!(hits.len() >= 3);
        assert!(hits.iter().all(|&i| MODELS[i].maker == "Sculpfun" && MODELS[i].name.contains("S30")));
        assert_eq!(search("").len(), MODELS.len());
        assert!(search("marlin").iter().any(|&i| MODELS[i].firmware == Firmware::Marlin));
        assert!(search("ruida").is_empty());
        let sf = &MODELS[search("SF-A9")[0]];
        assert_eq!(sf.profile().bed_width, 400.0);
    }
}
