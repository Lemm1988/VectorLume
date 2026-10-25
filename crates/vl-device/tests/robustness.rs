//! Robustheit gegenüber gestörten Antworten der Steuerung (Rauschen auf der seriellen Leitung, abgeschnittene
//! Zeilen, fremde Firmware) und beschädigten GRBL-Einstellungsdateien (siehe `vl_testkit::check`).

use vl_device::grbl_settings;
use vl_device::protocol::{parse_response, parse_status};
use vl_testkit::check;

/// Typische Zeilen von GRBL 1.1, GRBL 0.9, Marlin und Smoothieware.
const RESPONSES: &str = "Grbl 1.1h ['$' for help]\nok\nerror:20\nALARM:1\n$30=1000\n$130=400.000\n\
[MSG:'$H'|'$X' to unlock]\n[GC:G0 G54 G17 G21 G90 G94 M5 M9 T0 F0 S0]\n[VER:1.1h.20190825:]\n[OPT:VZ,15,128]\n\
<Run|MPos:10.000,20.500,0.000|FS:1500,300|WCO:1.000,2.000,0.000>\n<Hold:0|WPos:1.0,2.0,3.0|Bf:15,128|F:500>\n\
<Idle|MPos:0.000,0.000,0.000|FS:0,0|Pn:XYZ|Ov:100,100,100|A:SF>\n<Idle,MPos:0.000,0.000,0.000,WPos:0.000,0.000,0.000>\n\
X:10.00 Y:-2.50 Z:0.00 E:0.00 Count X:800 Y:-200 Z:0\nok C: X:1.0000 Y:2.0000 Z:0.0000\necho:busy: processing\n\
Smoothie command shell\n";

#[test]
fn controller_responses_survive_noise() {
    check("antworten", vec![RESPONSES.as_bytes().to_vec()], |d| {
        let text = String::from_utf8_lossy(d);
        let mut statuses = 0;
        for line in text.lines() {
            statuses += usize::from(parse_status(line).is_some());
            let _ = parse_response(line);
        }
        statuses >= 4
    });
}

#[test]
fn grbl_settings_file_survives_damage() {
    let file = "; Sicherung der GRBL-Einstellungen\n$0=10\n$1=255 (Haltezeit)\n$30=1000\n$32=1\n$100=80.000\n\
        $110=6000.000\n$130=400.000\nok\n";
    check("grbl-einstellungen", vec![file.as_bytes().to_vec()], |d| {
        grbl_settings::from_file(&String::from_utf8_lossy(d)).is_ok()
    });
}
