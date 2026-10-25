//! Gerätesteuerung für VectorLume: GRBL-, Marlin- und Smoothieware-Protokoll, serielles Streaming und Simulator.

pub mod driver;
pub mod grbl_settings;
pub mod protocol;
pub mod sim;
pub mod transport;

pub use driver::{Command, Device, Direction, Event, RT_JOG_CANCEL, RT_RESET, RT_STATUS};
pub use protocol::{MachineState, Status, alarm_text, error_text};
pub use transport::{NETWORK_PORT, PortInfo, SIMULATOR_PORT, TCP_PREFIX, available_ports, with_default_port};

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use vl_core::Firmware;

    /// Testhilfe: puffert Ereignisse, damit beim Warten keine nachfolgenden verloren gehen.
    struct Probe {
        dev: Device,
        pending: std::collections::VecDeque<Event>,
    }

    impl Probe {
        fn send(&self, cmd: Command) {
            self.dev.send(cmd);
        }

        fn wait_for(&mut self, timeout: Duration, mut pred: impl FnMut(&Event) -> bool) -> Vec<Event> {
            let start = Instant::now();
            let mut seen = Vec::new();
            loop {
                while let Some(ev) = self.pending.pop_front() {
                    let hit = pred(&ev);
                    seen.push(ev);
                    if hit {
                        return seen;
                    }
                }
                if start.elapsed() > timeout {
                    let tail: Vec<_> = seen.iter().rev().take(5).collect();
                    panic!("Zeitüberschreitung, letzte Ereignisse: {tail:?}");
                }
                self.pending.extend(self.dev.poll());
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    fn connect_sim() -> Probe {
        connect_sim_with(Firmware::Grbl)
    }

    fn connect_sim_with(firmware: Firmware) -> Probe {
        let dev = Device::spawn(|| {});
        dev.send(Command::Connect { port: SIMULATOR_PORT.into(), baud: 115_200, rx_buffer: 127, firmware });
        let mut probe = Probe { dev, pending: Default::default() };
        let evs = probe.wait_for(Duration::from_secs(5), |e| match firmware {
            Firmware::Grbl => matches!(e, Event::Setting(32, _)),
            _ => matches!(e, Event::Console { text, .. } if text.contains("FIRMWARE_NAME")),
        });
        assert!(evs.iter().any(|e| matches!(e, Event::Connected { .. })));
        probe
    }

    fn finished(evs: &[Event]) -> Option<(bool, String)> {
        evs.iter().find_map(|e| match e {
            Event::JobFinished { ok, message, .. } => Some((*ok, message.clone())),
            _ => None,
        })
    }

    fn moves(n: usize, x0: usize) -> Vec<String> {
        (0..n).map(|i| format!("G1 X{} Y{} F3000 S100", x0 + i, i % 3)).collect()
    }

    fn console_has(evs: &[Event], needle: &str) -> bool {
        evs.iter().any(|e| matches!(e, Event::Console { text, .. } if text.contains(needle)))
    }

    /// Nach einem Störfall muss die Verbindung weiter funktionieren: Entsperren und ein neuer Job laufen durch.
    fn still_usable(dev: &mut Probe) {
        dev.send(Command::Line("$X".into()));
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Console { text, .. } if text.contains("Unlocked")));
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Status(s) if s.state == MachineState::Idle));
        dev.send(Command::StartJob { name: "Danach".into(), lines: moves(20, 0).into() });
        let evs = dev.wait_for(Duration::from_secs(20), |e| matches!(e, Event::JobFinished { .. }));
        assert_eq!(finished(&evs).map(|f| f.0), Some(true), "{evs:?}");
    }

    /// Stromausfall mitten im Job: kein weiteres Nachschieben (Laser-Modus und Position wären verloren).
    #[test]
    fn controller_reboot_aborts_job() {
        let mut dev = connect_sim();
        let mut lines = moves(30, 0);
        lines.push("$SIM=REBOOT".into());
        lines.extend(moves(200, 100));
        dev.send(Command::StartJob { name: "Neustart".into(), lines: lines.into() });
        let evs = dev.wait_for(Duration::from_secs(20), |e| matches!(e, Event::JobFinished { .. }));
        let (ok, message) = finished(&evs).unwrap();
        assert!(!ok && message.contains("neu gestartet"), "{message}");
        // die Steuerung verlangt nach dem Neustart Referenzfahrt; danach geht alles wie vorher
        let evs =
            dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Status(s) if s.state == MachineState::Alarm));
        assert!(!evs.iter().any(|e| matches!(e, Event::JobProgress { acked, .. } if *acked > 31)));
        still_usable(&mut dev);
    }

    /// Endschalter mitten im Job: GRBL nimmt bis zum Zurücksetzen nichts an, auch kein `$X`.
    #[test]
    fn hard_limit_alarm_resets_and_unlocks() {
        let mut dev = connect_sim();
        let mut lines = moves(30, 0);
        lines.push("$SIM=LIMIT".into());
        lines.extend(moves(200, 100));
        dev.send(Command::StartJob { name: "Endschalter".into(), lines: lines.into() });
        let evs = dev.wait_for(Duration::from_secs(20), |e| matches!(e, Event::JobFinished { .. }));
        let (ok, message) = finished(&evs).unwrap();
        assert!(!ok && message.starts_with("Alarm 1"), "{message}");
        let evs = dev.wait_for(
            Duration::from_secs(5),
            |e| matches!(e, Event::Console { text, .. } if text.contains("zurückgesetzt")),
        );
        assert!(console_has(&evs, "Reset to continue"));
        still_usable(&mut dev);
    }

    /// Eingefrorene Steuerung: nach einigen Sekunden ohne Antwort wird gewarnt, danach Entwarnung.
    #[test]
    fn silent_controller_is_reported() {
        let mut dev = connect_sim();
        dev.send(Command::Line("$SIM=HANG=5000".into()));
        let evs = dev.wait_for(Duration::from_secs(8), |e| matches!(e, Event::Warning(_)));
        let Some(Event::Warning(text)) = evs.last() else { unreachable!() };
        assert!(text.contains("antwortet seit"), "{text}");
        dev.wait_for(
            Duration::from_secs(5),
            |e| matches!(e, Event::Console { text, .. } if text.contains("antwortet wieder")),
        );
        dev.send(Command::StartJob { name: "Danach".into(), lines: moves(10, 0).into() });
        let evs = dev.wait_for(Duration::from_secs(20), |e| matches!(e, Event::JobFinished { .. }));
        assert_eq!(finished(&evs).map(|f| f.0), Some(true), "{evs:?}");
    }

    /// Netzwerkverbindung: ein lokaler TCP-Server reicht die Bytes an den Simulator weiter.
    #[test]
    fn streams_over_tcp() {
        use crate::transport::Transport;
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_millis(2))).unwrap();
            let mut sim = sim::SimTransport::new(1000.0);
            let mut buf = [0u8; 512];
            loop {
                match sock.read(&mut buf) {
                    Ok(0) => return,
                    Ok(n) => sim.write_all(&buf[..n]).unwrap(),
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                    Err(_) => return,
                }
                let n = sim.read(&mut buf).unwrap();
                if n > 0 && sock.write_all(&buf[..n]).is_err() {
                    return;
                }
            }
        });
        let dev = Device::spawn(|| {});
        let port = format!("{TCP_PREFIX}{addr}");
        dev.send(Command::Connect { port, baud: 0, rx_buffer: 127, firmware: Firmware::Grbl });
        let mut dev = Probe { dev, pending: Default::default() };
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Setting(32, _)));
        let lines: Vec<String> = (0..50).map(|i| format!("G1 X{} Y{} F6000 S100", i % 7, i % 5)).collect();
        dev.send(Command::StartJob { name: "Netz".into(), lines: lines.into() });
        let evs = dev.wait_for(Duration::from_secs(10), |e| matches!(e, Event::JobFinished { .. }));
        assert_eq!(finished(&evs).map(|f| f.0), Some(true));
        dev.send(Command::Disconnect);
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Disconnected { .. }));
    }

    #[test]
    fn tcp_connect_failure_is_reported() {
        let dev = Device::spawn(|| {});
        // Port 1 auf localhost ist praktisch nie offen
        dev.send(Command::Connect {
            port: format!("{TCP_PREFIX}127.0.0.1:1"),
            baud: 0,
            rx_buffer: 127,
            firmware: Firmware::Grbl,
        });
        let mut dev = Probe { dev, pending: Default::default() };
        dev.wait_for(Duration::from_secs(10), |e| matches!(e, Event::Disconnected { reason: Some(_) }));
    }

    #[test]
    fn marlin_streams_with_ok_and_m400() {
        let mut dev = connect_sim_with(Firmware::Marlin);
        // Ruhezustand: Position kommt über M114, ohne die Konsole zu füllen
        let evs = dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Status(s) if s.mpos.is_some()));
        assert!(!evs.iter().any(|e| matches!(e, Event::Console { text, .. } if text.starts_with("X:"))));
        let mut lines = vec!["G21".to_string(), "G90".into(), "M4 I S0".into()];
        for i in 0..60 {
            lines.push(format!("G1 X{:.2} Y{:.2} F60000 S{}", (i % 10) as f64 * 0.1, (i / 10) as f64 * 0.1, i));
        }
        lines.push("G0 X3 Y4".into());
        lines.push("M5".into());
        let total = lines.len() + 1;
        dev.send(Command::StartJob { name: "Marlin".into(), lines: lines.into() });
        let evs = dev.wait_for(Duration::from_secs(20), |e| matches!(e, Event::JobFinished { .. }));
        assert_eq!(finished(&evs).map(|f| f.0), Some(true), "{:?}", finished(&evs));
        assert!(evs.iter().any(|e| matches!(e, Event::JobStarted { total: t, .. } if *t == total)));
        // Nach M400 steht die Maschine am Ziel der letzten Bewegung
        dev.wait_for(
            Duration::from_secs(5),
            |e| matches!(e, Event::Status(s) if s.state == MachineState::Idle && s.mpos == Some([3.0, 4.0, 0.0])),
        );
    }

    #[test]
    fn marlin_pause_and_resume_on_host() {
        let mut dev = connect_sim_with(Firmware::Marlin);
        let lines: Vec<String> = (0..40).map(|i| format!("G1 X{} F3000 S10", (i % 2) * 2)).collect();
        dev.send(Command::StartJob { name: "Pause".into(), lines: lines.into() });
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::JobStarted { .. }));
        dev.send(Command::Pause);
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Status(s) if s.state == MachineState::Hold));
        let before = dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Status(_)));
        assert!(finished(&before).is_none());
        dev.send(Command::Resume);
        let evs = dev.wait_for(Duration::from_secs(20), |e| matches!(e, Event::JobFinished { .. }));
        assert_eq!(finished(&evs).map(|f| f.0), Some(true));
    }

    #[test]
    fn smoothie_stop_halts_until_m999() {
        let mut dev = connect_sim_with(Firmware::Smoothieware);
        let lines = vec!["G1 X300 Y300 F100 S0.5".to_string(); 2];
        dev.send(Command::StartJob { name: "Lang".into(), lines: lines.into() });
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Status(s) if s.state == MachineState::Run));
        dev.send(Command::Stop);
        let evs = dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::JobFinished { .. }));
        assert_eq!(finished(&evs).map(|f| f.0), Some(false));
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Status(s) if s.state == MachineState::Alarm));
        dev.send(Command::StartJob { name: "Neu".into(), lines: ["G0 X1"].into() });
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::JobFinished { ok: false, .. }));
        dev.send(Command::Line("M999".into()));
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Status(s) if s.state == MachineState::Idle));
        dev.send(Command::StartJob { name: "Kurz".into(), lines: ["G1 X1 F6000 S0.2"].into() });
        let evs = dev.wait_for(Duration::from_secs(10), |e| matches!(e, Event::JobFinished { .. }));
        assert_eq!(finished(&evs).map(|f| f.0), Some(true));
    }

    #[test]
    fn streams_job_through_simulator() {
        let mut dev = connect_sim();
        let mut lines = vec!["G21".to_string(), "G90".into(), "M4 S0".into(), "G0 X0 Y0".into()];
        for i in 0..400 {
            // Viele kurze Zeilen füllen den Empfangspuffer – der Simulator meldet einen Überlauf als Fehler.
            lines.push(format!(
                "G1 X{:.3} Y{:.3} F30000 S{}",
                (i % 20) as f64 * 0.05,
                (i / 20) as f64 * 0.05,
                i % 1000
            ));
        }
        lines.push("M5".into());
        let total = lines.len();
        dev.send(Command::StartJob { name: "Test".into(), lines: lines.into() });
        let evs = dev.wait_for(Duration::from_secs(20), |e| matches!(e, Event::JobFinished { .. }));
        let finished = evs.iter().find_map(|e| match e {
            Event::JobFinished { ok, message, .. } => Some((*ok, message.clone())),
            _ => None,
        });
        assert_eq!(finished.as_ref().map(|f| f.0), Some(true), "{finished:?}");
        assert!(evs.iter().any(|e| matches!(e, Event::JobProgress { acked, .. } if *acked == total)));
    }

    #[test]
    fn stop_resets_and_reports() {
        let mut dev = connect_sim();
        let lines = vec!["G1 X300 Y300 F100 S500".to_string(); 3];
        dev.send(Command::StartJob { name: "Lang".into(), lines: lines.into() });
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::JobStarted { .. }));
        std::thread::sleep(Duration::from_millis(300));
        dev.send(Command::Stop);
        let evs = dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::JobFinished { .. }));
        assert!(evs.iter().any(|e| matches!(e, Event::JobFinished { ok: false, .. })));
        // Reset während der Bewegung -> Alarm, neue Jobs werden abgelehnt.
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Alarm(3, _)));
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Status(s) if s.state == MachineState::Alarm));
        dev.send(Command::StartJob { name: "Neu".into(), lines: ["G0 X1"].into() });
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::JobFinished { ok: false, .. }));
        dev.send(Command::Line("$X".into()));
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Status(s) if s.state == MachineState::Idle));
    }

    #[test]
    fn rejects_console_lines_during_job() {
        let mut dev = connect_sim();
        dev.send(Command::StartJob { name: "Lang".into(), lines: ["G1 X100 F100 S10"].into() });
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::JobStarted { .. }));
        dev.send(Command::Line("G0 X0".into()));
        dev.wait_for(
            Duration::from_secs(5),
            |e| matches!(e, Event::Console { dir: Direction::Info, text } if text.contains("keine Befehle")),
        );
        dev.send(Command::Stop);
    }
}
