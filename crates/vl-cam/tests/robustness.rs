//! Robustheit beim Öffnen fremder oder beschädigter Dateien: Projektdateien (laden und daraus einen Auftrag
//! planen), Materialbibliotheken und G-Code-Dateien dürfen weder abstürzen noch hängen (siehe `vl_testkit::check`).

use std::sync::Arc;
use vl_cam::{JobSettings, MachineProfile, build_job, lines_from_file};
use vl_core::{
    DitherMode, Document, ImageAdjust, ImageObject, LayerMode, MaterialEntry, MaterialLibrary, Point, RasterImage,
    VectorPath, project,
};
use vl_testkit::check;

/// Projekt mit je einer Ebene pro Bearbeitungsart und den Sonderfunktionen, die eigene Schleifen haben
/// (Laschen, Perforation, Anlauf, Überschnitt, Durchgänge, Kreuzschraffur, Versatz-Füllen, Rastern).
fn project_sample() -> String {
    let mut doc = Document::new();
    doc.add_path("Kontur", 0, VectorPath::rect(10.0, 10.0, 30.0, 20.0, 2.0));
    doc.add_path("Kreis", 1, VectorPath::ellipse(Point::new(60.0, 20.0), 8.0, 6.0));
    doc.add_path("Ring", 2, VectorPath::ellipse(Point::new(90.0, 20.0), 6.0, 6.0));
    doc.add_path("Linie", 4, VectorPath::line(Point::new(0.0, 50.0), Point::new(40.0, 55.0)));
    let pixels = (0..16).map(|i| (i * 16) as u8).collect();
    let image = ImageObject {
        image: Arc::new(RasterImage::new(4, 4, pixels)),
        width_mm: 4.0,
        height_mm: 4.0,
        adjust: ImageAdjust::default(),
        clip: Vec::new(),
    };
    doc.add_image("Bild", 3, image, Point::new(120.0, 20.0));
    let l = &mut doc.layers[0].settings;
    (l.passes, l.tab_count, l.tab_width, l.kerf, l.lead_in, l.lead_out, l.overcut) = (2, 2, 1.0, 0.1, 1.0, 1.0, 0.5);
    let l = &mut doc.layers[1].settings;
    (l.mode, l.interval, l.crosshatch, l.overscan, l.fill_angle) = (LayerMode::Fill, 0.2, true, 2.0, 30.0);
    let l = &mut doc.layers[2].settings;
    (l.mode, l.interval) = (LayerMode::OffsetFill, 0.5);
    let l = &mut doc.layers[3].settings;
    (l.mode, l.interval, l.dither) = (LayerMode::Image, 0.1, DitherMode::Jarvis);
    let l = &mut doc.layers[4].settings;
    (l.perforate, l.perf_cut, l.perf_gap, l.z_step) = (true, 1.0, 0.5, 0.2);
    project::project_to_string(&doc).unwrap()
}

#[test]
fn project_survives_damage() {
    check("projekt", vec![project_sample().into_bytes()], |d| {
        let Ok(doc) = project::project_from_str(&String::from_utf8_lossy(d)) else { return false };
        build_job(&doc, &[], &MachineProfile::default(), &JobSettings::default()).is_ok()
    });
}

#[test]
fn material_library_survives_damage() {
    let mut lib = MaterialLibrary::default();
    lib.entries.push(MaterialEntry { material: "Pappel".into(), thickness: 3.0, ..Default::default() });
    lib.entries.push(MaterialEntry::default());
    check("material", vec![lib.to_json().unwrap().into_bytes()], |d| {
        MaterialLibrary::from_json(&String::from_utf8_lossy(d)).is_ok()
    });
}

#[test]
fn gcode_file_survives_damage() {
    let gcode = "; Kommentar\n%\nG21 G90\nM4 S0\nG0 X10 Y10 F3000\nG1 X20 Y10 S500 F1500 (Klammer)\nG2 X30 Y20 I5 J5\n\
        G3 X10 Y10 R15\n$H\nM5\nM2\n%\n";
    check("gcode", vec![gcode.as_bytes().to_vec()], |d| !lines_from_file(&String::from_utf8_lossy(d)).is_empty());
}
