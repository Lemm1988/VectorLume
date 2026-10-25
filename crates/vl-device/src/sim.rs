//! Eingebauter Simulator: verhält sich wie eine GRBL-1.1-Steuerung im Laser-Modus, inklusive Empfangs- und
//! Planerpuffer, damit Streaming und Oberfläche ohne Laser getestet werden können. Für Marlin und
//! Smoothieware bildet er deren Abweichungen nach (keine Echtzeitbefehle bzw. nur `?`, `M114`, `M400`,
//! `M410`, `M112`/`M999`).
//!
//! Störfälle für Tests (nur GRBL, als Zeile gesendet, auch mitten in einem Job): `$SIM=REBOOT` startet die
//! Steuerung neu wie nach einem Stromausfall, `$SIM=LIMIT` löst einen Endschalter-Alarm aus (danach wird bis zum
//! Zurücksetzen nichts mehr ausgeführt), `$SIM=HANG=<ms>` lässt die Steuerung so lange verstummen.

use crate::protocol::MachineState;
use crate::transport::Transport;
use std::collections::VecDeque;
use std::io;
use std::time::Instant;
use vl_core::Firmware;

const PLANNER_SIZE: usize = 15;
const RX_BUFFER: usize = 128;
const RAPID_RATE: f64 = 6000.0;

struct SimMove {
    from: [f64; 2],
    to: [f64; 2],
    duration: f64,
}

pub struct SimTransport {
    fw: Firmware,
    out: VecDeque<u8>,
    line: Vec<u8>,
    rx_lines: VecDeque<String>,
    rx_bytes: usize,
    planner: VecDeque<SimMove>,
    elapsed_in_move: f64,
    pos: [f64; 2],
    state: MachineState,
    hold: bool,
    absolute: bool,
    rapid: bool,
    feed: f64,
    spindle: f64,
    laser_on: bool,
    jogging: bool,
    settings: Vec<(u32, String)>,
    last_tick: Instant,
    speedup: f64,
    /// Nach einem kritischen Alarm (Endschalter): Zeilen werden erst nach dem Zurücksetzen wieder ausgeführt.
    critical: bool,
    /// Bis dahin keine Antworten und keine Verarbeitung (eingefrorene Steuerung).
    silent_until: Option<Instant>,
}

impl SimTransport {
    /// `speedup` = Faktor gegenüber Echtzeit (1.0 = realistisch, Tests nutzen große Werte).
    pub fn new(speedup: f64) -> Self {
        Self::with_firmware(speedup, Firmware::Grbl)
    }

    pub fn with_firmware(speedup: f64, fw: Firmware) -> Self {
        let mut s = Self {
            fw,
            out: VecDeque::new(),
            line: Vec::new(),
            rx_lines: VecDeque::new(),
            rx_bytes: 0,
            planner: VecDeque::new(),
            elapsed_in_move: 0.0,
            pos: [0.0, 0.0],
            state: MachineState::Idle,
            hold: false,
            absolute: true,
            rapid: true,
            feed: 0.0,
            spindle: 0.0,
            laser_on: false,
            jogging: false,
            settings: [
                (0, "10"),
                (1, "25"),
                (10, "1"),
                (22, "1"),
                (30, "1000"),
                (31, "0"),
                (32, "1"),
                (100, "80.000"),
                (101, "80.000"),
                (110, "6000.000"),
                (111, "6000.000"),
                (120, "1000.000"),
                (121, "1000.000"),
                (130, "400.000"),
                (131, "400.000"),
            ]
            .into_iter()
            .map(|(k, v)| (k, v.to_string()))
            .collect(),
            last_tick: Instant::now(),
            speedup: speedup.max(0.01),
            critical: false,
            silent_until: None,
        };
        s.reset();
        s
    }

    pub fn position(&self) -> [f64; 2] {
        self.pos
    }

