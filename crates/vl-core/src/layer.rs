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
    /// Geschlossene Formen mit konzentrischen Linien von außen nach innen füllen.
    OffsetFill,
}

impl LayerMode {
    pub const ALL: [LayerMode; 4] = [LayerMode::Line, LayerMode::Fill, LayerMode::OffsetFill, LayerMode::Image];

    pub fn label(self) -> &'static str {
        match self {
            LayerMode::Line => "Linie",
            LayerMode::Fill => "Füllen",
            LayerMode::Image => "Bild",
            LayerMode::OffsetFill => "Versatz-Füllen",
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
    /// Runde Rasterpunkte, deren Größe dem Grauwert folgt (Rasterweite und -winkel einstellbar).
    Halftone,
    /// Parallele Linien, deren Breite dem Grauwert folgt (Kupferstich-Optik).
    LineScreen,
}

impl DitherMode {
    pub const ALL: [DitherMode; 11] = [
        DitherMode::Threshold,
        DitherMode::FloydSteinberg,
        DitherMode::Jarvis,
        DitherMode::Stucki,
        DitherMode::Atkinson,
        DitherMode::Burkes,
        DitherMode::Sierra,
        DitherMode::Ordered,
        DitherMode::Halftone,
        DitherMode::LineScreen,
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
            DitherMode::Halftone => "Halbton (Punkte)",
            DitherMode::LineScreen => "Linienraster",
        }
    }

    /// Modi mit einstellbarer Rasterweite und -winkel.
    pub fn is_screen(self) -> bool {
        matches!(self, DitherMode::Halftone | DitherMode::LineScreen)
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
    /// Schnittbreite in mm (nur Linie): geschlossene Konturen werden um die halbe Breite nach außen versetzt,
    /// Löcher nach innen, damit die Teile maßhaltig werden.
    pub kerf: f64,
    /// Laschen je geschlossener Kontur (0 = keine), damit Teile nicht herausfallen.
    pub tab_count: u32,
    pub tab_width: f64,
    /// Laschen nur an Außenkonturen (nicht an Löchern).
    pub tabs_outer_only: bool,
    /// Fokusversatz in mm gegenüber der Fokushöhe (Z0); positiv = Kopf höher. Nur mit Z-Achse.
    pub z_offset: f64,
    /// Absenkung je weiterem Durchgang in mm (für dicke Materialien). Nur mit Z-Achse.
    pub z_step: f64,
    /// Überschnitt in mm (nur Linie): geschlossene Konturen laufen so weit über den Startpunkt hinaus,
    /// damit sich Anfang und Ende sicher überlappen.
    pub overcut: f64,
    /// Perforieren (nur Linie): abwechselnd `perf_cut` mm schneiden und `perf_gap` mm Laser aus,
    /// z. B. für Falz- und Abreißlinien.
    pub perforate: bool,
    pub perf_cut: f64,
    pub perf_gap: f64,
    /// Rasterweite (Abstand der Punkte bzw. Linien) in mm für Halbton und Linienraster.
    pub screen_cell: f64,
    /// Rasterwinkel in Grad für Halbton und Linienraster.
    pub screen_angle: f64,
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
            LayerMode::OffsetFill => (3000.0, 25.0),
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
            kerf: 0.0,
            tab_count: 0,
            tab_width: 0.5,
            tabs_outer_only: true,
            z_offset: 0.0,
            z_step: 0.0,
            overcut: 0.0,
            perforate: false,
            perf_cut: 2.0,
            perf_gap: 1.0,
            screen_cell: 0.8,
            screen_angle: 45.0,
        }
    }

    /// Schnitt- und Lückenlänge, wenn Perforieren eingeschaltet ist.
    pub fn perforation(&self) -> Option<(f64, f64)> {
        (self.perforate && self.perf_cut > 0.0 && self.perf_gap > 0.0).then_some((self.perf_cut, self.perf_gap))
    }

    /// Z-Höhe für einen Durchgang (0-basiert) oder `None`, wenn keine Z-Bewegung eingestellt ist.
    pub fn z_for_pass(&self, pass: u32) -> Option<f64> {
        (self.z_offset != 0.0 || self.z_step != 0.0).then_some(self.z_offset - pass as f64 * self.z_step)
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
    /// Gesperrte Ebenen werden angezeigt und gelasert, ihre Objekte lassen sich aber nicht auswählen.
    #[serde(default)]
    pub locked: bool,
    pub settings: CutSettings,
    /// Unterebenen: weitere Bearbeitungen derselben Objekte nach `settings`, z. B. erst füllen, dann Kontur
    /// schneiden. Jede hat eigene Werte und einen eigenen Ausgabe-Schalter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sub: Vec<CutSettings>,
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
            locked: false,
            settings: CutSettings::default(),
            sub: Vec::new(),
        }
    }

    /// Hauptbearbeitung und Unterebenen in Ausgabereihenfolge.
    pub fn operations(&self) -> impl Iterator<Item = &CutSettings> {
        std::iter::once(&self.settings).chain(&self.sub)
    }

    pub fn operation(&self, i: usize) -> Option<&CutSettings> {
        if i == 0 { Some(&self.settings) } else { self.sub.get(i - 1) }
    }

    pub fn operation_mut(&mut self, i: usize) -> Option<&mut CutSettings> {
        if i == 0 { Some(&mut self.settings) } else { self.sub.get_mut(i - 1) }
    }

    /// Bearbeitung, die die Bilder der Ebene rastert: die erste im Modus Bild, sonst die Hauptbearbeitung.
    pub fn image_operation(&self) -> usize {
        self.operations().position(|c| c.mode == LayerMode::Image).unwrap_or(0)
    }

    /// Bearbeitet die Bearbeitung `i` Vektorformen? Unterebenen im Modus Bild sind nur für Bilder da.
    pub fn handles_vectors(&self, i: usize) -> bool {
        i == 0 || self.operation(i).is_some_and(|c| c.mode != LayerMode::Image)
    }

    /// Modi aller Bearbeitungen, z. B. „Füllen + Linie“ (deutsche Bezeichnungen).
    pub fn mode_labels(&self) -> Vec<&'static str> {
        self.operations().map(|c| c.mode.label()).collect()
    }

    /// Objekte dieser Ebene können ausgewählt werden.
    pub fn selectable(&self) -> bool {
        self.visible && !self.locked
    }
}
