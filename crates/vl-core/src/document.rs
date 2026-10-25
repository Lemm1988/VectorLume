//! Das Dokument: Objekte (Pfade und Bilder) auf Farbebenen.

use crate::geom::{Affine, Point, Rect};
use crate::layer::{LAYER_COUNT, Layer, LayerMode};
use crate::path::{Polyline, VectorPath};
use base64::Engine as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashSet;
use std::sync::Arc;

/// Menge für schnelle Mitgliedschaftsprüfung (Auswahl mit Tausenden Objekten).
fn id_set(ids: &[ObjectId]) -> HashSet<ObjectId> {
    ids.iter().copied().collect()
}

pub type ObjectId = u64;
pub type GroupId = u64;

/// Graustufenbild (0 = schwarz, 255 = weiß). Transparenz wird beim Import auf Weiß gerechnet.
/// `Eq` erlaubt `Arc<RasterImage>`-Vergleiche über den Zeiger (schnelle Undo-Prüfung).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RasterImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl RasterImage {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        assert_eq!(pixels.len(), width as usize * height as usize, "Pixelanzahl passt nicht");
        Self { width, height, pixels }
    }

    pub fn get(&self, x: u32, y: u32) -> u8 {
        self.pixels[y as usize * self.width as usize + x as usize]
    }

    pub fn to_png(&self) -> Result<Vec<u8>, String> {
        use image::ImageEncoder;
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(&self.pixels, self.width, self.height, image::ExtendedColorType::L8)
            .map_err(|e| e.to_string())?;
        Ok(out)
    }

    pub fn from_encoded(bytes: &[u8]) -> Result<Self, String> {
        let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
        Ok(Self::from_dynamic(&img))
    }

    /// Wandelt ein beliebiges Bild in Graustufen um und legt Transparenz auf weißen Hintergrund.
    pub fn from_dynamic(img: &image::DynamicImage) -> Self {
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        let pixels = rgba
            .pixels()
            .map(|p| {
                let [r, g, b, a] = p.0;
                let lum = 0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64;
                let alpha = a as f64 / 255.0;
                (lum * alpha + 255.0 * (1.0 - alpha)).round().clamp(0.0, 255.0) as u8
            })
            .collect();
        Self::new(w, h, pixels)
    }
}

#[derive(Serialize, Deserialize)]
struct EncodedImage {
    width: u32,
    height: u32,
    png: String,
}

impl Serialize for RasterImage {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let png = self.to_png().map_err(serde::ser::Error::custom)?;
        EncodedImage {
            width: self.width,
            height: self.height,
            png: base64::engine::general_purpose::STANDARD.encode(png),
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for RasterImage {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let enc = EncodedImage::deserialize(d)?;
        let bytes =
            base64::engine::general_purpose::STANDARD.decode(enc.png.as_bytes()).map_err(serde::de::Error::custom)?;
        RasterImage::from_encoded(&bytes).map_err(serde::de::Error::custom)
    }
}

/// Bildanpassungen, die vor dem Dithering angewendet werden: zuerst Entrauschen und Schärfen, dann die Tonwerte.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageAdjust {
    /// -100 … 100
    pub brightness: f64,
    /// -100 … 100
    pub contrast: f64,
    /// 0.2 … 5.0, 1.0 = unverändert
    pub gamma: f64,
    /// Stärke der Unscharfmaskierung in % (0 = aus … 300)
    pub sharpen: f64,
    /// Radius der Unscharfmaskierung in mm auf dem Werkstück
    pub sharpen_radius: f64,
    /// Radius des Medianfilters in mm auf dem Werkstück (0 = aus): entfernt Rauschen und Störpixel, Kanten
    /// bleiben scharf
    pub denoise: f64,
}

impl Default for ImageAdjust {
    fn default() -> Self {
        Self { brightness: 0.0, contrast: 0.0, gamma: 1.0, sharpen: 0.0, sharpen_radius: 0.3, denoise: 0.0 }
    }
}

/// Größter Medianradius in Rasterzellen (begrenzt die Rechenzeit bei feinen Rastern).
const MAX_DENOISE_CELLS: usize = 25;

impl ImageAdjust {
    pub fn is_identity(&self) -> bool {
        self.brightness == 0.0 && self.contrast == 0.0 && (self.gamma - 1.0).abs() < 1e-9 && !self.filters()
    }

    pub fn sharpens(&self) -> bool {
        self.sharpen > 0.0
    }

    pub fn denoises(&self) -> bool {
        self.denoise > 0.0
    }

    /// Wirkt ein Nachbarschaftsfilter (Entrauschen oder Schärfen)? Dann hängt das Ergebnis von der Rastergröße ab.
    pub fn filters(&self) -> bool {
        self.sharpens() || self.denoises()
    }

    /// Entrauschen, dann Schärfen eines Grauwertrasters (siehe [`Self::denoise_grid`], [`Self::sharpen_grid`]).
    pub fn filter_grid(&self, gray: &mut [f32], mask: Option<&[bool]>, cols: usize, rows: usize, cell_mm: f64) {
        self.denoise_grid(gray, mask, cols, rows, cell_mm);
        self.sharpen_grid(gray, mask, cols, rows, cell_mm);
    }