    fn send(&mut self, text: &str) {
        self.out.extend(text.as_bytes());
        self.out.extend(b"\r\n");
    }

    fn silent(&mut self) -> bool {
        match self.silent_until {
            Some(t) if Instant::now() < t => true,
            Some(_) => {
                self.silent_until = None;
                false
            }
            None => false,
        }
    }

    fn homing_enabled(&self) -> bool {
        self.settings.iter().any(|(k, v)| *k == 22 && v.trim() == "1")
    }

    /// Neustart wie nach einem Stromausfall: alles Empfangene ist verloren, die Position zählt ab null.
    fn reboot(&mut self) {
        self.planner.clear();
        self.rx_lines.clear();
        self.rx_bytes = 0;
        self.line.clear();
        self.elapsed_in_move = 0.0;
        self.pos = [0.0, 0.0];
        self.critical = false;
        self.state = if self.homing_enabled() { MachineState::Alarm } else { MachineState::Idle };
        self.reset();
    }

    /// Endschalter ausgelöst: sofort anhalten, Laser aus, Alarm bis zum Zurücksetzen.
    fn hard_limit(&mut self) {
        self.planner.clear();
        self.rx_lines.clear();
        self.rx_bytes = 0;
        self.elapsed_in_move = 0.0;
        self.laser_on = false;
        self.state = MachineState::Alarm;
        self.critical = true;
        self.send("ALARM:1");
        self.send("[MSG:Reset to continue]");
    }

    fn reset(&mut self) {
        let was_moving = !self.planner.is_empty();
        self.planner.clear();
        self.rx_lines.clear();
        self.rx_bytes = 0;
        self.line.clear();
        self.elapsed_in_move = 0.0;
        self.hold = false;
        self.absolute = true;
        self.rapid = true;
        self.laser_on = false;
        self.jogging = false;
        self.spindle = 0.0;
        self.critical = false;
        match self.fw {
            Firmware::Grbl => {
                self.send("");
                self.send("Grbl 1.1h ['$' for help]");
            }
            Firmware::Marlin => {
                self.send("start");
                self.send("echo:Marlin 2.1.2 (VectorLume Simulator)");
                return;
            }
            Firmware::Smoothieware => {
                self.send("Smoothie");
                return;
            }
        }
        if was_moving {
            self.state = MachineState::Alarm;
            self.send("ALARM:3");
            self.send("[MSG:Reset to continue]");
        } else if self.state == MachineState::Alarm {
            self.send("[MSG:'$H'|'$X' to unlock]");
        } else {
            self.state = MachineState::Idle;
        }
    }

    fn status(&mut self) {
        let state = if self.hold {
            MachineState::Hold
        } else if self.state == MachineState::Alarm {
            MachineState::Alarm
        } else if self.planner.is_empty() {
            MachineState::Idle
        } else if self.jogging {
            MachineState::Jog
        } else {
            MachineState::Run
        };
        let s = if self.laser_on && !self.planner.is_empty() { self.spindle } else { 0.0 };
        let line = format!(
            "<{}|MPos:{:.3},{:.3},0.000|Bf:{},{}|FS:{:.0},{:.0}|WCO:0.000,0.000,0.000>",
            state.as_grbl(),
            self.pos[0],
            self.pos[1],
            PLANNER_SIZE - self.planner.len(),
            RX_BUFFER - self.rx_bytes,
            if self.planner.is_empty() { 0.0 } else { self.feed },
            s
        );
        self.send(&line);
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let mut dt = now.duration_since(self.last_tick).as_secs_f64() * self.speedup;
        self.last_tick = now;
        if !self.hold {
            while dt > 0.0 {
                let Some(m) = self.planner.front() else { break };
                let remaining = m.duration - self.elapsed_in_move;
                if dt >= remaining {
                    dt -= remaining;
                    self.pos = m.to;
                    self.planner.pop_front();
                    self.elapsed_in_move = 0.0;
                } else {
                    self.elapsed_in_move += dt;
                    let t = (self.elapsed_in_move / m.duration).clamp(0.0, 1.0);
                    self.pos = [m.from[0] + (m.to[0] - m.from[0]) * t, m.from[1] + (m.to[1] - m.from[1]) * t];
                    dt = 0.0;
                }
            }
            if self.planner.is_empty() {
                self.jogging = false;
            }
        }
        self.process_lines();
    }

