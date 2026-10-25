//! Gerätetreiber: eigener Thread, der die Verbindung hält, Zeilen streamt (GRBL: Zeichenzähl-Verfahren,
//! Marlin/Smoothieware: Senden und auf `ok` warten), den Status abfragt und Ereignisse an die Oberfläche meldet.

use crate::protocol::{MachineState, Response, Status, alarm_text, error_text, parse_response};
use crate::sim::SimTransport;
use crate::transport::{SIMULATOR_PORT, SerialTransport, TCP_PREFIX, TcpTransport, Transport};
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use vl_core::{Firmware, LineBuf};

const STATUS_INTERVAL: Duration = Duration::from_millis(200);
/// Marlin kennt keine Echtzeit-Statusabfrage: im Ruhezustand wird die Position mit `M114` abgefragt.
const POSITION_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// Wartezeit nach dem Öffnen, falls sich die Steuerung nicht meldet (Arduino-Boards starten beim Öffnen neu).
const STARTUP_TIMEOUT: Duration = Duration::from_millis(2500);
/// Netzwerk-Steuerungen starten beim Verbinden nicht neu.
const STARTUP_TIMEOUT_NETWORK: Duration = Duration::from_millis(300);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// Senden und Warten: bleibt außerhalb eines Jobs so lange jede Antwort aus, gilt die Zeile als beantwortet
/// (z. B. nach einem Stopp verschluckte `ok`), damit die Verbindung nicht hängen bleibt.
const STALE_REPLY_TIMEOUT: Duration = Duration::from_secs(3);
/// So lange ohne jede Antwort auf die Statusabfrage, dann wird gewarnt (GRBL antwortet sonst sofort).
const SILENCE_WARNING: Duration = Duration::from_secs(4);
/// Mindestabstand automatischer Resets nach „Reset to continue“ (falls der Endschalter noch ausgelöst ist).
const AUTO_RESET_INTERVAL: Duration = Duration::from_secs(3);
/// Längere Zeilen gibt es bei keiner Steuerung; ohne Zeilenende (falsche Baudrate, fremdes Gerät) wird verworfen.
const MAX_LINE: usize = 16 * 1024;

pub const RT_STATUS: u8 = b'?';
pub const RT_HOLD: u8 = b'!';
pub const RT_RESUME: u8 = b'~';
pub const RT_RESET: u8 = 0x18;
pub const RT_JOG_CANCEL: u8 = 0x85;

#[derive(Clone, Debug)]
pub enum Command {
    Connect {
        port: String,
        baud: u32,
        rx_buffer: usize,
        firmware: Firmware,
    },
    Disconnect,
    /// Einzelne Zeile aus Konsole oder Bedienelementen (wird während eines Jobs abgelehnt).
    Line(String),
    Realtime(u8),
    StartJob {
        name: String,
        lines: LineBuf,
    },
    Pause,
    Resume,
    /// Sofortiger Stopp: Bewegung abbrechen und Laser aus.
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Sent,
    Received,
    Info,
}

#[derive(Clone, Debug)]
pub enum Event {
    Connected {
        port: String,
    },
    Disconnected {
        reason: Option<String>,
    },
    Console {
        dir: Direction,
        text: String,
    },
    Status(Status),
    Setting(u32, String),
    JobStarted {
        name: String,
        total: usize,
    },
    JobProgress {
        acked: usize,
        total: usize,
    },
    JobFinished {
        ok: bool,
        message: String,
        seconds: f64,
    },
    Alarm(u32, String),
    /// Wichtiger Hinweis, der auffallen soll (z. B. Steuerung antwortet nicht mehr).
    Warning(String),
}

/// Herkunft einer gesendeten Zeile – entscheidet, was mit ihrer Antwort geschieht.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Console,
    Job,
    /// Interne Abfrage (z. B. `M114`), Antwort erscheint nicht in der Konsole.
    Poll,
}