    /// Medianfilter mit quadratischem Fenster (Radius `denoise` in mm) über ein Grauwertraster (0 … 1). Gleitendes
    /// Histogramm auf 256 Stufen (Huang): je Zelle nur eine Spalte hinein und eine hinaus. Zellen außerhalb von
    /// `mask` bleiben unverändert und zählen nicht mit.
    pub fn denoise_grid(&self, gray: &mut [f32], mask: Option<&[bool]>, cols: usize, rows: usize, cell_mm: f64) {
        let n = cols * rows;
        if !self.denoises() || n == 0 || gray.len() < n || mask.is_some_and(|m| m.len() < n) {
            return;
        }
        let r = ((self.denoise / cell_mm.max(1e-9)).round() as usize).clamp(1, MAX_DENOISE_CELLS);
        let inside = |i: usize| mask.is_none_or(|m| m[i]);
        let q: Vec<u8> = gray[..n].iter().map(|&g| (g.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
        for y in 0..rows {
            let (y0, y1) = (y.saturating_sub(r), (y + r).min(rows - 1));
            let column = |c: usize| (y0..=y1).map(move |yy| yy * cols + c).filter(move |&i| inside(i));
            let mut hist = [0u32; 256];
            // `m`: aktueller Median, `below`: Anzahl der Werte kleiner `m`
            let (mut count, mut m, mut below) = (0u32, 0usize, 0u32);
            for x in 0..cols {
                let entering = if x == 0 { 0..(r + 1).min(cols) } else { (x + r).min(cols)..(x + r + 1).min(cols) };
                for i in entering.flat_map(column) {
                    hist[q[i] as usize] += 1;
                    count += 1;
                    below += u32::from((q[i] as usize) < m);
                }
                if x > r {
                    for i in column(x - r - 1) {
                        hist[q[i] as usize] -= 1;
                        count -= 1;
                        below -= u32::from((q[i] as usize) < m);
                    }
                }
                let i = y * cols + x;
                if count == 0 || !inside(i) {
                    continue;
                }
                let target = (count - 1) / 2;
                while below > target {
                    m -= 1;
                    below -= hist[m];
                }
                while below + hist[m] <= target {
                    below += hist[m];
                    m += 1;
                }
                gray[i] = m as f32 / 255.0;
            }
        }
    }

    /// Unscharfmaskierung `v + Stärke · (v − Weichzeichnung)` eines Grauwertrasters (0 … 1) mit Zellgröße
    /// `cell_mm`. Die Weichzeichnung ist ein dreifacher Kastenfilter (≈ Gauß mit σ = Radius). Zellen außerhalb
    /// von `mask` bleiben unverändert und zählen nicht zur Umgebung, damit an Bildrändern keine Säume entstehen.
    pub fn sharpen_grid(&self, gray: &mut [f32], mask: Option<&[bool]>, cols: usize, rows: usize, cell_mm: f64) {
        let n = cols * rows;
        if !self.sharpens() || n == 0 || gray.len() < n || mask.is_some_and(|m| m.len() < n) {
            return;
        }
        let sigma = self.sharpen_radius.max(0.0) / cell_mm.max(1e-9);
        // drei Kästen der Breite 2r+1 haben zusammen die Varianz (w² − 1) / 4
        let r = ((((4.0 * sigma * sigma + 1.0).sqrt() - 1.0) / 2.0).round() as usize).clamp(1, cols.max(rows));
        let inside = |i: usize| mask.is_none_or(|m| m[i]);
        let mut scratch = Vec::new();
        let mut blur = |a: &mut Vec<f32>| {
            for _ in 0..3 {
                box_rows(a, cols, r);
                box_columns(a, cols, rows, r, &mut scratch);
            }
        };
        let mut num: Vec<f32> = (0..n).map(|i| if inside(i) { gray[i] } else { 0.0 }).collect();
        blur(&mut num);
        // Anteil der Umgebung, der im Raster bzw. in der Maske liegt; ohne Maske zerfällt er in Zeile × Spalte
        let den = mask.map(|m| {
            let mut d: Vec<f32> = m[..n].iter().map(|&b| if b { 1.0 } else { 0.0 }).collect();
            blur(&mut d);
            d
        });
        let (mut den_x, mut den_y) = (vec![1f32; cols], vec![1f32; rows]);
        for _ in 0..3 {
            box_rows(&mut den_x, cols, r);
            box_rows(&mut den_y, rows, r);
        }
        let amount = (self.sharpen / 100.0) as f32;
        for i in 0..n {
            let d = den.as_ref().map_or_else(|| den_x[i % cols] * den_y[i / cols], |d| d[i]);
            if inside(i) && d > 1e-6 {
                let v = gray[i];
                gray[i] = (v + amount * (v - num[i] / d)).clamp(0.0, 1.0);
            }
        }
    }

    /// Entrauscht und schärft 8-Bit-Grauwerte (Pixelgröße `px_mm`); die Tonwerte bleiben unberührt.
    pub fn filter_pixels(&self, pixels: &mut [u8], cols: usize, rows: usize, px_mm: f64) {
        if !self.filters() {
            return;
        }
        let mut gray: Vec<f32> = pixels.iter().map(|&v| v as f32 / 255.0).collect();
        self.filter_grid(&mut gray, None, cols, rows, px_mm);
        for (p, g) in pixels.iter_mut().zip(gray) {
            *p = (g * 255.0).round() as u8;
        }
    }

    /// Bild mit allen Anpassungen (Filter und Tonwerte), `px_mm` = Pixelgröße auf dem Werkstück.
    pub fn adjusted_image(&self, img: &RasterImage, px_mm: f64) -> RasterImage {
        let mut pixels = img.pixels.clone();
        self.filter_pixels(&mut pixels, img.width as usize, img.height as usize, px_mm);
        let lut: Vec<u8> = (0..=255u8).map(|v| (self.apply(v as f32 / 255.0) * 255.0).round() as u8).collect();
        for p in &mut pixels {
            *p = lut[*p as usize];
        }
        RasterImage::new(img.width, img.height, pixels)
    }

    /// Wendet die Anpassung auf einen Grauwert 0…1 an.
    pub fn apply(&self, v: f32) -> f32 {
        let mut v = v as f64;
        if (self.gamma - 1.0).abs() > 1e-9 {
            v = v.clamp(0.0, 1.0).powf(1.0 / self.gamma.max(0.01));
        }
        v += self.brightness / 100.0;
        let c = (self.contrast / 100.0).clamp(-0.99, 0.99);
        let factor = if c >= 0.0 { 1.0 / (1.0 - c) } else { 1.0 + c };
        v = (v - 0.5) * factor + 0.5;
        v.clamp(0.0, 1.0) as f32
    }
}

/// Gleitender Mittelwert der Breite 2r+1 entlang jeder Zeile; außerhalb des Rasters zählt 0.
fn box_rows(a: &mut [f32], cols: usize, r: usize) {
    let norm = 1.0 / (2 * r + 1) as f32;
    let mut line = vec![0f32; cols];
    for row in a.chunks_exact_mut(cols) {
        line.copy_from_slice(row);
        let mut sum: f32 = line[..(r + 1).min(cols)].iter().sum();
        for k in 0..cols {
            row[k] = sum * norm;
            if k + r + 1 < cols {
                sum += line[k + r + 1];
            }
            if k >= r {
                sum -= line[k - r];
            }
        }
    }
}

/// Wie [`box_rows`] entlang der Spalten, zeilenweise gerechnet (speicherfreundlich).
fn box_columns(a: &mut [f32], cols: usize, rows: usize, r: usize, src: &mut Vec<f32>) {
    let norm = 1.0 / (2 * r + 1) as f32;
    src.clear();
    src.extend_from_slice(&a[..cols * rows]);
    let mut sum = vec![0f32; cols];
    for row in src.chunks_exact(cols).take(r + 1) {
        sum.iter_mut().zip(row).for_each(|(s, v)| *s += v);
    }
    for k in 0..rows {
        a[k * cols..(k + 1) * cols].iter_mut().zip(&sum).for_each(|(o, s)| *o = s * norm);
        if k + r + 1 < rows {
            sum.iter_mut().zip(&src[(k + r + 1) * cols..(k + r + 2) * cols]).for_each(|(s, v)| *s += v);
        }
        if k >= r {
            sum.iter_mut().zip(&src[(k - r) * cols..(k - r + 1) * cols]).for_each(|(s, v)| *s -= v);
        }
    }
}

/// Verschieben in der Objektreihenfolge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZOrder {
    /// ganz nach vorne (zuletzt)
    Front,
    /// eine Stelle nach vorne
    Forward,
    /// eine Stelle nach hinten
    Backward,
    /// ganz nach hinten (zuerst)
    Back,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageObject {
    pub image: Arc<RasterImage>,
    /// Größe ohne Transformation in mm; lokales Koordinatensystem (0,0)…(width_mm,height_mm).
    pub width_mm: f64,
    pub height_mm: f64,
    #[serde(default)]
    pub adjust: ImageAdjust,
    /// Zuschnitt: geschlossene Umrisse im lokalen Koordinatensystem; nur Pixel darin werden angezeigt und
    /// graviert (Gerade-Ungerade-Regel). Leer = ganzes Bild.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clip: Vec<Vec<Point>>,
}

impl ImageObject {
    pub fn local_rect(&self) -> Rect {
        Rect::from_xywh(0.0, 0.0, self.width_mm, self.height_mm)
    }

    /// Zuschnitt als Raster aus `cols`×`rows` Zellen über die ganze Bildfläche (`true` = Zellmitte liegt im
    /// Zuschnitt); `None`, wenn das Bild nicht zugeschnitten ist.
    pub fn clip_mask(&self, cols: usize, rows: usize) -> Option<Vec<bool>> {
        if self.clip.is_empty() || cols == 0 || rows == 0 {
            return None;
        }
        let (sx, sy) = (self.width_mm / cols as f64, self.height_mm / rows as f64);
        let mut mask = vec![false; cols * rows];
        let mut xs: Vec<f64> = Vec::new();
        for r in 0..rows {
            let y = (r as f64 + 0.5) * sy;
            xs.clear();
            for ring in &self.clip {
                for (i, &a) in ring.iter().enumerate() {
                    let b = ring[(i + 1) % ring.len()];
                    if (a.y <= y) != (b.y <= y) {
                        xs.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
                    }
                }
            }
            xs.sort_by(f64::total_cmp);
            for span in xs.as_chunks::<2>().0 {
                // Zellen, deren Mitte zwischen den Schnittpunkten liegt
                let first = (span[0] / sx - 0.5).ceil().max(0.0) as usize;
                let last = (span[1] / sx - 0.5).floor();
                if last < 0.0 {
                    continue;
                }
                for c in first..=(last as usize).min(cols - 1) {
                    mask[r * cols + c] = true;
                }
            }
        }
        Some(mask)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    Path(VectorPath),
    Image(ImageObject),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Object {
    pub id: ObjectId,
    pub name: String,
    pub layer: usize,
    #[serde(default)]
    pub group: Option<GroupId>,
    pub transform: Affine,
    pub shape: Shape,
    /// Bei Texten: Daten zum erneuten Erzeugen (Bearbeiten, variabler Text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<Box<crate::text::TextData>>,
    /// Leistung dieses Objekts in Prozent der Ebenenleistung (z. B. für Materialtests).
    #[serde(default = "full_power", skip_serializing_if = "is_full_power")]
    pub power_scale: f64,
    /// Parameter einer Grundform (Eckenradius, Ecken, Zacken); gilt nur, solange der Pfad daraus besteht.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primitive: Option<crate::primitive::Primitive>,
    /// Von Hand gesetzte Laschen (Punkte auf der Kontur, Objektkoordinaten); an ihrer Kontur ersetzen sie die
    /// automatisch verteilten.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tabs: Vec<Point>,
}

fn full_power() -> f64 {
    100.0
}

fn is_full_power(v: &f64) -> bool {
    *v == 100.0
}

impl Object {
    /// Nachdem die Transformation in die Form eingerechnet wurde: Transformation zurücksetzen, Laschen wandern mit.
    pub fn reset_transform(&mut self) {
        for t in &mut self.tabs {
            *t = self.transform.apply(*t);
        }
        self.transform = Affine::IDENTITY;
    }

    pub fn is_image(&self) -> bool {
        matches!(self.shape, Shape::Image(_))
    }

    /// Umrandung im Dokument-Koordinatensystem.
    pub fn bounds(&self) -> Rect {
        match &self.shape {
            Shape::Path(p) => p.bounds(&self.transform),
            Shape::Image(img) => self.transform.transform_rect(&img.local_rect()),
        }
    }

    /// Abgeflachte Geometrie im Dokument-Koordinatensystem (Bilder: Umrissrechteck).
    pub fn polylines(&self, tol: f64) -> Vec<Polyline> {
        match &self.shape {
            Shape::Path(p) => p.flatten(&self.transform, tol),
            Shape::Image(img) => {
                let pts = img.local_rect().corners().map(|c| self.transform.apply(c)).to_vec();
                vec![Polyline::new(pts, true)]
            }
        }
    }
}

/// Notizen zum Projekt (Material, Einstellungen, Hinweise), wahlweise beim Öffnen angezeigt.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectNotes {
    pub text: String,
    #[serde(default)]
    pub show_on_open: bool,
}

impl ProjectNotes {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub objects: Vec<Object>,
    pub layers: Vec<Layer>,
    /// Ausgabereihenfolge der Ebenen (Permutation von 0..LAYER_COUNT).
    pub layer_order: Vec<usize>,
    next_id: u64,
    #[serde(default, skip_serializing_if = "ProjectNotes::is_empty")]
    pub notes: ProjectNotes,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    pub fn new() -> Self {
        Self {
            objects: Vec::new(),
            layers: (0..LAYER_COUNT).map(Layer::new).collect(),
            layer_order: (0..LAYER_COUNT).collect(),
            next_id: 1,
            notes: ProjectNotes::default(),
        }
    }

    /// Stellt nach dem Laden sicher, dass Ebenen und Reihenfolge vollständig sind.
    pub fn repair(&mut self) {
        while self.layers.len() < LAYER_COUNT {
            self.layers.push(Layer::new(self.layers.len()));
        }
        for (i, l) in self.layers.iter_mut().enumerate() {
            l.index = i;
        }
        let mut seen = [false; LAYER_COUNT];
        self.layer_order.retain(|&i| i < LAYER_COUNT && !std::mem::replace(&mut seen[i], true));
        for (i, s) in seen.iter().enumerate() {
            if !s {
                self.layer_order.push(i);
            }
        }
        for o in &mut self.objects {
            o.layer = o.layer.min(LAYER_COUNT - 1);
        }
        let max_id = self.objects.iter().map(|o| o.id).max().unwrap_or(0);
        let max_group = self.objects.iter().filter_map(|o| o.group).max().unwrap_or(0);
        self.next_id = self.next_id.max(max_id.max(max_group) + 1);
    }

    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn add(&mut self, name: impl Into<String>, layer: usize, transform: Affine, shape: Shape) -> ObjectId {
        let id = self.alloc_id();
        self.objects.push(Object {
            id,
            name: name.into(),
            layer: layer.min(LAYER_COUNT - 1),
            group: None,
            transform,
            shape,
            text: None,
            power_scale: 100.0,
            primitive: None,
            tabs: Vec::new(),
        });
        id
    }

    pub fn add_path(&mut self, name: impl Into<String>, layer: usize, path: VectorPath) -> ObjectId {
        self.add(name, layer, Affine::IDENTITY, Shape::Path(path))
    }

    /// Grundform mit nachträglich änderbaren Parametern.
    pub fn add_primitive(&mut self, name: impl Into<String>, layer: usize, p: crate::primitive::Primitive) -> ObjectId {
        let id = self.add_path(name, layer, p.path());
        if let Some(o) = self.get_mut(id) {
            o.primitive = Some(p);
        }
        id
    }

    /// Fügt ein Bild ein. Bildebenen werden automatisch auf den Modus „Bild“ gestellt, wenn sie noch leer sind.
    pub fn add_image(&mut self, name: impl Into<String>, layer: usize, image: ImageObject, at: Point) -> ObjectId {
        if self.objects_on_layer(layer).next().is_none() {
            self.layers[layer].settings = crate::layer::CutSettings::default_for(LayerMode::Image);
        }
        self.add(name, layer, Affine::translate(at.x, at.y), Shape::Image(image))
    }

    pub fn get(&self, id: ObjectId) -> Option<&Object> {
        self.objects.iter().find(|o| o.id == id)
    }

    pub fn get_mut(&mut self, id: ObjectId) -> Option<&mut Object> {
        self.objects.iter_mut().find(|o| o.id == id)
    }

    pub fn remove(&mut self, ids: &[ObjectId]) {
        let ids = id_set(ids);
        self.objects.retain(|o| !ids.contains(&o.id));
    }

    pub fn objects_on_layer(&self, layer: usize) -> impl Iterator<Item = &Object> {
        self.objects.iter().filter(move |o| o.layer == layer)
    }

    /// Benutzte Ebenen in Ausgabereihenfolge.
    pub fn used_layers(&self) -> Vec<usize> {
        let mut used = [false; LAYER_COUNT];
        for o in &self.objects {
            used[o.layer] = true;
        }
        self.layer_order.iter().copied().filter(|&l| used[l]).collect()
    }

    /// Die Objekte mit den IDs `ids` in Dokumentreihenfolge – auch bei Tausenden IDs schnell (statt `get` je ID).
    pub fn objects_in<'a>(&'a self, ids: &[ObjectId]) -> impl Iterator<Item = &'a Object> + use<'a> {
        let ids = id_set(ids);
        self.objects.iter().filter(move |o| ids.contains(&o.id))
    }

    pub fn objects_in_mut<'a>(&'a mut self, ids: &[ObjectId]) -> impl Iterator<Item = &'a mut Object> + use<'a> {
        let ids = id_set(ids);
        self.objects.iter_mut().filter(move |o| ids.contains(&o.id))
    }

    pub fn bounds_of(&self, ids: &[ObjectId]) -> Rect {
        let ids = id_set(ids);
        self.objects.iter().filter(|o| ids.contains(&o.id)).fold(Rect::EMPTY, |r, o| r.union(&o.bounds()))
    }

    pub fn bounds(&self) -> Rect {
        self.objects.iter().fold(Rect::EMPTY, |r, o| r.union(&o.bounds()))
    }

    pub fn transform_objects(&mut self, ids: &[ObjectId], t: &Affine) {
        let ids = id_set(ids);
        for o in self.objects.iter_mut().filter(|o| ids.contains(&o.id)) {
            o.transform = o.transform.then(t);
        }
    }

    /// Alle Objekte derselben Gruppen wie `ids` (für die Auswahl ganzer Gruppen).
    pub fn expand_groups(&self, ids: &[ObjectId]) -> Vec<ObjectId> {
        let ids = id_set(ids);
        let groups: HashSet<GroupId> =
            self.objects.iter().filter(|o| ids.contains(&o.id)).filter_map(|o| o.group).collect();
        self.objects
            .iter()
            .filter(|o| ids.contains(&o.id) || o.group.is_some_and(|g| groups.contains(&g)))
            .map(|o| o.id)
            .collect()
    }

    pub fn group(&mut self, ids: &[ObjectId]) -> Option<GroupId> {
        if ids.len() < 2 {
            return None;
        }
        let gid = self.alloc_id();
        let ids = id_set(ids);
        for o in self.objects.iter_mut().filter(|o| ids.contains(&o.id)) {
            o.group = Some(gid);
        }
        Some(gid)
    }

    pub fn ungroup(&mut self, ids: &[ObjectId]) {
        let ids = id_set(ids);
        for o in self.objects.iter_mut().filter(|o| ids.contains(&o.id)) {
            o.group = None;
        }
    }

    /// Automatisch gruppieren: Jede äußerste Form bildet mit allem, was in einer ihrer geschlossenen Konturen liegt,
    /// eine Gruppe (z. B. ein Teil mit seinen Löchern nach dem DXF-Import). Bestehende Gruppen zählen als Einheit;
    /// Formen ohne Inhalt und ohne Umgebung bleiben, wie sie sind. Liefert die Zahl der gebildeten Gruppen.
    pub fn auto_group(&mut self, ids: &[ObjectId]) -> usize {
        // Einheiten: einzelne Objekte oder ganze bestehende Gruppen
        let mut units: Vec<Vec<ObjectId>> = Vec::new();
        let mut by_group: Vec<(GroupId, usize)> = Vec::new();
        for id in self.expand_groups(ids) {
            let Some(o) = self.get(id) else { continue };
            match o.group.and_then(|g| by_group.iter().find(|(h, _)| *h == g).map(|(_, i)| *i)) {
                Some(i) => units[i].push(id),
                None => {
                    if let Some(g) = o.group {
                        by_group.push((g, units.len()));
                    }
                    units.push(vec![id]);
                }
            }
        }
        let polys: Vec<Vec<Polyline>> = units
            .iter()
            .map(|u| u.iter().filter_map(|id| self.get(*id)).flat_map(|o| o.polylines(0.05)).collect())
            .collect();
        let bounds: Vec<Rect> = polys.iter().map(|p| crate::path::bounds_of(p)).collect();
        let area = |r: &Rect| r.width() * r.height();
        let contains = |a: usize, b: usize| {
            let (ra, rb) = (&bounds[a], &bounds[b]);
            let Some(probe) = polys[b].first().and_then(|p| p.points.first()) else { return false };
            a != b
                && ra.contains_rect(rb)
                && (area(ra) > area(rb) || (area(ra) == area(rb) && a < b))
                && polys[a].iter().any(|p| crate::path::point_in_polylines(*probe, std::slice::from_ref(p)))
        };
        // Jede Einheit gehört zur größten Einheit, die sie umschließt (das ist zugleich die äußerste).
        let n = units.len();
        let mut members: Vec<Vec<usize>> = vec![Vec::new(); n];
        for b in 0..n {
            let outer =
                (0..n).filter(|&a| contains(a, b)).max_by(|&x, &y| area(&bounds[x]).total_cmp(&area(&bounds[y])));
            if let Some(a) = outer {
                members[a].push(b);
            }
        }
        let mut count = 0;
        for (a, inner) in members.iter().enumerate().filter(|(_, m)| !m.is_empty()) {
            let ids: Vec<ObjectId> =
                std::iter::once(a).chain(inner.iter().copied()).flat_map(|u| units[u].clone()).collect();
            self.group(&ids);
            count += 1;
        }
        count
    }

    /// Verschiebt eine Ebene in der Ausgabereihenfolge um `delta` Plätze innerhalb der benutzten Ebenen.
    pub fn move_layer(&mut self, layer: usize, delta: i32) {
        let used = self.used_layers();
        let Some(pos) = used.iter().position(|&l| l == layer) else { return };
        let target = pos as i32 + delta;
        if target < 0 || target >= used.len() as i32 {
            return;
        }
        let other = used[target as usize];
        let a = self.layer_order.iter().position(|&l| l == layer).unwrap();
        let b = self.layer_order.iter().position(|&l| l == other).unwrap();
        self.layer_order.swap(a, b);
    }

    /// Kopiert Objekte mit neuen IDs (Gruppen bleiben als neue Gruppen erhalten) und gibt die neuen IDs zurück.
    /// Ändert die Reihenfolge der Objekte (weiter hinten in der Liste = oben gezeichnet und ohne Optimierung später
    /// bearbeitet); die gewählten behalten untereinander ihre Reihenfolge. `true`, wenn sich etwas geändert hat.
    pub fn reorder(&mut self, ids: &[ObjectId], how: ZOrder) -> bool {
        let ids = id_set(ids);
        let sel = |o: &Object| ids.contains(&o.id);
        let before: Vec<ObjectId> = self.objects.iter().map(|o| o.id).collect();
        let n = self.objects.len();
        match how {
            ZOrder::Front | ZOrder::Back => {
                let (picked, rest): (Vec<Object>, Vec<Object>) =
                    std::mem::take(&mut self.objects).into_iter().partition(|o| sel(o));
                let (first, second) = if how == ZOrder::Front { (rest, picked) } else { (picked, rest) };
                self.objects = first.into_iter().chain(second).collect();
            }
            ZOrder::Forward => {
                for i in (0..n.saturating_sub(1)).rev() {
                    if sel(&self.objects[i]) && !sel(&self.objects[i + 1]) {
                        self.objects.swap(i, i + 1);
                    }
                }
            }
            ZOrder::Backward => {
                for i in 1..n {
                    if sel(&self.objects[i]) && !sel(&self.objects[i - 1]) {
                        self.objects.swap(i, i - 1);
                    }
                }
            }
        }
        self.objects.iter().map(|o| o.id).ne(before)
    }

    pub fn duplicate(&mut self, objects: &[Object], offset: Point) -> Vec<ObjectId> {
        let mut group_map: Vec<(GroupId, GroupId)> = Vec::new();
        let mut ids = Vec::new();
        for o in objects {
            let mut copy = o.clone();
            copy.id = self.alloc_id();
            if let Some(g) = o.group {
                let new_g = match group_map.iter().find(|(old, _)| *old == g) {
                    Some((_, n)) => *n,
                    None => {
                        let n = self.alloc_id();
                        group_map.push((g, n));
                        n
                    }
                };
                copy.group = Some(new_g);
            }
            copy.transform = copy.transform.then(&Affine::translate(offset.x, offset.y));
            ids.push(copy.id);
            self.objects.push(copy);
        }
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_group_collects_parts_with_their_holes() {
        let mut doc = Document::new();
        let part1 = doc.add_path("Teil", 0, VectorPath::rect(0.0, 0.0, 50.0, 30.0, 0.0));
        let hole1 = doc.add_path("Loch", 0, VectorPath::rect(10.0, 10.0, 5.0, 5.0, 0.0));
        let hole2 = doc.add_path("Loch", 0, VectorPath::ellipse(Point::new(35.0, 15.0), 4.0, 4.0));
        // Punkt im Loch gehört trotzdem zum Teil
        let dot = doc.add_path("Punkt", 0, VectorPath::ellipse(Point::new(12.5, 12.5), 1.0, 1.0));
        let part2 = doc.add_path("Teil", 0, VectorPath::rect(60.0, 0.0, 20.0, 20.0, 0.0));
        let hole3 = doc.add_path("Loch", 0, VectorPath::rect(65.0, 5.0, 5.0, 5.0, 0.0));
        let lone = doc.add_path("Allein", 0, VectorPath::rect(100.0, 0.0, 10.0, 10.0, 0.0));
        // eine bestehende Gruppe zählt als Einheit und landet als Ganzes in Teil 2
        let a = doc.add_path("A", 0, VectorPath::ellipse(Point::new(75.0, 15.0), 1.0, 1.0));
        let b = doc.add_path("B", 0, VectorPath::ellipse(Point::new(75.0, 5.0), 1.0, 1.0));
        doc.group(&[a, b]);
        let all = [part1, hole1, hole2, dot, part2, hole3, lone, a, b];
        assert_eq!(doc.auto_group(&all), 2);
        let g = |id| doc.get(id).unwrap().group;
        assert!(g(part1).is_some() && [hole1, hole2, dot].iter().all(|&i| g(i) == g(part1)));
        assert!(g(part2).is_some() && [hole3, a, b].iter().all(|&i| g(i) == g(part2)));
        assert_ne!(g(part1), g(part2));
        assert_eq!(g(lone), None);
        // nochmal: nichts Neues (jede Gruppe ist eine Einheit ohne Umgebung)
        assert_eq!(doc.auto_group(&all), 0);
    }

    #[test]
    fn image_clip_mask() {
        let mut img = ImageObject {
            image: std::sync::Arc::new(RasterImage::new(1, 1, vec![0])),
            width_mm: 10.0,
            height_mm: 10.0,
            adjust: ImageAdjust::default(),
            clip: Vec::new(),
        };
        assert!(img.clip_mask(10, 10).is_none());
        // Rahmen mit Loch: äußeres Quadrat 1…9, inneres 4…6
        let sq = |a: f64, b: f64| vec![Point::new(a, a), Point::new(b, a), Point::new(b, b), Point::new(a, b)];
        img.clip = vec![sq(1.0, 9.0), sq(4.0, 6.0)];
        let m = img.clip_mask(10, 10).unwrap();
        let at = |c: usize, r: usize| m[r * 10 + c];
        assert!(!at(0, 0) && at(1, 1) && at(8, 8) && !at(9, 9));
        assert!(!at(4, 4) && !at(5, 5) && at(3, 5));
        assert_eq!(m.iter().filter(|v| **v).count(), 64 - 4);
    }

    #[test]
    fn layer_order_moves_within_used_layers() {
        let mut doc = Document::new();
        doc.add_path("a", 0, VectorPath::rect(0.0, 0.0, 1.0, 1.0, 0.0));
        doc.add_path("b", 5, VectorPath::rect(0.0, 0.0, 1.0, 1.0, 0.0));
        assert_eq!(doc.used_layers(), vec![0, 5]);
        doc.move_layer(5, -1);
        assert_eq!(doc.used_layers(), vec![5, 0]);
        doc.move_layer(5, -1);
        assert_eq!(doc.used_layers(), vec![5, 0]);
    }

    #[test]
    fn duplicate_keeps_groups_separate() {
        let mut doc = Document::new();
        let a = doc.add_path("a", 0, VectorPath::rect(0.0, 0.0, 1.0, 1.0, 0.0));
        let b = doc.add_path("b", 0, VectorPath::rect(2.0, 0.0, 1.0, 1.0, 0.0));
        let g = doc.group(&[a, b]).unwrap();
        let originals: Vec<Object> = doc.objects.clone();
        let new_ids = doc.duplicate(&originals, Point::new(5.0, 0.0));
        let new_group = doc.get(new_ids[0]).unwrap().group.unwrap();
        assert_ne!(new_group, g);
        assert_eq!(doc.get(new_ids[1]).unwrap().group, Some(new_group));
        assert_eq!(doc.expand_groups(&[a]).len(), 2);
    }

    #[test]
    fn image_adjust_identity() {
        let a = ImageAdjust::default();
        for v in [0.0f32, 0.25, 0.5, 1.0] {
            assert!((a.apply(v) - v).abs() < 1e-6);
        }
    }

    #[test]
    fn sharpen_raises_edge_contrast_and_respects_mask() {
        // Kante 0,2 | 0,8 in einem 40 × 4 Raster, Zellen 0,1 mm
        let (cols, rows) = (40, 4);
        let base: Vec<f32> = (0..cols * rows).map(|i| if i % cols < 20 { 0.2 } else { 0.8 }).collect();
        let mut g = base.clone();
        ImageAdjust::default().sharpen_grid(&mut g, None, cols, rows, 0.1);
        assert_eq!(g, base, "Stärke 0 ändert nichts");
        let a = ImageAdjust { sharpen: 100.0, sharpen_radius: 0.2, ..Default::default() };
        assert!(!a.is_identity());
        a.sharpen_grid(&mut g, None, cols, rows, 0.1);
        assert!(g[19] < 0.15 && g[20] > 0.85, "Kante verstärkt: {} {}", g[19], g[20]);
        assert!((g[0] - 0.2).abs() < 1e-4 && (g[39] - 0.8).abs() < 1e-4, "Flächen bleiben");

        // Gleichmäßige Fläche neben ausgeblendeten (weißen) Zellen: kein dunkler Saum
        let mask: Vec<bool> = (0..cols * rows).map(|i| i % cols >= 10).collect();
        let mut g: Vec<f32> = mask.iter().map(|&m| if m { 0.5 } else { 1.0 }).collect();
        a.sharpen_grid(&mut g, Some(&mask), cols, rows, 0.1);
        assert!(g.iter().zip(&mask).all(|(v, m)| if *m { (v - 0.5).abs() < 1e-4 } else { *v == 1.0 }));
    }

    #[test]
    fn denoise_removes_specks_and_keeps_edges() {
        // Kante 0,2 | 0,8 mit einzelnen Störpixeln, Zellen 0,1 mm, Radius 0,1 mm = 3 × 3 Fenster
        let (cols, rows) = (30, 10);
        let base: Vec<f32> = (0..cols * rows).map(|i| if i % cols < 15 { 0.2 } else { 0.8 }).collect();
        let mut g = base.clone();
        for &i in &[3 * cols + 5, 6 * cols + 22, 0, cols * rows - 1] {
            g[i] = if base[i] < 0.5 { 1.0 } else { 0.0 };
        }
        let a = ImageAdjust { denoise: 0.1, ..Default::default() };
        assert!(!a.is_identity() && a.filters());
        a.denoise_grid(&mut g, None, cols, rows, 0.1);
        for (i, (v, b)) in g.iter().zip(&base).enumerate() {
            assert!((v - b).abs() < 0.003, "Zelle {i}: {v} statt {b}");
        }

        // Ausgeblendete Zellen bleiben, und ihre Werte ziehen den Median nicht
        let mask: Vec<bool> = (0..cols * rows).map(|i| i % cols >= 10).collect();
        let mut g: Vec<f32> = mask.iter().map(|&m| if m { 0.5 } else { 0.0 }).collect();
        a.denoise_grid(&mut g, Some(&mask), cols, rows, 0.1);
        assert!(g.iter().zip(&mask).all(|(v, m)| if *m { (v - 0.5).abs() < 0.003 } else { *v == 0.0 }));
    }

    #[test]
    fn reorder_moves_selection_as_block() {
        let mut doc = Document::new();
        let ids: Vec<ObjectId> =
            (0..5).map(|k| doc.add_path(format!("{k}"), 0, VectorPath::rect(0.0, 0.0, 1.0, 1.0, 0.0))).collect();
        let order = |d: &Document| d.objects.iter().map(|o| o.name.clone()).collect::<Vec<_>>().join("");
        assert!(doc.reorder(&[ids[1], ids[2]], ZOrder::Forward));
        assert_eq!(order(&doc), "03124");
        assert!(doc.reorder(&[ids[1], ids[2]], ZOrder::Front));
        assert_eq!(order(&doc), "03412");
        assert!(!doc.reorder(&[ids[1], ids[2]], ZOrder::Front), "schon vorne");
        assert!(doc.reorder(&[ids[4]], ZOrder::Back));
        assert_eq!(order(&doc), "40312");
        assert!(doc.reorder(&[ids[3]], ZOrder::Backward));
        assert_eq!(order(&doc), "43012");
    }
}