    fn process_lines(&mut self) {
        while self.planner.len() < PLANNER_SIZE && !self.critical {
            let Some(line) = self.rx_lines.front() else { break };
            // M400 wartet, bis alle Bewegungen ausgeführt sind.
            if !self.fw.is_grbl() && line.trim().eq_ignore_ascii_case("M400") && !self.planner.is_empty() {
                break;
            }
            let line = self.rx_lines.pop_front().unwrap();
            self.rx_bytes -= line.len() + 1;
            let reply = match self.fw {
                Firmware::Grbl => self.execute(&line),
                _ => self.execute_other(&line),
            };
            // Störfälle antworten nicht selbst
            if !reply.is_empty() {
                self.send(&reply);
            }
        }
    }

    /// Marlin/Smoothieware: eigene Befehle, unbekannte Befehle werden mit `ok` quittiert.
    fn execute_other(&mut self, line: &str) -> String {
        let l = line.trim().to_ascii_uppercase();
        let halted = self.state == MachineState::Alarm;
        if self.fw == Firmware::Smoothieware && halted && l != "M999" && l != "$X" {
            return "!!".into();
        }
        match l.as_str() {
            "M114" => {
                let [x, y] = self.pos;
                self.send(&format!("X:{x:.2} Y:{y:.2} Z:0.00 E:0.00 Count X:0 Y:0 Z:0"));
                return "ok".into();
            }
            "M115" => {
                let name = match self.fw {
                    Firmware::Marlin => "FIRMWARE_NAME:Marlin 2.1.2 (VectorLume Simulator) MACHINE_TYPE:Laser",
                    _ => "FIRMWARE_NAME:Smoothieware, FIRMWARE_VERSION:edge (VectorLume Simulator)",
                };
                self.send(name);
                return "ok".into();
            }
            "M400" => return "ok".into(),
            "M410" => {
                self.planner.clear();
                self.laser_on = false;
                return "ok".into();
            }
            "M112" => {
                self.planner.clear();
                self.laser_on = false;
                self.state = MachineState::Alarm;
                return "ok".into();
            }
            "M999" | "$X" => {
                self.state = MachineState::Idle;
                return "ok".into();
            }
            "$H" | "G28" | "G28 X Y" | "G28XY" => {
                self.planner.clear();
                self.pos = [0.0, 0.0];
                return "ok".into();
            }
            _ => {}
        }
        // Marlins Inline-Schalter `I` hat keinen Wert; der Rest ist G-Code wie bei GRBL.
        let cleaned: String = l.replace(" I ", " ").replace(" I", " ");
        let reply = self.execute(&cleaned);
        if reply.starts_with("error") && self.fw == Firmware::Marlin {
            self.send(&format!("echo:Unknown command: \"{}\"", line.trim()));
        }
        "ok".into()
    }

    fn planned_end(&self) -> [f64; 2] {
        self.planner.back().map(|m| m.to).unwrap_or(self.pos)
    }

    fn queue_move(&mut self, target: [f64; 2], rate: f64) {
        let from = self.planned_end();
        let dist = ((target[0] - from[0]).powi(2) + (target[1] - from[1]).powi(2)).sqrt();
        if dist > 1e-9 {
            self.planner.push_back(SimMove { from, to: target, duration: dist / (rate.max(1.0) / 60.0) });
        }
    }

