//! Maßhaltigkeit: Was gezeichnet ist, steht exakt so im G-Code – unabhängig von Drehung, Nullpunkt-Ecke und
//! Startart. Der Schnittbreitenausgleich verschiebt jede Kante um genau die halbe Schnittbreite.

use vl_cam::{JobSettings, MachineProfile, OriginCorner, StartFrom, build_job};
use vl_core::{Affine, Document, Point, VectorPath};

/// Teil mit drei Steckschlitzen (3,8 mm breit, 10 mm tief) an der Oberkante und einem quadratischen Loch.
fn part(doc: &mut Document, angle_deg: f64) -> Vec<Point> {
    let (x0, y0) = (40.0, 50.0);
    let mut outline = vec![Point::new(x0, y0)];
    for sx in [10.0, 28.1, 46.2] {
        outline.extend([
            Point::new(x0 + sx, y0),
            Point::new(x0 + sx, y0 + 10.0),
            Point::new(x0 + sx + 3.8, y0 + 10.0),
            Point::new(x0 + sx + 3.8, y0),
        ]);
    }
    outline.extend([Point::new(x0 + 60.0, y0), Point::new(x0 + 60.0, y0 + 30.0), Point::new(x0, y0 + 30.0)]);
    let hole = vec![
        Point::new(x0 + 25.0, y0 + 15.0),
        Point::new(x0 + 35.0, y0 + 15.0),
        Point::new(x0 + 35.0, y0 + 25.0),
        Point::new(x0 + 25.0, y0 + 25.0),
    ];
    let rot = Affine::rotate_about(angle_deg.to_radians(), Point::new(x0 + 30.0, y0 + 15.0));
    let id1 = doc.add_path("Teil", 0, VectorPath::polygon(&outline, true));
    let id2 = doc.add_path("Loch", 0, VectorPath::polygon(&hole, true));
    doc.transform_objects(&[id1, id2], &rot);
    outline.iter().chain(&hole).map(|&p| rot.apply(p)).collect()
}

/// Gebrannte Polylinien aus dem G-Code (absolut oder relativ), in Ausgabekoordinaten.
fn burned(code: &[String]) -> Vec<Vec<Point>> {
    let (mut x, mut y) = (0.0, 0.0);
    let (mut relative, mut motion, mut s) = (false, 0, 0.0);
    let mut out: Vec<Vec<Point>> = Vec::new();
    let mut fresh = true;
    for line in code {
        let line = line.split(';').next().unwrap();
        let (mut nx, mut ny, mut moved) = (x, y, false);
        for w in line.split_whitespace() {
            let (c, v) = w.split_at(1);
            match c {
                "G" => match v {
                    "0" | "1" => motion = v.parse().unwrap(),
                    "90" => relative = false,
                    "91" => relative = true,
                    _ => {}
                },
                "X" => {
                    let v: f64 = v.parse().unwrap();
                    nx = if relative { x + v } else { v };
                    moved = true;
                }
                "Y" => {
                    let v: f64 = v.parse().unwrap();
                    ny = if relative { y + v } else { v };
                    moved = true;
                }
                "S" => s = v.parse().unwrap(),
                _ => {}
            }
        }
        if !moved {
            continue;
        }
        if motion == 1 && s > 0.0 {
            if fresh {
                out.push(vec![Point::new(x, y)]);
                fresh = false;
            }
            out.last_mut().unwrap().push(Point::new(nx, ny));
        } else {
            fresh = true;
        }
        (x, y) = (nx, ny);
    }
    out
}

/// Breiten aller rechteckigen Schlitze: drei Strecken a-b-c-d, a-b und c-d parallel und gegenläufig, b-c senkrecht.
fn slot_widths(polys: &[Vec<Point>]) -> Vec<f64> {
    let mut out = Vec::new();
    for p in polys {
        let n = p.len() - 1; // geschlossen: letzter Punkt = erster
        for i in 0..n {
            let [a, b, c, d] = std::array::from_fn(|k| p[(i + k) % n]);
            let (v1, v2, v3) = (b - a, c - b, d - c);
            let (l1, l2, l3) = (v1.length(), v2.length(), v3.length());
            if !(2.0..6.0).contains(&l2) || !(5.0..15.0).contains(&l1) || !(5.0..15.0).contains(&l3) {
                continue;
            }
            // Winkeltoleranz 0,1°: der G-Code rundet jede Koordinate auf 1 µm
            if v1.dot(v3) / (l1 * l3) < -0.99999 && (v1.dot(v2) / (l1 * l2)).abs() < 2e-3 {
                out.push(l2);
            }
        }
    }
    out
}

/// Fasst Teilstücke auf einer Geraden zusammen (die Flächenberechnung teilt Kanten an den Ecken manchmal auf).
fn merge_collinear(p: &[Point]) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::new();
    for &q in p {
        if out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let (u, v) = (b - a, q - b);
            // b liegt höchstens 2 µm neben der Geraden a–q (Rundung auf 1 µm)
            if (u.x * v.y - u.y * v.x).abs() < 0.002 * (q - a).length() && u.dot(v) > 0.0 {
                out.pop();
            }
        }
        out.push(q);
    }
    out
}

fn job_code(doc: &Document, profile: &MachineProfile, settings: &JobSettings) -> (Vec<String>, vl_cam::Job) {
    let job = build_job(doc, &[], profile, settings).unwrap();
    (job.gcode.iter().map(str::to_string).collect(), job)
}

