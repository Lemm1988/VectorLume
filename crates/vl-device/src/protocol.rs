//! Auswertung der Antworten einer GRBL-1.1-Steuerung.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MachineState {
    #[default]
    Unknown,
    Idle,
    Run,
    Hold,
    Jog,
    Alarm,
    Door,
    Check,
    Home,
    Sleep,
}

impl MachineState {
    pub fn parse(s: &str) -> Self {
        match s.split(':').next().unwrap_or("") {
            "Idle" => Self::Idle,
            "Run" => Self::Run,
            "Hold" => Self::Hold,
            "Jog" => Self::Jog,
            "Alarm" => Self::Alarm,
            "Door" => Self::Door,
            "Check" => Self::Check,
            "Home" => Self::Home,
            "Sleep" => Self::Sleep,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Unbekannt",
            Self::Idle => "Bereit",
            Self::Run => "Läuft",
            Self::Hold => "Pause",
            Self::Jog => "Verfahren",
            Self::Alarm => "Alarm",
            Self::Door => "Tür offen",
            Self::Check => "Prüfmodus",
            Self::Home => "Referenzfahrt",
            Self::Sleep => "Ruhezustand",
        }
    }

    pub fn as_grbl(self) -> &'static str {
        match self {
            Self::Unknown | Self::Idle => "Idle",
            Self::Run => "Run",
            Self::Hold => "Hold:0",
            Self::Jog => "Jog",
            Self::Alarm => "Alarm",
            Self::Door => "Door:0",
            Self::Check => "Check",
            Self::Home => "Home",
            Self::Sleep => "Sleep",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub state: MachineState,
    pub mpos: Option<[f64; 3]>,
    pub wpos: Option<[f64; 3]>,
    pub wco: Option<[f64; 3]>,
    pub feed: Option<f64>,
    pub spindle: Option<f64>,
    /// Freie Plätze im Planer und im Empfangspuffer (`Bf:`).
    pub buffer: Option<(u32, u32)>,
}

impl Status {
    /// Arbeitskoordinaten; fehlt WPos, wird sie aus MPos und dem zuletzt bekannten WCO berechnet.
    pub fn work_position(&self, last_wco: Option<[f64; 3]>) -> Option<[f64; 3]> {
        if let Some(w) = self.wpos {
            return Some(w);
        }
        let m = self.mpos?;
        let wco = self.wco.or(last_wco).unwrap_or([0.0; 3]);
        Some([m[0] - wco[0], m[1] - wco[1], m[2] - wco[2]])
    }
}

fn parse_triplet(s: &str) -> Option<[f64; 3]> {
    let mut it = s.split(',').map(|v| v.trim().parse::<f64>());
    let x = it.next()?.ok()?;
    let y = it.next()?.ok()?;
    let z = it.next().and_then(|r| r.ok()).unwrap_or(0.0);
    Some([x, y, z])
}

pub fn parse_status(line: &str) -> Option<Status> {
    let inner = line.trim().strip_prefix('<')?.strip_suffix('>')?;
    let mut parts = inner.split('|');
    let mut st = Status { state: MachineState::parse(parts.next()?), ..Default::default() };
    for part in parts {
        let Some((key, val)) = part.split_once(':') else { continue };
        match key {
            "MPos" => st.mpos = parse_triplet(val),
            "WPos" => st.wpos = parse_triplet(val),
            "WCO" => st.wco = parse_triplet(val),
            "FS" => {
                let mut it = val.split(',');
                st.feed = it.next().and_then(|v| v.parse().ok());
                st.spindle = it.next().and_then(|v| v.parse().ok());
            }
            "F" => st.feed = val.parse().ok(),
            "Bf" => {
                let mut it = val.split(',').filter_map(|v| v.parse().ok());
                if let (Some(a), Some(b)) = (it.next(), it.next()) {
                    st.buffer = Some((a, b));
                }
            }
            _ => {}
        }
    }
    Some(st)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Response {
    Ok,
    Error(u32),
    Alarm(u32),
    Status(Status),
    /// Begrüßung nach einem Reset, z. B. `Grbl 1.1h ['$' for help]`.
    Welcome(String),
    /// Einstellung aus `$$`, z. B. `$30=1000`.
    Setting(u32, String),
    /// Meldung in eckigen Klammern, z. B. `[MSG:...]`, `[VER:...]`.
    Feedback(String),
    Other(String),
}

pub fn parse_response(line: &str) -> Response {
    let l = line.trim();
    if l == "ok" {
        return Response::Ok;
    }
    if let Some(code) = l.strip_prefix("error:") {
        return Response::Error(code.trim().parse().unwrap_or(0));
    }
    if let Some(code) = l.strip_prefix("ALARM:") {
        return Response::Alarm(code.trim().parse().unwrap_or(0));
    }
    if l.starts_with('<')
        && let Some(st) = parse_status(l)
    {
        return Response::Status(st);
    }
    if l.starts_with("Grbl ") || l.starts_with("GrblHAL ") {
        return Response::Welcome(l.to_string());
    }
    if let Some(rest) = l.strip_prefix('$')
        && let Some((k, v)) = rest.split_once('=')
        && let Ok(n) = k.parse()
    {
        return Response::Setting(n, v.to_string());
    }
    if l.starts_with('[') && l.ends_with(']') {
        return Response::Feedback(l[1..l.len() - 1].to_string());
    }
    Response::Other(l.to_string())
}

pub fn error_text(code: u32) -> &'static str {
    match code {
        1 => "Befehlsbuchstabe erwartet",
        2 => "Ungültiges Zahlenformat",
        3 => "$-Befehl unbekannt",
        4 => "Negativer Wert nicht erlaubt",
        5 => "Referenzfahrt ist nicht aktiviert ($22)",
        6 => "Schrittimpuls zu kurz eingestellt",
        7 => "EEPROM-Lesefehler, Standardwerte geladen",
        8 => "Befehl nur im Ruhezustand möglich",
        9 => "G-Code im Alarm- oder Jog-Zustand gesperrt",
        10 => "Softlimits benötigen aktivierte Referenzfahrt",
        11 => "Zeile zu lang",
        12 => "Einstellung ergibt zu hohe Schrittrate",
        13 => "Sicherheitstür offen",
        14 => "Text zu lang für EEPROM",
        15 => "Jog-Ziel außerhalb des Verfahrwegs",
        16 => "Ungültiger Jog-Befehl",
        17 => "Laser-Modus benötigt PWM-Ausgang",
        20 => "Nicht unterstützter G-Code-Befehl",
        21 => "Mehrere Befehle derselben Modalgruppe",
        22 => "Vorschub (F) nicht gesetzt",
        23 => "Ganzzahl erwartet",
        24 => "Mehrere Befehle mit Achsangaben in einer Zeile",
        25 => "Wort in der Zeile wiederholt",
        26 => "Achsangaben fehlen",
        27 => "Ungültige Zeilennummer",
        28 => "Pflichtwert fehlt",
        29 => "Koordinatensystem G59.x nicht unterstützt",
        30 => "G53 nur zusammen mit G0/G1 erlaubt",
        31 => "Achsangaben ohne passenden Befehl",
        32 => "Bogen ohne Achsangabe in der Ebene",
        33 => "Ungültiges Bewegungsziel",
        34 => "Ungültiger Bogenradius",
        35 => "Bogen ohne Mittelpunktsangabe",
        36 => "Ungenutzte Werte in der Zeile",
        37 => "Werkzeuglängenkorrektur ungültig",
        38 => "Werkzeugnummer zu groß",
        _ => "Unbekannter Fehler",
    }
}

pub fn alarm_text(code: u32) -> &'static str {
    match code {
        1 => "Endschalter ausgelöst – Position verloren, Referenzfahrt nötig",
        2 => "Ziel außerhalb des Verfahrwegs (Softlimit)",
        3 => "Reset während einer Bewegung – Position evtl. verloren",
        4 => "Taster nicht im erwarteten Zustand",
        5 => "Taster hat keinen Kontakt gefunden",
        6 => "Referenzfahrt durch Reset abgebrochen",
        7 => "Referenzfahrt: Sicherheitstür geöffnet",
        8 => "Referenzfahrt: Endschalter nicht freigefahren",
        9 => "Referenzfahrt: Endschalter nicht gefunden",
        _ => "Unbekannter Alarm",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_with_mpos_and_wco() {
        let st = parse_status("<Run|MPos:10.000,20.500,0.000|FS:1500,300|WCO:1.000,2.000,0.000>").unwrap();
        assert_eq!(st.state, MachineState::Run);
        assert_eq!(st.feed, Some(1500.0));
        assert_eq!(st.spindle, Some(300.0));
        assert_eq!(st.work_position(None), Some([9.0, 18.5, 0.0]));
    }

    #[test]
    fn status_variants() {
        let st = parse_status("<Hold:0|WPos:1.0,2.0,3.0|Bf:15,128|F:500>").unwrap();
        assert_eq!(st.state, MachineState::Hold);
        assert_eq!(st.wpos, Some([1.0, 2.0, 3.0]));
        assert_eq!(st.buffer, Some((15, 128)));
        assert_eq!(st.feed, Some(500.0));
        // Zweiachsige Steuerung ohne Z
        assert_eq!(parse_status("<Idle|MPos:1,2>").unwrap().mpos, Some([1.0, 2.0, 0.0]));
    }

    #[test]
    fn responses() {
        assert_eq!(parse_response("ok\r"), Response::Ok);
        assert_eq!(parse_response("error:20"), Response::Error(20));
        assert_eq!(parse_response("ALARM:1"), Response::Alarm(1));
        assert_eq!(parse_response("$30=1000"), Response::Setting(30, "1000".into()));
        assert!(matches!(parse_response("Grbl 1.1h ['$' for help]"), Response::Welcome(_)));
        assert_eq!(parse_response("[MSG:Caution: Unlocked]"), Response::Feedback("MSG:Caution: Unlocked".into()));
    }
}
