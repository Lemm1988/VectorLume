//! Steuerungs-Firmware: bestimmt G-Code-Dialekt (vl-cam) und Übertragungsverfahren (vl-device).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Firmware {
    /// GRBL 1.1 und kompatible (grblHAL, FluidNC): Zeichenzähl-Verfahren, Echtzeitbefehle, `$`-Einstellungen.
    #[default]
    Grbl,
    /// Marlin 2.1 mit `LASER_FEATURE`: Leistung je Bewegung (`M3 I`), Senden und Warten auf `ok`.
    Marlin,
    /// Smoothieware: Laser feuert bei G1 mit S = 0…1, Statusabfrage mit `?`.
    Smoothieware,
}

impl Firmware {
    pub const ALL: [Firmware; 3] = [Firmware::Grbl, Firmware::Marlin, Firmware::Smoothieware];

    pub fn label(self) -> &'static str {
        match self {
            Firmware::Grbl => "GRBL 1.1 (auch grblHAL, FluidNC)",
            Firmware::Marlin => "Marlin 2.1 (experimentell)",
            Firmware::Smoothieware => "Smoothieware (experimentell)",
        }
    }

    /// Üblicher Höchstwert für S.
    pub fn default_s_max(self) -> f64 {
        match self {
            Firmware::Grbl => 1000.0,
            Firmware::Marlin => 255.0,
            Firmware::Smoothieware => 1.0,
        }
    }

    /// GRBL-Zeichenzähl-Verfahren und Echtzeitbefehle (`?`, `!`, `~`, Strg-X).
    pub fn is_grbl(self) -> bool {
        self == Firmware::Grbl
    }

    pub fn home_command(self) -> &'static str {
        match self {
            Firmware::Grbl | Firmware::Smoothieware => "$H",
            Firmware::Marlin => "G28 X Y",
        }
    }

    /// Sperre bzw. Halt nach Not-Stopp aufheben.
    pub fn unlock_command(self) -> &'static str {
        match self {
            Firmware::Grbl => "$X",
            Firmware::Marlin | Firmware::Smoothieware => "M999",
        }
    }

    /// Zeilen für eine Verfahrbewegung ohne Laser. `axes` z. B. `"X10 Y-5"`.
    pub fn jog_lines(self, relative: bool, axes: &str, feed: f64) -> Vec<String> {
        let mode = if relative { "G91" } else { "G90" };
        let f = feed.round();
        match self {
            Firmware::Grbl => vec![format!("$J={mode} G21 {axes} F{f}")],
            _ => {
                let mut v = vec![mode.to_string(), format!("G0 {axes} F{f}")];
                if relative {
                    v.push("G90".into());
                }
                v
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jog_lines_per_firmware() {
        assert_eq!(Firmware::Grbl.jog_lines(true, "X10", 3000.0), vec!["$J=G91 G21 X10 F3000"]);
        assert_eq!(Firmware::Marlin.jog_lines(true, "Y-2.5", 1500.4), vec!["G91", "G0 Y-2.5 F1500", "G90"]);
        assert_eq!(Firmware::Smoothieware.jog_lines(false, "X0 Y0", 3000.0), vec!["G90", "G0 X0 Y0 F3000"]);
    }
}