/// Warteschlange der zu sendenden Zeilen. Jobzeilen bleiben im kompakten Puffer und werden erst beim
/// Senden gelesen – eine Kopie von Millionen einzelner Strings wäre bei großen Rasterjobs teuer.
#[derive(Default)]
struct SendQueue {
    entries: VecDeque<Entry>,
}

enum Entry {
    Line(String, Kind),
    /// Noch nicht gesendete Zeilen ab `next` (nie leer).
    Job {
        lines: LineBuf,
        next: usize,
    },
}

impl SendQueue {
    fn push(&mut self, line: String, kind: Kind) {
        self.entries.push_back(Entry::Line(line, kind));
    }

    fn push_job(&mut self, lines: LineBuf) {
        if !lines.is_empty() {
            self.entries.push_back(Entry::Job { lines, next: 0 });
        }
    }

    fn front(&self) -> Option<(&str, Kind)> {
        Some(match self.entries.front()? {
            Entry::Line(line, kind) => (line.as_str(), *kind),
            Entry::Job { lines, next } => (&lines[*next], Kind::Job),
        })
    }

    /// Entfernt die vorderste Zeile.
    fn pop(&mut self) {
        if let Some(Entry::Job { lines, next }) = self.entries.front_mut() {
            *next += 1;
            if *next < lines.len() {
                return;
            }
        }
        self.entries.pop_front();
    }

    /// Verwirft alle noch nicht gesendeten Jobzeilen.
    fn drop_job(&mut self) {
        self.entries.retain(|e| matches!(e, Entry::Line(_, kind) if *kind != Kind::Job));
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn clear(&mut self) {
        self.entries.clear();
    }
}

struct JobState {
    name: String,
    total: usize,
    acked: usize,
    all_acked_at: Option<Instant>,
    started: Instant,
    last_progress: Instant,
}

struct Connection {
    transport: Box<dyn Transport>,
    firmware: Firmware,
    rx_capacity: usize,
    inflight: VecDeque<(usize, Kind)>,
    inflight_bytes: usize,
    queue: SendQueue,
    job: Option<JobState>,
    rx_buf: Vec<u8>,
    last_poll: Instant,
    ready: bool,
    opened: Instant,
    startup_wait: Duration,
    last_state: MachineState,
    /// Pause ohne Echtzeitbefehl (Marlin/Smoothieware): keine weiteren Job-Zeilen senden.
    paused: bool,
    /// Zuletzt gemeldete Position (Marlin `M114`).
    last_pos: Option<[f64; 3]>,
    last_reply: Instant,
    last_send: Instant,
    /// Zuletzt irgendetwas empfangen (Überwachung, ob die Steuerung noch antwortet).
    last_rx: Instant,
    silence_warned: bool,
    last_auto_reset: Option<Instant>,
    /// Zeilen aus Zeichensalat (falsche Baudrate) – gewarnt wird einmal je Verbindung.
    garbled: u32,
}

impl Connection {
    /// Zustand für Steuerungen ohne eigene Zustandsmeldung.
    fn derived_state(&self) -> MachineState {
        match (&self.job, self.paused) {
            (Some(_), true) => MachineState::Hold,
            (Some(_), false) => MachineState::Run,
            (None, _) if self.last_state == MachineState::Alarm => MachineState::Alarm,
            (None, _) => MachineState::Idle,
        }
    }
}

pub struct Device {
    tx: Sender<Command>,
    rx: Receiver<Event>,
    thread: Option<JoinHandle<()>>,
}

impl Device {
    /// Startet den Treiber-Thread. `wake` wird nach jedem Ereignis aufgerufen (z. B. Oberfläche neu zeichnen).
    pub fn spawn(wake: impl Fn() + Send + 'static) -> Self {
        let (tx, cmd_rx) = channel();
        let (ev_tx, rx) = channel();
        let thread = std::thread::Builder::new()
            .name("vectorlume-device".into())
            .spawn(move || Driver { events: ev_tx, wake: Box::new(wake), conn: None }.run(cmd_rx))
            .expect("Treiber-Thread konnte nicht gestartet werden");
        Self { tx, rx, thread: Some(thread) }
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }

