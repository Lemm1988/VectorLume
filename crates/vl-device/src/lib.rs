//! Gerätesteuerung für VectorLume: GRBL-, Marlin- und Smoothieware-Protokoll, serielles Streaming und Simulator.

pub mod driver;
pub mod protocol;
pub mod sim;
pub mod transport;

pub use driver::{Command, Device, Direction, Event, RT_JOG_CANCEL, RT_RESET, RT_STATUS};
pub use protocol::{MachineState, Status, alarm_text, error_text};
pub use transport::{PortInfo, SIMULATOR_PORT, available_ports};

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
        dev.send(Command::StartJob { name: "Marlin".into(), lines });
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
        dev.send(Command::StartJob { name: "Pause".into(), lines });
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
        dev.send(Command::StartJob { name: "Lang".into(), lines });
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Status(s) if s.state == MachineState::Run));
        dev.send(Command::Stop);
        let evs = dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::JobFinished { .. }));
        assert_eq!(finished(&evs).map(|f| f.0), Some(false));
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Status(s) if s.state == MachineState::Alarm));
        dev.send(Command::StartJob { name: "Neu".into(), lines: vec!["G0 X1".into()] });
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::JobFinished { ok: false, .. }));
        dev.send(Command::Line("M999".into()));
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Status(s) if s.state == MachineState::Idle));
        dev.send(Command::StartJob { name: "Kurz".into(), lines: vec!["G1 X1 F6000 S0.2".into()] });
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
        dev.send(Command::StartJob { name: "Test".into(), lines });
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
        dev.send(Command::StartJob { name: "Lang".into(), lines });
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::JobStarted { .. }));
        std::thread::sleep(Duration::from_millis(300));
        dev.send(Command::Stop);
        let evs = dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::JobFinished { .. }));
        assert!(evs.iter().any(|e| matches!(e, Event::JobFinished { ok: false, .. })));
        // Reset während der Bewegung -> Alarm, neue Jobs werden abgelehnt.
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Alarm(3, _)));
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Status(s) if s.state == MachineState::Alarm));
        dev.send(Command::StartJob { name: "Neu".into(), lines: vec!["G0 X1".into()] });
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::JobFinished { ok: false, .. }));
        dev.send(Command::Line("$X".into()));
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Status(s) if s.state == MachineState::Idle));
    }

    #[test]
    fn rejects_console_lines_during_job() {
        let mut dev = connect_sim();
        dev.send(Command::StartJob { name: "Lang".into(), lines: vec!["G1 X100 F100 S10".into()] });
        dev.wait_for(Duration::from_secs(5), |e| matches!(e, Event::JobStarted { .. }));
        dev.send(Command::Line("G0 X0".into()));
        dev.wait_for(
            Duration::from_secs(5),
            |e| matches!(e, Event::Console { dir: Direction::Info, text } if text.contains("keine Befehle")),
        );
        dev.send(Command::Stop);
    }
}
