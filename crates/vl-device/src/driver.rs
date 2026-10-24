//! Gerätetreiber: eigener Thread, der die Verbindung hält, Zeilen nach dem Zeichenzähl-Verfahren
//! streamt, den Status abfragt und Ereignisse an die Oberfläche meldet.

use crate::protocol::{MachineState, Response, Status, alarm_text, error_text, parse_response};
use crate::sim::SimTransport;
use crate::transport::{SIMULATOR_PORT, SerialTransport, Transport};
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const STATUS_INTERVAL: Duration = Duration::from_millis(200);
/// Wartezeit nach dem Öffnen, falls sich die Steuerung nicht meldet (Arduino-Boards starten beim Öffnen neu).
const STARTUP_TIMEOUT: Duration = Duration::from_millis(2500);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

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
    },
    Disconnect,
    /// Einzelne Zeile aus Konsole oder Bedienelementen (wird während eines Jobs abgelehnt).
    Line(String),
    Realtime(u8),
    StartJob {
        name: String,
        lines: Vec<String>,
    },
    Pause,
    Resume,
    /// Sofortiger Stopp: Vorschub anhalten und Steuerung zurücksetzen (Laser aus).
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
    Connected { port: String },
    Disconnected { reason: Option<String> },
    Console { dir: Direction, text: String },
    Status(Status),
    Setting(u32, String),
    JobStarted { name: String, total: usize },
    JobProgress { acked: usize, total: usize },
    JobFinished { ok: bool, message: String, seconds: f64 },
    Alarm(u32, String),
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
    rx_capacity: usize,
    inflight: VecDeque<(usize, bool)>,
    inflight_bytes: usize,
    queue: VecDeque<(String, bool)>,
    job: Option<JobState>,
    rx_buf: Vec<u8>,
    last_poll: Instant,
    ready: bool,
    opened: Instant,
    last_state: MachineState,
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
            Command::Connect { port, baud, rx_buffer } => {
                if self.conn.is_some() {
                    self.drop_connection(None);
                }
                let transport: Result<Box<dyn Transport>, String> = if port == SIMULATOR_PORT {
                    Ok(Box::new(SimTransport::new(1.0)))
                } else {
                    SerialTransport::open(&port, baud)
                        .map(|t| Box::new(t) as Box<dyn Transport>)
                        .map_err(|e| e.to_string())
                };
                match transport {
                    Ok(transport) => {
                        self.conn = Some(Connection {
                            transport,
                            rx_capacity: rx_buffer.max(32),
                            inflight: VecDeque::new(),
                            inflight_bytes: 0,
                            queue: VecDeque::new(),
                            job: None,
                            rx_buf: Vec::new(),
                            last_poll: Instant::now(),
                            ready: false,
                            opened: Instant::now(),
                            last_state: MachineState::Unknown,
                        });
                        self.emit(Event::Connected { port });
                        // Firmware-Info und Einstellungen abfragen (Laser-Modus, S-Max, Verfahrweg).
                        self.enqueue("$I".into(), false);
                        self.enqueue("$$".into(), false);
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
                    Some(false) => self.enqueue(line, false),
                }
            }
            Command::Realtime(b) => self.write_realtime(b),
            Command::StartJob { name, lines } => self.start_job(name, lines),
            Command::Pause => self.write_realtime(RT_HOLD),
            Command::Resume => self.write_realtime(RT_RESUME),
            Command::Stop => {
                if self.conn.is_some() {
                    self.emergency_stop();
                }
            }
        }
    }

    fn enqueue(&mut self, line: String, job: bool) {
        if let Some(c) = &mut self.conn {
            c.queue.push_back((line, job));
        }
    }

    fn start_job(&mut self, name: String, lines: Vec<String>) {
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
        let total = lines.len();
        for l in lines {
            c.queue.push_back((l, true));
        }
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

    fn write_realtime(&mut self, b: u8) {
        let Some(c) = &mut self.conn else { return };
        if let Err(e) = c.transport.write_all(&[b]) {
            self.drop_connection(Some(e.to_string()));
        }
    }

    /// Vorschub anhalten, Steuerung zurücksetzen (schaltet den Laser ab) und alle Warteschlangen leeren.
    fn emergency_stop(&mut self) {
        let Some(c) = &mut self.conn else { return };
        let _ = c.transport.write_all(&[RT_HOLD]);
        std::thread::sleep(Duration::from_millis(50));
        let _ = c.transport.write_all(&[RT_RESET]);
        c.queue.clear();
        c.inflight.clear();
        c.inflight_bytes = 0;
        c.ready = false;
        c.opened = Instant::now();
        if let Some(job) = c.job.take() {
            let seconds = job.started.elapsed().as_secs_f64();
            self.emit(Event::JobFinished {
                ok: false, message: format!("„{}“ wurde gestoppt.", job.name), seconds
            });
        }
        self.info("Stopp: Steuerung zurückgesetzt, Laser aus.");
    }

    fn drop_connection(&mut self, reason: Option<String>) {
        if let Some(mut c) = self.conn.take() {
            if let Some(job) = c.job.take() {
                // Versuch, den Laser abzuschalten, falls die Verbindung noch teilweise funktioniert.
                let _ = c.transport.write_all(&[RT_HOLD, RT_RESET]);
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
        loop {
            let n = c.transport.read(&mut buf).map_err(|e| format!("Lesefehler: {e}"))?;
            if n == 0 {
                break;
            }
            c.rx_buf.extend_from_slice(&buf[..n]);
            if n < buf.len() {
                break;
            }
        }
        let mut lines = Vec::new();
        while let Some(pos) = c.rx_buf.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = c.rx_buf.drain(..=pos).collect();
            let text = String::from_utf8_lossy(&raw).trim().to_string();
            if !text.is_empty() {
                lines.push(text);
            }
        }
        for line in lines {
            self.on_line(line);
        }

        let Some(c) = &mut self.conn else { return Ok(()) };
        if !c.ready && c.opened.elapsed() >= STARTUP_TIMEOUT {
            c.ready = true;
        }
        // Senden nach dem Zeichenzähl-Verfahren: nie mehr Bytes unterwegs, als der Empfangspuffer fasst.
        let mut sent = Vec::new();
        while c.ready {
            let Some((line, _)) = c.queue.front() else { break };
            let len = line.len() + 1;
            if c.inflight_bytes + len > c.rx_capacity && !c.inflight.is_empty() {
                break;
            }
            let (line, job) = c.queue.pop_front().unwrap();
            c.transport.write_all(format!("{line}\n").as_bytes()).map_err(|e| format!("Schreibfehler: {e}"))?;
            c.inflight.push_back((len, job));
            c.inflight_bytes += len;
            if !job {
                sent.push(line);
            }
        }
        if c.last_poll.elapsed() >= STATUS_INTERVAL {
            c.last_poll = Instant::now();
            c.transport.write_all(&[RT_STATUS]).map_err(|e| format!("Schreibfehler: {e}"))?;
        }
        for line in sent {
            self.emit(Event::Console { dir: Direction::Sent, text: line });
        }
        Ok(())
    }

    fn pop_inflight(&mut self) -> Option<bool> {
        let c = self.conn.as_mut()?;
        let (len, job) = c.inflight.pop_front()?;
        c.inflight_bytes -= len;
        Some(job)
    }

    fn on_line(&mut self, line: String) {
        match parse_response(&line) {
            Response::Status(st) => {
                if let Some(c) = &mut self.conn {
                    c.last_state = st.state;
                    // Job ist fertig, wenn alles bestätigt ist und die Maschine wieder steht.
                    let finished = c.job.as_ref().is_some_and(|j| {
                        j.all_acked_at.is_some_and(|t| t.elapsed() > Duration::from_millis(250))
                            && st.state == MachineState::Idle
                    });
                    if finished {
                        let job = c.job.take().unwrap();
                        let seconds = job.started.elapsed().as_secs_f64();
                        self.emit(Event::JobFinished {
                            ok: true,
                            message: format!("„{}“ ist fertig.", job.name),
                            seconds,
                        });
                    }
                }
                self.emit(Event::Status(st));
            }
            Response::Ok => {
                if self.pop_inflight() == Some(true) {
                    self.job_acked();
                } else {
                    self.emit(Event::Console { dir: Direction::Received, text: line });
                }
            }
            Response::Error(code) => {
                let was_job = self.pop_inflight() == Some(true);
                let text = format!("{line} – {}", error_text(code));
                self.emit(Event::Console { dir: Direction::Received, text: text.clone() });
                if was_job {
                    let acked = self.conn.as_ref().and_then(|c| c.job.as_ref()).map(|j| j.acked + 1).unwrap_or(0);
                    self.info(format!("Fehler in Job-Zeile {acked}: {text}"));
                    self.emergency_stop();
                }
            }
            Response::Alarm(code) => {
                let text = alarm_text(code).to_string();
                self.emit(Event::Console { dir: Direction::Received, text: format!("{line} – {text}") });
                if let Some(c) = &mut self.conn {
                    c.last_state = MachineState::Alarm;
                    // GRBL verwirft im Alarm Puffer und Planer.
                    c.queue.retain(|(_, job)| !job);
                    if let Some(job) = c.job.take() {
                        let seconds = job.started.elapsed().as_secs_f64();
                        self.emit(Event::JobFinished { ok: false, message: format!("Alarm {code}: {text}"), seconds });
                    }
                }
                self.emit(Event::Alarm(code, text));
            }
            Response::Welcome(_) => {
                if let Some(c) = &mut self.conn {
                    // Nach einem Reset ist der Empfangspuffer der Steuerung leer.
                    c.inflight.clear();
                    c.inflight_bytes = 0;
                    c.ready = true;
                }
                self.emit(Event::Console { dir: Direction::Received, text: line });
            }
            Response::Setting(n, v) => {
                self.emit(Event::Setting(n, v));
                self.emit(Event::Console { dir: Direction::Received, text: line });
            }
            Response::Feedback(_) | Response::Other(_) => {
                self.emit(Event::Console { dir: Direction::Received, text: line });
            }
        }
    }

    fn job_acked(&mut self) {
        let Some(c) = &mut self.conn else { return };
        let Some(job) = &mut c.job else { return };
        job.acked += 1;
        if job.acked >= job.total && job.all_acked_at.is_none() {
            job.all_acked_at = Some(Instant::now());
        }
        if job.last_progress.elapsed() >= PROGRESS_INTERVAL || job.acked >= job.total {
            job.last_progress = Instant::now();
            let ev = Event::JobProgress { acked: job.acked, total: job.total };
            self.emit(ev);
        }
    }
}