    pub fn poll(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // Kanal schließen: der Thread stoppt einen laufenden Job und beendet sich.
        let (dead_tx, _) = channel();
        drop(std::mem::replace(&mut self.tx, dead_tx));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Driver {
    events: Sender<Event>,
    wake: Box<dyn Fn() + Send>,
    conn: Option<Connection>,
}

impl Driver {
    fn emit(&self, ev: Event) {
        let _ = self.events.send(ev);
        (self.wake)();
    }

    fn info(&self, text: impl Into<String>) {
        self.emit(Event::Console { dir: Direction::Info, text: text.into() });
    }

    fn firmware(&self) -> Firmware {
        self.conn.as_ref().map(|c| c.firmware).unwrap_or_default()
    }

    fn run(mut self, commands: Receiver<Command>) {
        loop {
            let timeout = if self.conn.is_some() { Duration::from_millis(2) } else { Duration::from_millis(100) };
            match commands.recv_timeout(timeout) {
                Ok(cmd) => {
                    self.handle(cmd);
                    while let Ok(cmd) = commands.try_recv() {
                        self.handle(cmd);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    if self.conn.as_ref().is_some_and(|c| c.job.is_some()) {
                        self.emergency_stop();
                    }
                    return;
                }
            }
            if let Err(e) = self.service() {
                self.drop_connection(Some(e));
            }
        }
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Connect { port, baud, rx_buffer, firmware } => {
                if self.conn.is_some() {
                    self.drop_connection(None);
                }
                let network = port.starts_with(TCP_PREFIX);
                let transport: Result<Box<dyn Transport>, String> = if port == SIMULATOR_PORT {
                    Ok(Box::new(SimTransport::with_firmware(1.0, firmware)))
                } else if let Some(addr) = port.strip_prefix(TCP_PREFIX) {
                    TcpTransport::connect(addr).map(|t| Box::new(t) as Box<dyn Transport>).map_err(|e| e.to_string())
                } else {
                    SerialTransport::open(&port, baud)
                        .map(|t| Box::new(t) as Box<dyn Transport>)
                        .map_err(|e| e.to_string())
                };
                match transport {
                    Ok(transport) => {
                        self.conn = Some(Connection {
                            transport,
                            firmware,
                            rx_capacity: rx_buffer.max(32),
                            inflight: VecDeque::new(),
                            inflight_bytes: 0,
                            queue: SendQueue::default(),
                            job: None,
                            rx_buf: Vec::new(),
                            last_poll: Instant::now(),
                            ready: false,
                            opened: Instant::now(),
                            startup_wait: if network { STARTUP_TIMEOUT_NETWORK } else { STARTUP_TIMEOUT },
                            last_state: MachineState::Unknown,
                            paused: false,
                            last_pos: None,
                            last_reply: Instant::now(),
                            last_send: Instant::now(),
                            last_rx: Instant::now(),
                            silence_warned: false,
                            last_auto_reset: None,
                            garbled: 0,
                        });
                        self.emit(Event::Connected { port });
                        if firmware.is_grbl() {
                            // Firmware-Info und Einstellungen abfragen (Laser-Modus, S-Max, Verfahrweg).
                            self.enqueue("$I".into(), Kind::Console);
                            self.enqueue("$$".into(), Kind::Console);
                        } else {
                            self.enqueue("M115".into(), Kind::Console);
                        }
                    }
                    Err(e) => {
                        self.emit(Event::Disconnected { reason: Some(format!("Verbindung fehlgeschlagen: {e}")) })
                    }
                }
            }
            Command::Disconnect => {
                if self.conn.as_ref().is_some_and(|c| c.job.is_some()) {
                    self.emergency_stop();
                }
                self.drop_connection(None);
            }
            Command::Line(line) => {
                let busy = self.conn.as_ref().map(|c| c.job.is_some());
                match busy {
                    None => self.info("Nicht verbunden."),
                    Some(true) => self.info("Während eines Jobs sind keine Befehle möglich."),
                    Some(false) => self.enqueue(line, Kind::Console),
                }
            }
            Command::Realtime(b) => {
                if self.firmware().is_grbl() {
                    self.write_realtime(b);
                }
            }
            Command::StartJob { name, lines } => self.start_job(name, lines),
            Command::Pause => self.set_paused(true),
            Command::Resume => self.set_paused(false),
            Command::Stop => {
                if self.conn.is_some() {
                    self.emergency_stop();
                }
            }
        }
    }

    fn enqueue(&mut self, line: String, kind: Kind) {
        if let Some(c) = &mut self.conn {
            c.queue.push(line, kind);
        }
    }

    fn start_job(&mut self, name: String, mut lines: LineBuf) {
        let Some(c) = &mut self.conn else {
            self.info("Nicht verbunden.");
            return;
        };
        if c.job.is_some() {
            self.info("Es läuft bereits ein Job.");
            return;
        }
        if matches!(c.last_state, MachineState::Alarm | MachineState::Door) {
            let msg = "Die Steuerung ist im Alarm-Zustand. Bitte zuerst Referenzfahrt oder Entsperren ausführen.";
            self.emit(Event::JobFinished { ok: false, message: msg.into(), seconds: 0.0 });
            return;
        }
        if !c.firmware.is_grbl() {
            // Das `ok` auf M400 kommt erst, wenn alle Bewegungen ausgeführt sind: Ende des Jobs.
            lines.push("M400");
        }
        let total = lines.len();
        c.queue.push_job(lines);
        c.paused = false;
        let now = Instant::now();
        c.job = Some(JobState {
            name: name.clone(),
            total,
            acked: 0,
            all_acked_at: None,
            started: now,
            last_progress: now,
        });
        self.emit(Event::JobStarted { name, total });
    }

    fn set_paused(&mut self, pause: bool) {
        let Some(c) = &mut self.conn else { return };
        if c.firmware.is_grbl() {
            self.write_realtime(if pause { RT_HOLD } else { RT_RESUME });
            return;
        }
        if c.job.is_none() || c.paused == pause {
            return;
        }
        c.paused = pause;
        let st = Status { state: c.derived_state(), mpos: c.last_pos, wpos: c.last_pos, ..Default::default() };
        self.emit(Event::Status(st));
        if pause {
            self.info("Pause: die Steuerung hält nach den bereits übertragenen Befehlen an.");
        }
    }

    fn write_realtime(&mut self, b: u8) {
        let Some(c) = &mut self.conn else { return };
        if let Err(e) = c.transport.write_all(&[b]) {
            self.drop_connection(Some(e.to_string()));
        }
    }

    /// Bewegung abbrechen, Laser aus und alle Warteschlangen leeren.
    fn emergency_stop(&mut self) {
        let Some(c) = &mut self.conn else { return };
        let message = match c.firmware {
            Firmware::Grbl => {
                // Vorschub anhalten, dann zurücksetzen (schaltet den Laser ab).
                let _ = c.transport.write_all(&[RT_HOLD]);
                std::thread::sleep(Duration::from_millis(50));
                let _ = c.transport.write_all(&[RT_RESET]);
                c.ready = false;
                c.opened = Instant::now();
                "Stopp: Steuerung zurückgesetzt, Laser aus."
            }
            Firmware::Marlin => {
                // M410 wird vom Notfall-Parser sofort ausgeführt und verwirft den Planer.
                let _ = c.transport.write_all(b"M410\nM5\n");
                "Stopp: Bewegungen abgebrochen (M410), Laser aus (M5)."
            }
            Firmware::Smoothieware => {
                // Strg-X hält sofort an; M112 ist die Rückfallebene für ältere Firmware.
                let _ = c.transport.write_all(b"\x18\nM112\n");
                c.last_state = MachineState::Alarm;
                "Stopp: Steuerung angehalten. Zum Weiterarbeiten „Entsperren“ (M999)."
            }
        };
        c.queue.clear();
        c.inflight.clear();
        c.inflight_bytes = 0;
        c.paused = false;
        if !c.firmware.is_grbl() {
            // Die Antworten auf die Stopp-Befehle still verbuchen.
            c.inflight.extend(std::iter::repeat_n((0, Kind::Poll), 2));
            c.last_send = Instant::now();
        }
        if let Some(job) = c.job.take() {
            let seconds = job.started.elapsed().as_secs_f64();
            self.emit(Event::JobFinished {
                ok: false, message: format!("„{}“ wurde gestoppt.", job.name), seconds
            });
        }
        self.info(message);
    }

    fn drop_connection(&mut self, reason: Option<String>) {
        if let Some(mut c) = self.conn.take() {
            if let Some(job) = c.job.take() {
                // Versuch, den Laser abzuschalten, falls die Verbindung noch teilweise funktioniert.
                let _ = match c.firmware {
                    Firmware::Grbl => c.transport.write_all(&[RT_HOLD, RT_RESET]),
                    Firmware::Marlin => c.transport.write_all(b"M410\nM5\n"),
                    Firmware::Smoothieware => c.transport.write_all(b"M112\n"),
                };
                let _ = self.events.send(Event::JobFinished {
                    ok: false,
                    message: format!("„{}“ abgebrochen: Verbindung getrennt.", job.name),
                    seconds: job.started.elapsed().as_secs_f64(),
                });
            }
            self.emit(Event::Disconnected { reason });
        }
    }

    fn service(&mut self) -> Result<(), String> {
        let Some(c) = &mut self.conn else { return Ok(()) };
        // Empfangen
        let mut buf = [0u8; 1024];
        let mut lines = Vec::new();
        let garbled_before = c.garbled;
        loop {
            let n = c.transport.read(&mut buf).map_err(|e| format!("Lesefehler: {e}"))?;
            if n == 0 {
                break;
            }
            c.rx_buf.extend_from_slice(&buf[..n]);
            c.last_rx = Instant::now();
            while let Some(pos) = c.rx_buf.iter().position(|&b| b == b'\n') {
                let raw: Vec<u8> = c.rx_buf.drain(..=pos).collect();
                let text = String::from_utf8_lossy(&raw).trim().to_string();
                if garbled(&text) {
                    c.garbled += 1;
                    // nur zur Ansicht in der Konsole, nicht auswerten
                    let short: String = text.chars().take(60).collect();
                    lines.push((short + if text.chars().count() > 60 { "…" } else { "" }, false));
                } else if !text.is_empty() {
                    lines.push((text, true));
                }
            }
            if c.rx_buf.len() > MAX_LINE {
                c.rx_buf.clear();
                c.garbled += 1;
            }
            if n < buf.len() {
                break;
            }
        }
        // Einzelne Störzeichen beim Einschalten sind normal; erst wiederholter Zeichensalat deutet auf die Baudrate.
        let warn_garbled = garbled_before < 2 && c.garbled >= 2;
        for (line, valid) in lines {
            if valid {
                self.on_line(line);
            } else {
                self.emit(Event::Console { dir: Direction::Received, text: line });
            }
        }
        if warn_garbled {
            self.emit(Event::Warning(
                "Die Steuerung sendet unverständliche Zeichen – stimmt die Baudrate in den Maschineneinstellungen (GRBL-Laser meist 115200)?".into(),
            ));
        }

        let Some(c) = &mut self.conn else { return Ok(()) };
        if !c.ready && c.opened.elapsed() >= c.startup_wait {
            c.ready = true;
        }
        let grbl = c.firmware.is_grbl();
        let polled = c.firmware != Firmware::Marlin;
        let mut silence = None;
        if polled && c.ready && c.last_rx.elapsed() >= SILENCE_WARNING && !c.silence_warned {
            c.silence_warned = true;
            silence = Some(c.last_rx.elapsed().as_secs());
        } else if c.silence_warned && c.last_rx.elapsed() < SILENCE_WARNING {
            c.silence_warned = false;
            silence = Some(0);
            if c.job.is_none() {
                // Was während der Funkstille unterwegs war, ist vermutlich verloren; sonst wartete jede weitere
                // Zeile auf ein `ok`, das nie kommt (und Jobs würden um eine Bestätigung verschoben gezählt).
                c.inflight.clear();
                c.inflight_bytes = 0;
            }
        }
        // Im Alarm führt auch GRBL nichts mehr aus: unbeantwortete Zeilen nicht ewig als unterwegs zählen.
        if (!grbl || c.last_state == MachineState::Alarm)
            && c.job.is_none()
            && !c.inflight.is_empty()
            && c.last_reply.elapsed() >= STALE_REPLY_TIMEOUT
            && c.last_send.elapsed() >= STALE_REPLY_TIMEOUT
        {
            c.inflight.clear();
            c.inflight_bytes = 0;
        }
        // GRBL: nie mehr Bytes unterwegs, als der Empfangspuffer fasst. Sonst: eine Zeile, dann auf `ok` warten.
        let mut sent = Vec::new();
        while c.ready {
            let Some((line, kind)) = c.queue.front() else { break };
            if c.paused && kind == Kind::Job {
                break;
            }
            let len = line.len() + 1;
            let full = if grbl { c.inflight_bytes + len > c.rx_capacity } else { true };
            if full && !c.inflight.is_empty() {
                break;
            }
            c.transport.write_all(format!("{line}\n").as_bytes()).map_err(|e| format!("Schreibfehler: {e}"))?;
            if kind == Kind::Console {
                sent.push(line.to_string());
            }
            c.queue.pop();
            c.inflight.push_back((len, kind));
            c.inflight_bytes += len;
            c.last_send = Instant::now();
        }
        let mut synthetic = None;
        match c.firmware {
            Firmware::Grbl | Firmware::Smoothieware => {
                if c.last_poll.elapsed() >= STATUS_INTERVAL {
                    c.last_poll = Instant::now();
                    c.transport.write_all(&[RT_STATUS]).map_err(|e| format!("Schreibfehler: {e}"))?;
                }
            }
            Firmware::Marlin => {
                if c.job.is_some() {
                    // Während des Jobs keine Abfragen dazwischenschieben, Zustand selbst melden.
                    if c.last_poll.elapsed() >= STATUS_INTERVAL {
                        c.last_poll = Instant::now();
                        synthetic = Some(Status {
                            state: c.derived_state(),
                            mpos: c.last_pos,
                            wpos: c.last_pos,
                            ..Default::default()
                        });
                    }
                } else if c.ready
                    && c.queue.is_empty()
                    && c.inflight.is_empty()
                    && c.last_poll.elapsed() >= POSITION_POLL_INTERVAL
                {
                    c.last_poll = Instant::now();
                    c.queue.push("M114".into(), Kind::Poll);
                }
            }
        }
        for line in sent {
            self.emit(Event::Console { dir: Direction::Sent, text: line });
        }
        match silence {
            Some(0) => self.info("Die Steuerung antwortet wieder."),
            Some(s) => self.emit(Event::Warning(format!(
                "Die Steuerung antwortet seit {s} s nicht mehr (USB-Kabel, Stromversorgung des Lasers?)."
            ))),
            None => {}
        }
        if let Some(st) = synthetic {
            self.emit(Event::Status(st));
        }
        Ok(())
    }

    fn pop_inflight(&mut self) -> Option<Kind> {
        let c = self.conn.as_mut()?;
        c.last_reply = Instant::now();
        let (len, kind) = c.inflight.pop_front()?;
        c.inflight_bytes -= len;
        Some(kind)
    }

    fn finish_job(&mut self, ok: bool, message: impl FnOnce(&str) -> String) {
        let Some(c) = &mut self.conn else { return };
        let Some(job) = c.job.take() else { return };
        c.paused = false;
        let seconds = job.started.elapsed().as_secs_f64();
        let message = message(&job.name);
        self.emit(Event::JobFinished { ok, message, seconds });
    }

    /// Steuerung meldet einen Halt (Smoothieware `!!`, Marlin `kill()`): Job abbrechen.
    fn halted(&mut self, text: String) {
        if let Some(c) = &mut self.conn {
            c.last_state = MachineState::Alarm;
            c.queue.drop_job();
        }
        self.finish_job(false, |_| format!("Steuerung angehalten: {text}"));
        self.emit(Event::Alarm(0, text));
    }

    fn on_line(&mut self, line: String) {
        let firmware = self.firmware();
        match parse_response(&line) {
            Response::Status(mut st) => {
                if let Some(c) = &mut self.conn {
                    if !firmware.is_grbl() && c.paused {
                        st.state = MachineState::Hold;
                    }
                    c.last_state = st.state;
                    // Job ist fertig, wenn alles bestätigt ist und die Maschine wieder steht.
                    let finished = firmware.is_grbl()
                        && c.job.as_ref().is_some_and(|j| {
                            j.all_acked_at.is_some_and(|t| t.elapsed() > Duration::from_millis(250))
                                && st.state == MachineState::Idle
                        });
                    if finished {
                        self.finish_job(true, |name| format!("„{name}“ ist fertig."));
                    }
                }
                self.emit(Event::Status(st));
            }
            Response::Position(p) => {
                let Some(c) = &mut self.conn else { return };
                c.last_pos = Some(p);
                let st = Status { state: c.derived_state(), mpos: Some(p), wpos: Some(p), ..Default::default() };
                if c.job.is_none() {
                    c.last_state = st.state;
                }
                if c.inflight.front().is_none_or(|(_, k)| *k != Kind::Poll) {
                    self.emit(Event::Console { dir: Direction::Received, text: line });
                }
                self.emit(Event::Status(st));
            }
            Response::Ok => match self.pop_inflight() {
                Some(Kind::Job) => self.job_acked(),
                Some(Kind::Poll) => {}
                _ => self.emit(Event::Console { dir: Direction::Received, text: line }),
            },
            Response::Error(code) => {
                let was_job = self.pop_inflight() == Some(Kind::Job);
                let text = if firmware.is_grbl() { format!("{line} – {}", error_text(code)) } else { line };
                self.emit(Event::Console { dir: Direction::Received, text: text.clone() });
                if was_job {
                    let acked = self.conn.as_ref().and_then(|c| c.job.as_ref()).map(|j| j.acked + 1).unwrap_or(0);
                    self.info(format!("Fehler in Job-Zeile {acked}: {text}"));
                    self.emergency_stop();
                }
            }
            Response::Halt(text) => {
                // Smoothieware beantwortet im Halt jede Zeile mit `!!` statt `ok`.
                self.pop_inflight();
                self.emit(Event::Console { dir: Direction::Received, text: line });
                let already = self.conn.as_ref().is_some_and(|c| c.last_state == MachineState::Alarm);
                if !already || self.conn.as_ref().is_some_and(|c| c.job.is_some()) {
                    self.halted(text);
                }
            }
            Response::Alarm(code) => {
                let text = alarm_text(code).to_string();
                self.emit(Event::Console { dir: Direction::Received, text: format!("{line} – {text}") });
                if let Some(c) = &mut self.conn {
                    c.last_state = MachineState::Alarm;
                    // GRBL verwirft im Alarm Puffer und Planer.
                    c.queue.drop_job();
                }
                self.finish_job(false, |_| format!("Alarm {code}: {text}"));
                self.emit(Event::Alarm(code, text));
            }
            Response::Welcome(_) => {
                let mut rebooted = false;
                if let Some(c) = &mut self.conn {
                    // Nach einem Reset ist der Empfangspuffer der Steuerung leer.
                    c.inflight.clear();
                    c.inflight_bytes = 0;
                    c.ready = true;
                    if c.job.is_some() {
                        // Neustart mitten im Job (Stromausfall, Störung): Laser-Modus und Position sind verloren.
                        // Nichts mehr nachschieben und bereits Unterwegs-Befindliches per Reset verwerfen.
                        rebooted = true;
                        c.queue.drop_job();
                        let _ = c.transport.write_all(&[RT_RESET]);
                    }
                }
                self.emit(Event::Console { dir: Direction::Received, text: line });
                if rebooted {
                    self.finish_job(false, |name| {
                        format!(
                            "Die Steuerung hat während „{name}“ neu gestartet (Stromausfall oder Störung?) – Job \
                             abgebrochen. Position prüfen, ggf. Referenzfahrt."
                        )
                    });
                }
            }
            Response::Setting(n, v) => {
                self.emit(Event::Setting(n, v));
                self.emit(Event::Console { dir: Direction::Received, text: line });
            }
            Response::Feedback(_) | Response::Other(_) => {
                if firmware.is_grbl() && line.contains("Reset to continue") {
                    // Nach einem kritischen Alarm (z. B. Endschalter) nimmt GRBL nichts mehr an, auch kein `$X`,
                    // bis zurückgesetzt wird. Der Laser steht ohnehin: gleich zurücksetzen, damit Entsperren geht.
                    self.emit(Event::Console { dir: Direction::Received, text: line });
                    let Some(c) = &mut self.conn else { return };
                    if c.last_auto_reset.is_none_or(|t| t.elapsed() >= AUTO_RESET_INTERVAL) {
                        c.last_auto_reset = Some(Instant::now());
                        c.queue.drop_job();
                        if let Err(e) = c.transport.write_all(&[RT_RESET]) {
                            self.drop_connection(Some(e.to_string()));
                            return;
                        }
                        self.info("Steuerung nach dem Alarm zurückgesetzt. Weiter mit Referenzfahrt oder Entsperren.");
                    }
                    return;
                }
                if !firmware.is_grbl() {
                    if line == "start" {
                        // Marlin nach dem Neustart (Öffnen des Anschlusses): bereit, nichts mehr unterwegs.
                        if let Some(c) = &mut self.conn {
                            c.inflight.clear();
                            c.inflight_bytes = 0;
                            c.ready = true;
                            c.last_state = MachineState::Idle;
                        }
                    } else if line.contains("busy:") {
                        // Marlin meldet sich bei langen Befehlen (M400, G28) alle paar Sekunden: noch beschäftigt.
                        if let Some(c) = &mut self.conn {
                            c.last_reply = Instant::now();
                        }
                        return;
                    } else if line.starts_with("Error:") && (line.contains("halted") || line.contains("kill")) {
                        self.emit(Event::Console { dir: Direction::Received, text: line.clone() });
                        self.halted(line);
                        return;
                    }
                }
                self.emit(Event::Console { dir: Direction::Received, text: line });
            }
        }
    }

    fn job_acked(&mut self) {
        let Some(c) = &mut self.conn else { return };
        let grbl = c.firmware.is_grbl();
        let Some(job) = &mut c.job else { return };
        job.acked += 1;
        if job.acked >= job.total && job.all_acked_at.is_none() {
            job.all_acked_at = Some(Instant::now());
        }
        let done = job.acked >= job.total;
        if job.last_progress.elapsed() >= PROGRESS_INTERVAL || done {
            job.last_progress = Instant::now();
            let ev = Event::JobProgress { acked: job.acked, total: job.total };
            self.emit(ev);
        }
        if done && !grbl {
            // Die letzte Zeile ist M400: alle Bewegungen sind ausgeführt.
            self.finish_job(true, |name| format!("„{name}“ ist fertig."));
        }
    }
}

/// Zeile aus Zeichensalat, wie ihn eine falsche Baudrate erzeugt: überwiegend Steuer- oder Nicht-ASCII-Zeichen
/// (Steuerungen antworten in ASCII).
fn garbled(text: &str) -> bool {
    let bad = text.chars().filter(|&ch| !ch.is_ascii() || (ch.is_ascii_control() && ch != '\t')).count();
    bad >= 3 && bad * 4 > text.chars().count()
}