    fn execute(&mut self, line: &str) -> String {
        let l = line.trim().to_ascii_uppercase();
        if l.is_empty() {
            return "ok".into();
        }
        if let Some(cmd) = l.strip_prefix('$') {
            return self.system_command(cmd);
        }
        if self.state == MachineState::Alarm {
            return "error:9".into();
        }
        let words = match parse_words(&l) {
            Some(w) => w,
            None => return "error:1".into(),
        };
        let (mut x, mut y, mut motion) = (None, None, None);
        for (letter, value) in words {
            match letter {
                'G' => match value as i32 {
                    0 => motion = Some(true),
                    1 => motion = Some(false),
                    90 => self.absolute = true,
                    91 => self.absolute = false,
                    21 | 17 | 94 | 54 => {}
                    _ => return "error:20".into(),
                },
                'M' => match value as i32 {
                    3 | 4 => self.laser_on = true,
                    5 => self.laser_on = false,
                    8 | 9 => {}
                    _ => return "error:20".into(),
                },
                'X' => x = Some(value),
                'Y' => y = Some(value),
                'F' => self.feed = value,
                'S' => self.spindle = value,
                // Z und eine vierte Achse (Drehachse) werden angenommen, aber nicht nachgebildet.
                'Z' | 'A' => {}
                _ => return "error:20".into(),
            }
        }
        if let Some(r) = motion {
            self.rapid = r;
        }
        if x.is_some() || y.is_some() {
            if !self.rapid && self.feed <= 0.0 {
                return "error:22".into();
            }
            let end = self.planned_end();
            let target = if self.absolute {
                [x.unwrap_or(end[0]), y.unwrap_or(end[1])]
            } else {
                [end[0] + x.unwrap_or(0.0), end[1] + y.unwrap_or(0.0)]
            };
            let rate = if self.rapid { RAPID_RATE } else { self.feed };
            self.queue_move(target, rate);
        }
        "ok".into()
    }

    fn system_command(&mut self, cmd: &str) -> String {
        match cmd {
            "$" => {
                let lines: Vec<String> = self.settings.iter().map(|(k, v)| format!("${k}={v}")).collect();
                for l in lines {
                    self.send(&l);
                }
                "ok".into()
            }
            "I" => {
                self.send("[VER:1.1h.20190825:VectorLume Simulator]");
                self.send("[OPT:V,15,128]");
                "ok".into()
            }
            "G" => {
                self.send(&format!(
                    "[GC:G{} G54 G17 G21 G{} G94 M{} M9 T0 F{:.0} S{:.0}]",
                    if self.rapid { 0 } else { 1 },
                    if self.absolute { 90 } else { 91 },
                    if self.laser_on { 4 } else { 5 },
                    self.feed,
                    self.spindle
                ));
                "ok".into()
            }
            "H" => {
                self.planner.clear();
                self.pos = [0.0, 0.0];
                self.state = MachineState::Idle;
                "ok".into()
            }
            "SIM=REBOOT" => {
                self.reboot();
                String::new()
            }
            "SIM=LIMIT" => {
                self.hard_limit();
                String::new()
            }
            "X" => {
                self.state = MachineState::Idle;
                self.send("[MSG:Caution: Unlocked]");
                "ok".into()
            }
            _ => {
                if let Some(ms) = cmd.strip_prefix("SIM=HANG=").and_then(|v| v.parse::<u64>().ok()) {
                    self.silent_until = Some(Instant::now() + std::time::Duration::from_millis(ms));
                    return String::new();
                }
                if let Some(jog) = cmd.strip_prefix("J=") {
                    if self.state == MachineState::Alarm {
                        return "error:9".into();
                    }
                    return self.jog(jog);
                }
                if let Some((k, v)) = cmd.split_once('=')
                    && let Ok(n) = k.parse::<u32>()
                {
                    match self.settings.iter_mut().find(|(key, _)| *key == n) {
                        Some(entry) => entry.1 = v.to_string(),
                        None => self.settings.push((n, v.to_string())),
                    }
                    return "ok".into();
                }
                "error:3".into()
            }
        }
    }

