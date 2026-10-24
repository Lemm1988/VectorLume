//! Gerätesteuerung für VectorLume: GRBL-Protokoll, serielles Streaming und Simulator.

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
        let dev = Device::spawn(|| {});
        dev.send(Command::Connect { port: SIMULATOR_PORT.into(), baud: 115_200, rx_buffer: 127 });
        let mut probe = Probe { dev, pending: Default::default() };
        let evs = probe.wait_for(Duration::from_secs(5), |e| matches!(e, Event::Setting(32, _)));
        assert!(evs.iter().any(|e| matches!(e, Event::Connected { .. })));
        probe
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
