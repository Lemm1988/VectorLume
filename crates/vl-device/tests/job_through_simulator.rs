//! End-to-End: Dokument → Job → G-Code → GRBL-Simulator. Jede Zeile muss mit `ok` bestätigt werden.

use std::sync::Arc;
use vl_cam::{JobSettings, MachineProfile, StartFrom, build_job, frame_gcode, strip_for_streaming};
use vl_core::{CutSettings, DitherMode, Document, ImageAdjust, ImageObject, LayerMode, Point, RasterImage, VectorPath};
use vl_device::sim::SimTransport;
use vl_device::transport::Transport;

fn sample_doc() -> Document {
    let mut doc = Document::new();
    doc.add_path("Rahmen", 0, VectorPath::rect(20.0, 20.0, 60.0, 40.0, 4.0));
    doc.add_path("Loch", 0, VectorPath::ellipse(Point::new(35.0, 35.0), 5.0, 5.0));
    doc.layers[1].settings =
        CutSettings { fill_angle: 30.0, crosshatch: true, interval: 0.5, ..CutSettings::default_for(LayerMode::Fill) };
    doc.add_path("Stern", 1, VectorPath::star(Point::new(60.0, 40.0), 12.0, 5.0, 5));
    let px = (0..40u32).flat_map(|y| (0..60u32).map(move |x| ((x * 4 + y * 2) % 256) as u8)).collect();
    doc.add_image(
        "Bild",
        2,
        ImageObject {
            image: Arc::new(RasterImage::new(60, 40, px)),
            width_mm: 12.0,
            height_mm: 8.0,
            adjust: ImageAdjust::default(),
        },
        Point::new(100.0, 30.0),
    );
    doc.layers[2].settings.dither = DitherMode::Grayscale;
    doc.layers[2].settings.interval = 0.2;
    doc
}

/// Streamt Zeilen nach dem Zeichenzähl-Verfahren und gibt (ok, Fehlerzeilen) zurück.
fn stream(lines: &[String]) -> (usize, Vec<String>, [f64; 2]) {
    let mut sim = SimTransport::new(1.0e6);
    let mut inflight: std::collections::VecDeque<usize> = Default::default();
    let mut used = 0;
    let mut next = 0;
    let mut oks = 0;
    let mut errors = Vec::new();
    let mut rx = Vec::new();
    let mut buf = [0u8; 4096];
    let mut idle_loops = 0;
    // Begrüßung abholen
    let _ = sim.read(&mut buf);
    while oks + errors.len() < lines.len() {
        // Zeile + Zeilenende muss in den 127-Byte-Puffer passen.
        while next < lines.len() && used + lines[next].len() < 127 {
            sim.write_all(format!("{}\n", lines[next]).as_bytes()).expect("Simulator-Puffer übergelaufen");
            inflight.push_back(lines[next].len() + 1);
            used += lines[next].len() + 1;
            next += 1;
        }
        let n = sim.read(&mut buf).unwrap();
        rx.extend_from_slice(&buf[..n]);
        let mut progressed = false;
        while let Some(pos) = rx.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = rx.drain(..=pos).collect();
            let text = String::from_utf8_lossy(&line).trim().to_string();
            if text == "ok" || text.starts_with("error") {
                let len = inflight.pop_front().expect("Antwort ohne gesendete Zeile");
                used -= len;
                progressed = true;
                if text == "ok" {
                    oks += 1;
                } else {
                    let idx = oks + errors.len();
                    errors.push(format!("{} -> {text}", lines[idx]));
                }
            }
        }
        idle_loops = if progressed { 0 } else { idle_loops + 1 };
        assert!(idle_loops < 200_000, "Simulator antwortet nicht mehr");
    }
    // Restliche Bewegungen ausführen lassen
    for _ in 0..50 {
        let _ = sim.read(&mut buf);
    }
    (oks, errors, sim.position())
}

#[test]
fn absolute_job_runs_without_errors() {
    let doc = sample_doc();
    let profile = MachineProfile::default();
    let job = build_job(&doc, &[], &profile, &JobSettings::default()).unwrap();
    let lines = strip_for_streaming(&job.gcode);
    let (oks, errors, pos) = stream(&lines);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(oks, lines.len());
    // Rückkehr zum Nullpunkt
    assert!(pos[0].abs() < 1e-6 && pos[1].abs() < 1e-6, "{pos:?}");
}

#[test]
fn relative_job_and_frame_return_to_start() {
    let doc = sample_doc();
    let profile = MachineProfile::default();
    let settings = JobSettings { start_from: StartFrom::CurrentPosition, ..Default::default() };
    let job = build_job(&doc, &[], &profile, &settings).unwrap();
    for lines in [strip_for_streaming(&job.gcode), strip_for_streaming(&frame_gcode(&job, &profile))] {
        let (oks, errors, pos) = stream(&lines);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(oks, lines.len());
        // Simulator startet bei 0/0 = aktuelle Position, relativer Job endet wieder dort.
        assert!(pos[0].abs() < 1e-3 && pos[1].abs() < 1e-3, "{pos:?}");
    }
}