    fn jog(&mut self, jog: &str) -> String {
        let Some(words) = parse_words(jog) else { return "error:16".into() };
        let (mut rel, mut x, mut y, mut f) = (!self.absolute, None, None, None);
        for (letter, v) in words {
            match letter {
                'G' if v as i32 == 91 => rel = true,
                'G' if v as i32 == 90 => rel = false,
                'G' if v as i32 == 21 => {}
                'X' => x = Some(v),
                'Y' => y = Some(v),
                'F' => f = Some(v),
                'Z' | 'A' => {}
                _ => return "error:16".into(),
            }
        }
        let Some(f) = f else { return "error:16".into() };
        let end = self.planned_end();
        let target = if rel {
            [end[0] + x.unwrap_or(0.0), end[1] + y.unwrap_or(0.0)]
        } else {
            [x.unwrap_or(end[0]), y.unwrap_or(end[1])]
        };
        self.jogging = true;
        self.queue_move(target, f);
        "ok".into()
    }

    fn realtime(&mut self, b: u8) {
        if self.fw == Firmware::Smoothieware && b == 0x18 {
            // Smoothieware: Strg-X hält an (Halt-Zustand bis M999/$X).
            self.planner.clear();
            self.elapsed_in_move = 0.0;
            self.laser_on = false;
            self.state = MachineState::Alarm;
            self.send("ALARM: Abort during cycle");
            return;
        }
        match b {
            b'?' => self.status(),
            b'!' => self.hold = !self.planner.is_empty() || self.hold,
            b'~' => self.hold = false,
            0x18 => self.reset(),
            0x85 if self.jogging => {
                self.planner.clear();
                self.jogging = false;
            }
            _ => {}
        }
    }
}

fn parse_words(l: &str) -> Option<Vec<(char, f64)>> {
    let mut words = Vec::new();
    let chars: Vec<char> = l.chars().filter(|c| !c.is_whitespace()).collect();
    let mut i = 0;
    while i < chars.len() {
        let letter = chars[i];
        if !letter.is_ascii_alphabetic() {
            return None;
        }
        i += 1;
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == '-' || chars[i] == '+') {
            i += 1;
        }
        let num: String = chars[start..i].iter().collect();
        words.push((letter, num.parse().ok()?));
    }
    Some(words)
}

impl SimTransport {
    /// Bytes, die die Firmware sofort statt als Teil einer Zeile auswertet.
    fn is_realtime(&self, b: u8) -> bool {
        match self.fw {
            Firmware::Grbl => matches!(b, b'?' | b'!' | b'~' | 0x18 | 0x80..=0xFF),
            Firmware::Smoothieware => matches!(b, b'?' | 0x18),
            Firmware::Marlin => false,
        }
    }
}

impl Transport for SimTransport {
    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        if self.silent() {
            // eingefroren: alles geht verloren
            return Ok(());
        }
        self.tick();
        for &b in data {
            match b {
                _ if self.is_realtime(b) => self.realtime(b),
                b'\n' => {
                    let text = String::from_utf8_lossy(&self.line).trim_end_matches('\r').to_string();
                    self.line.clear();
                    if self.rx_bytes + text.len() + 1 > RX_BUFFER {
                        // Echte GRBL würde Daten verlieren – das deutet auf einen Fehler im Streaming hin.
                        return Err(io::Error::other("Simulator: Empfangspuffer übergelaufen"));
                    }
                    self.rx_bytes += text.len() + 1;
                    self.rx_lines.push_back(text);
                }
                b'\r' => {}
                _ => self.line.push(b),
            }
        }
        self.process_lines();
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.silent() {
            return Ok(0);
        }
        self.tick();
        let n = buf.len().min(self.out.len());
        for (slot, b) in buf.iter_mut().zip(self.out.drain(..n)) {
            *slot = b;
        }
        Ok(n)
    }
}
