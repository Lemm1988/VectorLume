//! Das Dokument: Objekte (Pfade und Bilder) auf Farbebenen.

use crate::geom::{Affine, Point, Rect};
use crate::layer::{LAYER_COUNT, Layer, LayerMode};
use crate::path::{Polyline, VectorPath};
use base64::Engine as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::Arc;

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

/// Bildanpassungen, die vor dem Dithering angewendet werden.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageAdjust {
    /// -100 … 100
    pub brightness: f64,
    /// -100 … 100
    pub contrast: f64,
    /// 0.2 … 5.0, 1.0 = unverändert
    pub gamma: f64,
}

impl Default for ImageAdjust {
    fn default() -> Self {
        Self { brightness: 0.0, contrast: 0.0, gamma: 1.0 }
    }
}

impl ImageAdjust {
    pub fn is_identity(&self) -> bool {
        self.brightness == 0.0 && self.contrast == 0.0 && (self.gamma - 1.0).abs() < 1e-9
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
}

fn full_power() -> f64 {
    100.0
}

fn is_full_power(v: &f64) -> bool {
    *v == 100.0
}

impl Object {
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub objects: Vec<Object>,
    pub layers: Vec<Layer>,
    /// Ausgabereihenfolge der Ebenen (Permutation von 0..LAYER_COUNT).
    pub layer_order: Vec<usize>,
    next_id: u64,
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
        });
        id
    }

    pub fn add_path(&mut self, name: impl Into<String>, layer: usize, path: VectorPath) -> ObjectId {
        self.add(name, layer, Affine::IDENTITY, Shape::Path(path))
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

    pub fn bounds_of(&self, ids: &[ObjectId]) -> Rect {
        self.objects.iter().filter(|o| ids.contains(&o.id)).fold(Rect::EMPTY, |r, o| r.union(&o.bounds()))
    }

    pub fn bounds(&self) -> Rect {
        self.objects.iter().fold(Rect::EMPTY, |r, o| r.union(&o.bounds()))
    }

    pub fn transform_objects(&mut self, ids: &[ObjectId], t: &Affine) {
        for o in self.objects.iter_mut().filter(|o| ids.contains(&o.id)) {
            o.transform = o.transform.then(t);
        }
    }

    /// Alle Objekte derselben Gruppen wie `ids` (für die Auswahl ganzer Gruppen).
    pub fn expand_groups(&self, ids: &[ObjectId]) -> Vec<ObjectId> {
        let groups: Vec<GroupId> =
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
        for o in self.objects.iter_mut().filter(|o| ids.contains(&o.id)) {
            o.group = Some(gid);
        }
        Some(gid)
    }

    pub fn ungroup(&mut self, ids: &[ObjectId]) {
        for o in self.objects.iter_mut().filter(|o| ids.contains(&o.id)) {
            o.group = None;
        }
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
}
