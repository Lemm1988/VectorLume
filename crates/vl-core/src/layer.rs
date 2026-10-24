//! Farbebenen und ihre Laser-Einstellungen.

use serde::{Deserialize, Serialize};

/// Anzahl der Farbebenen in der Palette.
pub const LAYER_COUNT: usize = 30;

/// Eigene Farbpalette von VectorLume (gut lesbar auf dunklem und hellem Hintergrund).
pub const PALETTE: [[u8; 3]; LAYER_COUNT] = [
    [59, 130, 246],
    [239, 68, 68],
    [34, 197, 94],
    [249, 115, 22],
    [168, 85, 247],
    [6, 182, 212],
    [234, 179, 8],
    [236, 72, 153],
    [132, 204, 22],
    [20, 184, 166],
    [99, 102, 241],
    [245, 158, 11],
    [244, 63, 94],
    [16, 185, 129],
    [14, 165, 233],
    [217, 70, 239],
    [161, 98, 7],
    [156, 163, 175],
    [229, 231, 235],
    [185, 28, 28],
    [21, 128, 61],
    [29, 78, 216],
    [101, 163, 13],
    [251, 113, 133],
    [110, 231, 183],
    [196, 181, 253],
    [253, 186, 116],
    [100, 116, 139],
    [202, 138, 4],
    [38, 38, 38],
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LayerMode {
    /// Konturen abfahren (Schneiden, Linien gravieren).
    #[default]
    Line,
    /// Geschlossene Formen zeilenweise ausfüllen.
    Fill,
    /// Bilder rastern (Graustufen oder Dithering).
    Image,
}

impl LayerMode {
    pub const ALL: [LayerMode; 3] = [LayerMode::Line, LayerMode::Fill, LayerMode::Image];

    pub fn label(self) -> &'static str {
        match self {
            LayerMode::Line => "Linie",
            LayerMode::Fill => "Füllen",
            LayerMode::Image => "Bild",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DitherMode {
    Threshold,
    FloydSteinberg,
    #[default]
    Jarvis,
    Stucki,
    Atkinson,
    Burkes,
    Sierra,
    Ordered,
    /// Kein Dithering: Laserleistung folgt dem Grauwert.
    Grayscale,
}

impl DitherMode {
    pub const ALL: [DitherMode; 9] = [
        DitherMode::Threshold,
        DitherMode::FloydSteinberg,
        DitherMode::Jarvis,
        DitherMode::Stucki,
        DitherMode::Atkinson,
        DitherMode::Burkes,
        DitherMode::Sierra,
        DitherMode::Ordered,
        DitherMode::Grayscale,
    ];

    pub fn label(self) -> &'static str {
        match self {
            DitherMode::Threshold => "Schwellwert",
            DitherMode::FloydSteinberg => "Floyd-Steinberg",
            DitherMode::Jarvis => "Jarvis-Judice-Ninke",
            DitherMode::Stucki => "Stucki",
            DitherMode::Atkinson => "Atkinson",
            DitherMode::Burkes => "Burkes",
            DitherMode::Sierra => "Sierra",
            DitherMode::Ordered => "Geordnet (Bayer 8×8)",
            DitherMode::Grayscale => "Graustufen",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CutSettings {
    pub mode: LayerMode,
    /// Vorschub in mm/min.
    pub speed: f64,
    /// Maximale Leistung in Prozent.
    pub power_max: f64,
    /// Minimale Leistung in Prozent (Graustufen, Ecken).
    pub power_min: f64,
    pub passes: u32,
    pub air_assist: bool,
    /// Ebene wird bei der Ausgabe berücksichtigt.
    pub output: bool,
    /// Zeilenabstand für Füllen und Bild in mm.
    pub interval: f64,
    /// Füllwinkel in Grad.
    pub fill_angle: f64,
    pub crosshatch: bool,
    pub bidirectional: bool,
    /// Überlauf am Zeilenende in mm (Laser aus).
    pub overscan: f64,
    pub dither: DitherMode,
    pub negative: bool,
}

impl Default for CutSettings {
    fn default() -> Self {
        Self::default_for(LayerMode::Line)
    }
}

impl CutSettings {
    pub fn default_for(mode: LayerMode) -> Self {
        let (speed, power) = match mode {
            LayerMode::Line => (1500.0, 30.0),
            LayerMode::Fill => (6000.0, 25.0),
            LayerMode::Image => (6000.0, 35.0),
        };
        Self {
            mode,
            speed,
            power_max: power,
            power_min: 0.0,
            passes: 1,
            air_assist: mode == LayerMode::Line,
            output: true,
            interval: 0.1,
            fill_angle: 0.0,
            crosshatch: false,
            bidirectional: true,
            overscan: 2.5,
            dither: DitherMode::Jarvis,
            negative: false,
        }
    }

    pub fn summary(&self) -> String {
        format!("{} · {:.0} mm/min · {:.0}% · {}×", self.mode.label(), self.speed, self.power_max, self.passes)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub index: usize,
    pub name: String,
    pub color: [u8; 3],
    #[serde(default = "default_true")]
    pub visible: bool,
    pub settings: CutSettings,
}

fn default_true() -> bool {
    true
}

impl Layer {
    pub fn new(index: usize) -> Self {
        Self {
            index,
            name: format!("L{:02}", index + 1),
            color: PALETTE[index % LAYER_COUNT],
            visible: true,
            settings: CutSettings::default(),
        }
    }
}