#[test]
fn drawn_geometry_is_output_exactly() {
    for angle in [0.0, 37.0, 90.0, 180.0, 293.5] {
        for origin in OriginCorner::ALL {
            for start in [StartFrom::Absolute, StartFrom::UserOrigin, StartFrom::CurrentPosition] {
                let mut doc = Document::new();
                let design = part(&mut doc, angle);
                let profile = MachineProfile { origin, ..MachineProfile::default() };
                let settings =
                    JobSettings { start_from: start, user_origin: Point::new(55.5, 66.25), ..Default::default() };
                let (code, job) = job_code(&doc, &profile, &settings);
                let polys = burned(&code);
                let ctx = format!("{angle}° {origin:?} {start:?}");
                assert_eq!(polys.len(), 2, "{ctx}");
                // Jede gezeichnete Ecke steht auf den µm genau im G-Code und umgekehrt
                let expected: Vec<Point> = design.iter().map(|&p| job.mapping.map(p)).collect();
                for p in polys.iter().flatten() {
                    let d = expected.iter().map(|e| e.distance(*p)).fold(f64::MAX, f64::min);
                    assert!(d < 0.001, "{ctx}: G-Code-Punkt {p:?} liegt {d} mm neben der Zeichnung");
                }
                for e in &expected {
                    let d = polys.iter().flatten().map(|p| p.distance(*e)).fold(f64::MAX, f64::min);
                    assert!(d < 0.001, "{ctx}: Ecke {e:?} fehlt im G-Code");
                }
                // Unabhängig von der Abbildung gemessen: alle drei Schlitze 3,8 mm
                let w = slot_widths(&polys);
                assert_eq!(w.len(), 3, "{ctx}: {w:?}");
                assert!(w.iter().all(|w| (w - 3.8).abs() < 0.0015), "{ctx}: {w:?}");
            }
        }
    }
}

#[test]
fn kerf_offset_moves_every_edge_by_half_the_kerf() {
    for angle in [0.0, 37.0, 180.0] {
        let mut doc = Document::new();
        part(&mut doc, angle);
        doc.layers[0].settings.kerf = 0.2;
        let (code, _) = job_code(&doc, &MachineProfile::default(), &JobSettings::default());
        let polys = burned(&code);
        // Außenkontur wächst, das Loch schrumpft, die Schlitze werden um die volle Schnittbreite schmaler
        let w = slot_widths(&polys);
        assert_eq!(w.len(), 3, "{angle}°: {w:?}");
        assert!(w.iter().all(|w| (w - 3.6).abs() < 0.0015), "{angle}°: {w:?}");
        let mut lens: Vec<f64> = polys
            .iter()
            .map(|p| merge_collinear(p))
            .flat_map(|p| p.windows(2).map(|s| s[0].distance(s[1])).collect::<Vec<_>>())
            .filter(|l| *l > 9.0)
            .collect();
        lens.sort_by(f64::total_cmp);
        let has = |v: f64| lens.iter().any(|l| (l - v).abs() < 0.0015);
        assert!(has(60.2), "{angle}°: Unterkante 60,2 mm erwartet: {lens:?}");
        assert!(has(30.2), "{angle}°: Seitenkante 30,2 mm erwartet: {lens:?}");
        // Loch 9,8 mm; die Schlitzflanken bleiben 10 mm (Oberkante und Schlitzgrund wandern beide um 0,1 mm)
        assert!(has(9.8) && has(10.0), "{angle}°: {lens:?}");
        assert_eq!(lens.iter().filter(|l| (*l - 9.8).abs() < 0.0015).count(), 4, "{angle}°: Loch: {lens:?}");
    }
}

#[test]
fn backlash_compensation_keeps_drawing_in_machine_terms() {
    // Mit Spielausgleich bleiben die Schlitze im G-Code nicht 3,8 mm (der Motor fährt bewusst anders), aber jede
    // Zeichnungsecke wird angefahren – die Prüfung auf der Mechanik steckt in den Tests von `gcode`.
    let mut doc = Document::new();
    part(&mut doc, 0.0);
    let profile = MachineProfile { backlash_x: 0.15, backlash_y: 0.1, ..MachineProfile::default() };
    let (code, _) = job_code(&doc, &profile, &JobSettings::default());
    assert!(!code.iter().any(|l| l.contains("X-") || l.contains("Y-")), "nie unter 0");
    let plain = job_code(&doc, &MachineProfile::default(), &JobSettings::default()).0;
    assert!(code.len() > plain.len(), "Aufholzüge an den Umkehrpunkten");
}

#[test]
fn outer_before_inner_is_pointed_out() {
    let mut doc = Document::new();
    part(&mut doc, 0.0);
    let note = |settings: &JobSettings| job_code(&doc, &MachineProfile::default(), settings).1.notes.join("\n");
    assert!(!note(&JobSettings::default()).contains("Innen vor außen"), "Standard: Löcher zuerst");
    let off = JobSettings { inner_first: false, ..Default::default() };
    assert!(note(&off).contains("Innen vor außen"));
    // Ohne Loch im Teil kein Hinweis
    let mut plain = Document::new();
    plain.add_path(
        "Rechteck",
        0,
        VectorPath::polygon(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 5.0)], true),
    );
    let job = build_job(&plain, &[], &MachineProfile::default(), &off).unwrap();
    assert!(job.notes.iter().all(|n| !n.contains("Innen vor außen")));
}
