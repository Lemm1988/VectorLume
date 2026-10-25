//! LightBurn-Projekte (`.lbrn2`, XML) lesen, damit vorhandene Projekte weiterverwendet werden können:
//! Ebenen-Einstellungen, Rechtecke, Ellipsen, Pfade, Gruppen, Texte (über die mitgespeicherten Umrisse) und
//! eingebettete Bilder. Gelesen wird nur das Dateiformat der Nutzerdaten.
//!
//! LightBurn rechnet mit Y nach oben (Ursprung vorne links), VectorLume mit Y nach unten; umgerechnet wird über
//! die Höhe der Arbeitsfläche, damit die Teile an derselben Stelle auf dem Laserbett liegen.

use roxmltree::Node;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use vl_core::path::PathBuilder;
use vl_core::{
    Affine, CutSettings, DitherMode, Document, FillGrouping, ImageObject, LAYER_COUNT, LayerMode, ObjectId, Point,
    RasterImage, Shape, VectorPath,
};

/// Ergebnis: neues Dokument mit Ebenen-Einstellungen und Hinweise zu nicht übernommenen Teilen.
#[derive(Debug)]
pub struct LightBurnImport {
    pub doc: Document,
    pub warnings: Vec<String>,
}

/// Liest ein LightBurn-Projekt. `bed_height` ist die Höhe der Arbeitsfläche in mm (für die Y-Umrechnung),
/// `base_dir` der Ordner der Datei (für nicht eingebettete Bilder).
pub fn import_lightburn(data: &[u8], bed_height: f64, base_dir: Option<&Path>) -> Result<LightBurnImport, String> {
    let text = String::from_utf8_lossy(data);
    let xml = roxmltree::Document::parse(&text).map_err(|e| format!("Die Datei ist kein gültiges XML: {e}"))?;
    let root = xml.root_element();
    if root.tag_name().name() != "LightBurnProject" {
        return Err("Die Datei ist kein LightBurn-Projekt.".into());
    }
    if root.descendants().any(|n| n.has_tag_name("Shape") && n.children().any(|c| c.has_tag_name("Vert"))) {
        return Err(
            "Dieses ältere LightBurn-Format wird nicht unterstützt. Bitte in LightBurn als .lbrn2 speichern.".into()
        );
    }
    let mut im = Importer {
        doc: Document::new(),
        warnings: Vec::new(),
        flip: Affine([1.0, 0.0, 0.0, -1.0, 0.0, bed_height]),
        base_dir,
        verts: HashMap::new(),
        prims: HashMap::new(),
        images: HashMap::new(),
        used: root.descendants().filter_map(|n| n.attribute("CutIndex")).filter_map(|v| v.parse().ok()).collect(),
        layer_map: HashMap::new(),
        image_map: HashMap::new(),
        pending_image: HashMap::new(),
        entries: Vec::new(),
        skipped: HashMap::new(),
    };
    im.read_layers(root);
    for shape in root.children().filter(|n| n.has_tag_name("Shape")) {
        let mut ids = Vec::new();
        im.shape(shape, &Affine::IDENTITY, &mut ids);
        if shape.attribute("Type") == Some("Group") && ids.len() > 1 {
            im.doc.group(&ids);
        }
    }
    if im.doc.objects.is_empty() && im.skipped.is_empty() {
        return Err("Das Projekt enthält keine Objekte.".into());
    }
    // Ausgabereihenfolge nach LightBurn-Priorität, danach Nummer
    im.entries.sort_by_key(|e| (e.priority, e.index));
    let mut order: Vec<usize> = Vec::new();
    for l in im.entries.iter().map(|e| e.layer).chain(0..LAYER_COUNT) {
        if !order.contains(&l) {
            order.push(l);
        }
    }
    im.doc.layer_order = order;

    let mut warnings = im.warnings;
    let mut skipped: Vec<_> = im.skipped.into_iter().collect();
    skipped.sort();
    for (what, n) in skipped {
        warnings.push(format!("{n}× {what}"));
    }
    if root.attribute("MirrorX") == Some("True") || root.attribute("MirrorY") == Some("True") {
        warnings.push("Die Spiegelung der Ausgabe aus LightBurn wurde nicht übernommen.".into());
    }
    let mut doc = im.doc;
    if let Some(n) = root.children().find(|n| n.has_tag_name("Notes")) {
        doc.notes.text = n.attribute("Notes").unwrap_or_default().trim_end().to_string();
        doc.notes.show_on_open = n.attribute("ShowOnLoad") == Some("1");
    }
    Ok(LightBurnImport { doc, warnings })
}

struct Importer<'a> {
    doc: Document,
    warnings: Vec<String>,
    /// LightBurn (Y nach oben) → VectorLume (Y nach unten).
    flip: Affine,
    base_dir: Option<&'a Path>,
    /// Pfade teilen sich Punkt- und Segmentlisten über `VertID`/`PrimID`.
    verts: HashMap<String, Arc<Vec<Vert>>>,
    prims: HashMap<String, Arc<Prims>>,
    /// Kopien eines Bildes enthalten die Daten nur einmal; zugeordnet über den Dateinamen.
    images: HashMap<String, Arc<RasterImage>>,
    /// Von Formen benutzte LightBurn-Ebenennummern.
    used: Vec<usize>,
    /// LightBurn-Ebenennummer → Ebene in VectorLume (Vektoren bzw. Bilder).
    layer_map: HashMap<usize, usize>,
    image_map: HashMap<usize, usize>,
    /// Bild-Einstellungen von Ebenen, die auch Vektor-Einstellungen haben; werden beim ersten Bild auf eine
    /// eigene Ebene gelegt, weil eine VectorLume-Ebene nur eine Betriebsart hat.
    pending_image: HashMap<usize, LayerDef>,
    entries: Vec<LayerEntry>,
    /// Zusammengefasste Hinweise (Beschreibung → Anzahl).
    skipped: HashMap<String, usize>,
}

struct LayerDef {
    name: Option<String>,
    visible: bool,
    priority: i64,
    settings: CutSettings,
    sub: Option<CutSettings>,
}

struct LayerEntry {
    layer: usize,
    priority: i64,
    index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Vert {
    p: Point,
    /// Kontrollpunkt zum nächsten bzw. vom vorigen Punkt (Bézier).
    c0: Option<Point>,
    c1: Option<Point>,
}

#[derive(Debug, PartialEq)]
enum Prims {
    /// Segmente zwischen Punktnummern: Linie (`L`) oder Bézier (`B`).
    List(Vec<(bool, usize, usize)>),
    /// Alle Punkte als Linienzug.
    Polyline { closed: bool },
}

impl Importer<'_> {
    fn read_layers(&mut self, root: Node) {
        let mut vector: Vec<(usize, LayerDef)> = Vec::new();
        let mut image: Vec<(usize, LayerDef)> = Vec::new();
        for cs in root.children().filter(|n| n.has_tag_name("CutSetting") || n.has_tag_name("CutSetting_Img")) {
            let value = |name: &str| cs.children().find(|c| c.has_tag_name(name)).and_then(|c| c.attribute("Value"));
            let num = |name: &str| value(name).and_then(|v| v.trim().parse::<f64>().ok());
            let flag = |name: &str| value(name).map(|v| v != "0" && !v.eq_ignore_ascii_case("false"));
            let Some(index) = value("index").and_then(|v| v.trim().parse::<usize>().ok()) else { continue };
            let kind = cs.attribute("type").unwrap_or("Cut");
            let is_image = cs.has_tag_name("CutSetting_Img") || kind == "Image";
            let mode = if is_image {
                LayerMode::Image
            } else if kind.contains("Offset") {
                LayerMode::OffsetFill
            } else if kind.starts_with("Scan") {
                LayerMode::Fill
            } else {
                LayerMode::Line
            };
            let mut s = CutSettings::default_for(mode);
            if let Some(v) = num("speed") {
                s.speed = v * 60.0;
            }
            if let Some(v) = num("maxPower") {
                s.power_max = v.clamp(0.0, 100.0);
            }
            if let Some(v) = num("minPower") {
                s.power_min = v.clamp(0.0, s.power_max);
            }
            if let Some(v) = num("numPasses") {
                s.passes = (v.round() as u32).max(1);
            }
            if let Some(v) = num("interval").filter(|v| *v > 0.0) {
                s.interval = v;
            } else if let Some(dpi) = num("dpi").filter(|v| *v > 0.0) {
                s.interval = 25.4 / dpi;
            }
            if let Some(v) = num("angle") {
                s.fill_angle = v;
            }
            if let Some(v) = flag("crossHatch") {
                s.crosshatch = v;
            }
            if let Some(v) = flag("bidir") {
                s.bidirectional = v;
            }
            if let Some(v) = value("scanOpt") {
                s.fill_grouping = match v {
                    "byGroup" => FillGrouping::Groups,
                    "individual" => FillGrouping::Shapes,
                    _ => FillGrouping::All,
                };
            }
            if let Some(v) = flag("runBlower") {
                s.air_assist = v;
            }
            if let Some(v) = flag("doOutput") {
                s.output = v;
            }
            if let Some(v) = flag("negativeImage") {
                s.negative = v;
            }
            if let Some(m) = value("ditherMode") {
                s.dither = match m.to_ascii_lowercase().as_str() {
                    "threshold" => DitherMode::Threshold,
                    "ordered" => DitherMode::Ordered,
                    "atkinson" => DitherMode::Atkinson,
                    "jarvis" => DitherMode::Jarvis,
                    "stucki" => DitherMode::Stucki,
                    "floyd" | "floydsteinberg" => DitherMode::FloydSteinberg,
                    "burkes" => DitherMode::Burkes,
                    "sierra" => DitherMode::Sierra,
                    "grayscale" | "greyscale" => DitherMode::Grayscale,
                    "halftone" => DitherMode::Halftone,
                    other => {
                        self.warnings
                            .push(format!("Ebene {}: Bildmodus „{other}“ unbekannt – Jarvis verwendet.", index + 1));
                        DitherMode::Jarvis
                    }
                };
            }
            // „Füllen + Linie“: Kontur als Unterebene mit denselben Grundwerten
            let sub = (kind == "Scan+Cut").then(|| CutSettings::default_for(LayerMode::Line)).map(|mut line| {
                (line.speed, line.power_max, line.power_min, line.passes) =
                    (s.speed, s.power_max, s.power_min, s.passes);
                (line.air_assist, line.output) = (s.air_assist, s.output);
                line
            });
            let def = LayerDef {
                name: value("name").map(str::trim).filter(|n| !is_default_name(n)).map(str::to_string),
                visible: !flag("hide").unwrap_or(false),
                priority: num("priority").unwrap_or(0.0) as i64,
                settings: s,
                sub,
            };
            if is_image {
                image.push((index, def));
            } else {
                vector.push((index, def));
            }
        }
        // Normale Ebenen zuerst, damit Werkzeugebenen (ab Nummer 30) nur auf freie Ebenen kommen
        vector.sort_by_key(|e| e.0);
        for (index, def) in vector {
            self.place(index, def);
        }
        for (index, def) in image {
            if self.layer_map.contains_key(&index) {
                self.pending_image.insert(index, def);
            } else if let Some(layer) = self.place(index, def) {
                self.image_map.insert(index, layer);
            }
        }
    }

    /// Legt eine LightBurn-Ebene an; Werkzeugebenen werden nicht gelasert.
    fn place(&mut self, index: usize, mut def: LayerDef) -> Option<usize> {
        let layer = if index < LAYER_COUNT {
            index
        } else {
            def.settings.output = false;
            def.name = def.name.or_else(|| Some(format!("Werkzeug {}", index - LAYER_COUNT + 1)));
            self.free_layer()?
        };
        self.apply(layer, index, def);
        self.layer_map.insert(index, layer);
        Some(layer)
    }

    fn apply(&mut self, layer: usize, index: usize, def: LayerDef) {
        let l = &mut self.doc.layers[layer];
        if let Some(name) = def.name {
            l.name = name;
        }
        l.visible = def.visible;
        l.settings = def.settings;
        l.sub = def.sub.into_iter().collect();
        self.entries.push(LayerEntry { layer, priority: def.priority, index });
    }

    /// Ebene, die weder von LightBurn-Formen benutzt noch schon vergeben ist (von hinten gesucht).
    fn free_layer(&self) -> Option<usize> {
        (0..LAYER_COUNT).rev().find(|l| !self.used.contains(l) && !self.entries.iter().any(|e| e.layer == *l))
    }

    fn layer_of(&mut self, n: Node) -> usize {
        let index = n.attribute("CutIndex").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
        *self.layer_map.entry(index).or_insert(index.min(LAYER_COUNT - 1))
    }

    /// Ebene für Bilder: eigene Ebene, wenn die LightBurn-Ebene getrennte Bild-Einstellungen hat.
    fn image_layer_of(&mut self, n: Node) -> usize {
        let index = n.attribute("CutIndex").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
        if let Some(&l) = self.image_map.get(&index) {
            return l;
        }
        if let Some(mut def) = self.pending_image.remove(&index) {
            let vector_layer = self.layer_of(n);
            match self.free_layer() {
                Some(layer) => {
                    let base = def.name.take().unwrap_or_else(|| self.doc.layers[vector_layer].name.clone());
                    def.name = Some(format!("{base} Bild"));
                    self.apply(layer, index, def);
                    self.image_map.insert(index, layer);
                    return layer;
                }
                None => self.warnings.push(format!(
                    "Ebene {}: keine freie Ebene für die Bild-Einstellungen – Bilder bleiben auf der Ebene.",
                    index + 1
                )),
            }
        }
        self.layer_of(n)
    }

    fn skip(&mut self, what: impl Into<String>) {
        *self.skipped.entry(what.into()).or_default() += 1;
    }

    /// Liest eine Form samt Kindern; `parent` ist die Transformation der umgebenden Gruppen (LightBurn-Koordinaten).
    fn shape(&mut self, n: Node, parent: &Affine, ids: &mut Vec<ObjectId>) {
        let xf = xform(n).then(parent);
        let to_doc = xf.then(&self.flip);
        let ty = n.attribute("Type").unwrap_or("");
        let num = |name: &str| n.attribute(name).and_then(|v| v.trim().parse::<f64>().ok());
        let id = match ty {
            "Group" => {
                for c in n.children().filter(|c| c.has_tag_name("Children")).flat_map(|c| c.children()) {
                    if c.has_tag_name("Shape") {
                        self.shape(c, &xf, ids);
                    }
                }
                return;
            }
            "Rect" => {
                let (w, h) = (num("W").unwrap_or(0.0), num("H").unwrap_or(0.0));
                let rect = vl_core::primitive::Primitive::Rect {
                    x: -w / 2.0,
                    y: -h / 2.0,
                    width: w,
                    height: h,
                    radius: num("Cr").unwrap_or(0.0),
                };
                let id = self.add(n, "Rechteck", to_doc, Shape::Path(rect.path()));
                if let Some(o) = self.doc.get_mut(id) {
                    o.primitive = Some(rect);
                }
                id
            }
            "Ellipse" => {
                let path = VectorPath::ellipse(Point::ZERO, num("Rx").unwrap_or(0.0), num("Ry").unwrap_or(0.0));
                self.add(n, "Ellipse", to_doc, Shape::Path(path))
            }
            "Path" => match self.path(n) {
                Ok(path) => self.add(n, "Pfad", to_doc, Shape::Path(path)),
                Err(e) => {
                    self.skip(format!("Pfad nicht lesbar ({e})"));
                    return;
                }
            },
            "Bitmap" => {
                let Some(image) = self.bitmap(n) else { return };
                let (w, h) = (image.width_mm, image.height_mm);
                // Bildzeile 0 liegt oben, die Mitte des Bildes im Ursprung der Form
                let local = Affine([1.0, 0.0, 0.0, -1.0, -w / 2.0, h / 2.0]);
                let name = n
                    .attribute("File")
                    .and_then(|f| f.rsplit(['/', '\\']).next())
                    .filter(|f| !f.is_empty())
                    .unwrap_or("Bild")
                    .to_string();
                let layer = self.image_layer_of(n);
                self.doc.add(name, layer, local.then(&to_doc), Shape::Image(image))
            }
            _ => {
                // Texte und andere Formen über die mitgespeicherten Umrisse
                let backup = n.children().find(|c| c.has_tag_name("BackupPath"));
                let path = backup.map(|b| self.path(b));
                match path {
                    Some(Ok(path)) => {
                        let b = backup.unwrap();
                        let to_doc = xform(b).then(parent).then(&self.flip);
                        let name = n.attribute("Str").map(|s| s.replace('\n', " ")).unwrap_or_else(|| ty.to_string());
                        if ty == "Text" {
                            self.skip("Text als Umrisse übernommen (nicht mehr als Text bearbeitbar)");
                        }
                        self.add(n, &name, to_doc, Shape::Path(path))
                    }
                    _ => {
                        self.skip(format!("„{ty}“ ohne Umrisse übersprungen"));
                        return;
                    }
                }
            }
        };
        if let Some(s) = num("PowerScale").filter(|s| (*s - 100.0).abs() > 1e-9)
            && let Some(o) = self.doc.get_mut(id)
        {
            o.power_scale = s.clamp(1.0, 100.0);
        }
        ids.push(id);
    }

    fn add(&mut self, n: Node, name: &str, transform: Affine, shape: Shape) -> ObjectId {
        let layer = self.layer_of(n);
        self.doc.add(name, layer, transform, shape)
    }

    fn path(&mut self, n: Node) -> Result<VectorPath, String> {
        let list = |tag: &str| n.children().find(|c| c.has_tag_name(tag)).map(|c| c.text().unwrap_or(""));
        let verts = match (list("VertList"), n.attribute("VertID")) {
            (Some(t), id) => {
                let v = Arc::new(parse_verts(t)?);
                if let Some(id) = id {
                    self.verts.insert(id.to_string(), v.clone());
                }
                v
            }
            (None, Some(id)) => self.verts.get(id).cloned().ok_or("Punktliste fehlt")?,
            (None, None) => return Err("Punktliste fehlt".into()),
        };
        let prims = match (list("PrimList"), n.attribute("PrimID")) {
            (Some(t), id) => {
                let p = Arc::new(parse_prims(t)?);
                if let Some(id) = id {
                    self.prims.insert(id.to_string(), p.clone());
                }
                p
            }
            (None, Some(id)) => self.prims.get(id).cloned().ok_or("Segmentliste fehlt")?,
            (None, None) => Arc::new(Prims::Polyline { closed: true }),
        };
        build_path(&verts, &prims)
    }

    fn bitmap(&mut self, n: Node) -> Option<ImageObject> {
        let num = |name: &str| n.attribute(name).and_then(|v| v.trim().parse::<f64>().ok());
        let file = n.attribute("File").unwrap_or("");
        let decode = |b: Vec<u8>| RasterImage::from_encoded(&b).ok().map(Arc::new);
        let image = match n.attribute("Data").filter(|d| !d.is_empty()) {
            Some(d) => {
                use base64::Engine as _;
                base64::engine::general_purpose::STANDARD.decode(d.trim().as_bytes()).ok().and_then(decode)
            }
            None => self
                .images
                .get(file)
                .cloned()
                .or_else(|| self.find_file(file).and_then(|p| std::fs::read(p).ok()).and_then(decode)),
        };
        let Some(image) = image else {
            let name = file.rsplit(['/', '\\']).next().unwrap_or("");
            self.skip(format!("Bild „{name}“ nicht eingebettet und nicht gefunden – übersprungen"));
            return None;
        };
        if !file.is_empty() {
            self.images.entry(file.to_string()).or_insert_with(|| image.clone());
        }
        let mut obj = ImageObject {
            image,
            width_mm: num("W").unwrap_or(0.0),
            height_mm: num("H").unwrap_or(0.0),
            adjust: Default::default(),
            clip: Vec::new(),
        };
        // Werte −1…1 bzw. Gamma als Faktor
        obj.adjust.brightness = (num("Brightness").unwrap_or(0.0) * 100.0).clamp(-100.0, 100.0);
        obj.adjust.contrast = (num("Contrast").unwrap_or(0.0) * 100.0).round().clamp(-100.0, 100.0);
        obj.adjust.gamma = num("Gamma").filter(|g| *g > 0.0).unwrap_or(1.0).clamp(0.2, 5.0);
        if obj.width_mm <= 0.0 || obj.height_mm <= 0.0 {
            return None;
        }
        Some(obj)
    }

    fn find_file(&self, file: &str) -> Option<std::path::PathBuf> {
        if file.is_empty() {
            return None;
        }
        let p = Path::new(file);
        if p.is_file() {
            return Some(p.to_path_buf());
        }
        let name = file.rsplit(['/', '\\']).next()?;
        self.base_dir.map(|d| d.join(name)).filter(|p| p.is_file())
    }
}

/// „C00“ … „C29“ und „T1“/„T2“ sind die Standardnamen in LightBurn – dann bleibt der VectorLume-Name.
fn is_default_name(n: &str) -> bool {
    n.is_empty()
        || (n.len() == 3 && n.starts_with('C') && n[1..].chars().all(|c| c.is_ascii_digit()))
        || n == "T1"
        || n == "T2"
}

/// `<XForm>a b c d e f</XForm>`: x' = a·x + c·y + e, y' = b·x + d·y + f.
fn xform(n: Node) -> Affine {
    let Some(t) = n.children().find(|c| c.has_tag_name("XForm")).and_then(|c| c.text()) else {
        return Affine::IDENTITY;
    };
    let v: Vec<f64> = t.split_whitespace().filter_map(|s| s.parse().ok()).collect();
    match v[..] {
        [a, b, c, d, e, f] if v.iter().all(|x| x.is_finite()) => Affine([a, b, c, d, e, f]),
        _ => Affine::IDENTITY,
    }
}

/// Liest eine Zahl ab `pos` (auch mit Exponent) und gibt sie mit der Position danach zurück.
fn number(s: &[u8], mut pos: usize) -> Option<(f64, usize)> {
    while pos < s.len() && s[pos].is_ascii_whitespace() {
        pos += 1;
    }
    let start = pos;
    if pos < s.len() && (s[pos] == b'-' || s[pos] == b'+') {
        pos += 1;
    }
    while pos < s.len() && (s[pos].is_ascii_digit() || s[pos] == b'.') {
        pos += 1;
    }
    if pos < s.len() && (s[pos] == b'e' || s[pos] == b'E') {
        let mut p = pos + 1;
        if p < s.len() && (s[p] == b'-' || s[p] == b'+') {
            p += 1;
        }
        if p < s.len() && s[p].is_ascii_digit() {
            pos = p;
            while pos < s.len() && s[pos].is_ascii_digit() {
                pos += 1;
            }
        }
    }
    std::str::from_utf8(&s[start..pos]).ok()?.parse().ok().map(|v| (v, pos))
}

/// Punktliste: `V x y`, optional gefolgt von `c0x… c0y…` und `c1x… c1y…` (Kontrollpunkte). Ein einzelnes
/// `c0x1` ohne `c0y` bedeutet „kein Kontrollpunkt“; andere Kennbuchstaben (z. B. `S`) werden übersprungen.
fn parse_verts(t: &str) -> Result<Vec<Vert>, String> {
    let s = t.as_bytes();
    let mut out: Vec<Vert> = Vec::new();
    let mut pos = 0;
    let err = || "Punktliste fehlerhaft".to_string();
    while pos < s.len() {
        let c = s[pos];
        if c == b'V' {
            let (x, p) = number(s, pos + 1).ok_or_else(err)?;
            let (y, p) = number(s, p).ok_or_else(err)?;
            out.push(Vert { p: Point::new(x, y), c0: None, c1: None });
            pos = p;
        } else if c == b'c' && pos + 2 < s.len() && (s[pos + 1] == b'0' || s[pos + 1] == b'1') && s[pos + 2] == b'x' {
            let which = s[pos + 1];
            let (x, p) = number(s, pos + 3).ok_or_else(err)?;
            pos = p;
            let mut point = None;
            if s[pos..].starts_with(&[b'c', which, b'y']) {
                let (y, p) = number(s, pos + 3).ok_or_else(err)?;
                pos = p;
                point = Some(Point::new(x, y));
            }
            let v = out.last_mut().ok_or_else(err)?;
            if which == b'0' {
                v.c0 = point;
            } else {
                v.c1 = point;
            }
        } else {
            pos += 1;
        }
    }
    Ok(out)
}

/// Segmentliste: `L a b` (Linie) und `B a b` (Bézier) oder `LineClosed`/`LineOpen` für einen Linienzug.
fn parse_prims(t: &str) -> Result<Prims, String> {
    let t = t.trim();
    if t.is_empty() || t.starts_with("LineClosed") {
        return Ok(Prims::Polyline { closed: true });
    }
    if t.starts_with("LineOpen") {
        return Ok(Prims::Polyline { closed: false });
    }
    let s = t.as_bytes();
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < s.len() {
        match s[pos] {
            k @ (b'L' | b'B') => {
                let (a, p) = number(s, pos + 1).ok_or("Segmentliste fehlerhaft")?;
                let (b, p) = number(s, p).ok_or("Segmentliste fehlerhaft")?;
                out.push((k == b'B', a as usize, b as usize));
                pos = p;
            }
            c if c.is_ascii_whitespace() => pos += 1,
            c => return Err(format!("unbekanntes Segment „{}“", c as char)),
        }
    }
    Ok(Prims::List(out))
}

fn build_path(v: &[Vert], prims: &Prims) -> Result<VectorPath, String> {
    let mut pb = PathBuilder::new();
    match prims {
        Prims::Polyline { closed } => {
            let Some(first) = v.first() else { return Ok(VectorPath::default()) };
            pb.move_to(first.p);
            for w in &v[1..] {
                pb.line_to(w.p);
            }
            if *closed {
                pb.close();
            }
        }
        Prims::List(list) => {
            let get = |i: usize| v.get(i).ok_or_else(|| format!("Punkt {i} fehlt"));
            let mut start: Option<usize> = None;
            let mut last: Option<usize> = None;
            for &(bezier, a, b) in list {
                let (va, vb) = (get(a)?, get(b)?);
                if last != Some(a) {
                    pb.move_to(va.p);
                    start = Some(a);
                }
                if bezier {
                    pb.cubic_to(va.c0.unwrap_or(va.p), vb.c1.unwrap_or(vb.p), vb.p);
                } else {
                    pb.line_to(vb.p);
                }
                last = Some(b);
                if start == Some(b) {
                    pb.close();
                    last = None;
                }
            }
        }
    }
    Ok(pb.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<LightBurnProject AppVersion="2.0.05" FormatVersion="1" MaterialHeight="0" MirrorX="False" MirrorY="False">
    <Thumbnail Source=""/>
    <CutSetting type="Scan">
        <index Value="0"/><name Value="C00"/><maxPower Value="40"/><speed Value="100"/><priority Value="1"/>
        <interval Value="0.08"/><crossHatch Value="1"/>
    </CutSetting>
    <CutSetting type="Cut">
        <index Value="1"/><name Value="Ausschnitt"/><minPower Value="5"/><maxPower Value="90"/><speed Value="10"/>
        <numPasses Value="3"/><priority Value="0"/><doOutput Value="0"/>
    </CutSetting>
    <CutSetting type="Cut"><index Value="30"/><name Value="T1"/><maxPower Value="1"/><speed Value="100"/></CutSetting>
    <Shape Type="Rect" CutIndex="1" W="20" H="10" Cr="0" PowerScale="50"><XForm>1 0 0 1 30 40</XForm></Shape>
    <Shape Type="Ellipse" CutIndex="0" Rx="5" Ry="5"><XForm>1 0 0 1 100 100</XForm></Shape>
    <Shape Type="Group">
        <XForm>1 0 0 1 10 0</XForm>
        <Children>
            <Shape Type="Path" CutIndex="0" VertID="0" PrimID="0">
                <XForm>1 0 0 1 0 0</XForm>
                <VertList>V0 0c0x1c1x1V10 0c0x10c0y5c1x1V10 10c0x1c1x10c1y7S</VertList>
                <PrimList>L0 1B1 2L2 0</PrimList>
            </Shape>
            <Shape Type="Path" CutIndex="0" VertID="0" PrimID="0"><XForm>1 0 0 1 50 0</XForm></Shape>
        </Children>
    </Shape>
    <Shape Type="Path" CutIndex="30" VertID="1" PrimID="1">
        <XForm>1 0 0 1 0 0</XForm><VertList>V0 0c0x1c1x1V1e1 0c0x1c1x1</VertList><PrimList>LineOpen</PrimList>
    </Shape>
    <Shape Type="Text" CutIndex="0" Str="Hi" Font="Arial" H="5" HasBackupPath="1">
        <XForm>1 0 0 1 5 5</XForm>
        <BackupPath Type="Path" CutIndex="0">
            <XForm>1 0 0 1 5 5</XForm><VertList>V0 0c0x1c1x1V2 0c0x1c1x1V2 3c0x1c1x1</VertList><PrimList>LineClosed</PrimList>
        </BackupPath>
    </Shape>
    <Notes ShowOnLoad="1" Notes="Pappel 3 mm&#10;Fokus prüfen"/>
</LightBurnProject>"#;

    #[test]
    fn reads_layers_shapes_and_groups() {
        let im = import_lightburn(SAMPLE.as_bytes(), 400.0, None).unwrap();
        let doc = &im.doc;
        // Ebenen: Füllen mit Kreuzschraffur, Linie mit 3 Durchgängen und ohne Ausgabe, Werkzeugebene
        let l0 = &doc.layers[0].settings;
        assert_eq!(l0.mode, LayerMode::Fill);
        assert_eq!((l0.speed, l0.power_max, l0.interval, l0.crosshatch), (6000.0, 40.0, 0.08, true));
        let l1 = &doc.layers[1];
        assert_eq!(l1.name, "Ausschnitt");
        assert_eq!(doc.layers[0].name, "L01");
        assert_eq!((l1.settings.mode, l1.settings.passes, l1.settings.output), (LayerMode::Line, 3, false));
        assert_eq!((l1.settings.speed, l1.settings.power_min), (600.0, 5.0));
        let pos = |l: usize| doc.layer_order.iter().position(|&x| x == l).unwrap();
        assert!(pos(1) < pos(0), "Priorität 0 vor 1");
        let tool = doc.objects.iter().find(|o| o.name == "Pfad" && o.layer > 1).unwrap();
        assert!(!doc.layers[tool.layer].settings.output);
        assert_eq!(doc.layers[tool.layer].name, "Werkzeug 1");

        // Rechteck: Mitte bei (30, 40) in LightBurn → (30, 360) bei 400 mm Bett, Leistung 50 %
        let rect = doc.objects.iter().find(|o| o.name == "Rechteck").unwrap();
        let b = rect.bounds();
        assert!((b.center().x - 30.0).abs() < 1e-9 && (b.center().y - 360.0).abs() < 1e-9);
        assert!((b.width() - 20.0).abs() < 1e-9 && (b.height() - 10.0).abs() < 1e-9);
        assert_eq!((rect.layer, rect.power_scale), (1, 50.0));

        // Gruppe: zwei Pfade mit gemeinsamer Punktliste, Gruppenverschiebung angewendet
        let paths: Vec<_> = doc.objects.iter().filter(|o| o.name == "Pfad" && o.layer == 0).collect();
        assert_eq!(paths.len(), 2);
        assert!(paths[0].group.is_some() && paths[0].group == paths[1].group);
        assert!((paths[0].bounds().min.x - 10.0).abs() < 1e-6);
        assert!((paths[1].bounds().min.x - 60.0).abs() < 1e-6);
        let Shape::Path(p) = &paths[0].shape else { panic!() };
        assert_eq!(p.subpaths.len(), 1);
        assert!(p.subpaths[0].closed);
        // die schließende Linie zum Startpunkt entfällt, der Teilpfad ist geschlossen
        assert_eq!(p.subpaths[0].segments.len(), 2);

        // Text über die gespeicherten Umrisse, mit Hinweis
        let text = doc.objects.iter().find(|o| o.name == "Hi").unwrap();
        assert!((text.bounds().max.y - 395.0).abs() < 1e-9);
        assert!(im.warnings.iter().any(|w| w.contains("Text als Umrisse")));

        // Projektnotizen samt „beim Öffnen anzeigen“
        assert_eq!(doc.notes.text, "Pappel 3 mm\nFokus prüfen");
        assert!(doc.notes.show_on_open);
    }

    #[test]
    fn fill_plus_line_becomes_sublayer() {
        let xml = r#"<LightBurnProject AppVersion="2.0.05" FormatVersion="1">
    <CutSetting type="Scan+Cut"><index Value="3"/><maxPower Value="30"/><speed Value="80"/><numPasses Value="2"/></CutSetting>
    <Shape Type="Rect" CutIndex="3" W="10" H="10" Cr="0"><XForm>1 0 0 1 20 20</XForm></Shape>
</LightBurnProject>"#;
        let im = import_lightburn(xml.as_bytes(), 400.0, None).unwrap();
        let l = &im.doc.layers[3];
        assert_eq!(l.mode_labels(), ["Füllen", "Linie"]);
        assert_eq!((l.sub[0].speed, l.sub[0].power_max, l.sub[0].passes), (4800.0, 30.0, 2));
        assert!(im.warnings.is_empty(), "{:?}", im.warnings);
    }

    #[test]
    fn vertex_and_primitive_syntax() {
        let v = parse_verts("V1.5 -2e-1c0x3c0y4c1x1V-1 2S").unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].p, Point::new(1.5, -0.2));
        assert_eq!((v[0].c0, v[0].c1), (Some(Point::new(3.0, 4.0)), None));
        assert_eq!(v[1].p, Point::new(-1.0, 2.0));
        assert_eq!(parse_prims("L0 1B1 0").unwrap(), Prims::List(vec![(false, 0, 1), (true, 1, 0)]));
        assert!(parse_prims("X0 1").is_err());
        assert!(build_path(&v, &Prims::List(vec![(false, 0, 5)])).is_err());
        assert!(import_lightburn(b"<svg/>", 400.0, None).is_err());
    }

    /// Prüft echte Projekte: `VL_LBRN_DIR=… cargo test -p vl-io real_lightburn_files -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_lightburn_files() {
        let dir = std::env::var("VL_LBRN_DIR").expect("VL_LBRN_DIR setzen");
        let mut stack = vec![std::path::PathBuf::from(dir)];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|e| e == "lbrn2") {
                    let data = std::fs::read(&p).unwrap();
                    let im = import_lightburn(&data, 400.0, p.parent()).unwrap();
                    let b = im.doc.bounds();
                    println!(
                        "{:?}: {} Objekte, Ebenen {:?}, {:.1}×{:.1} mm bei ({:.1}, {:.1}), {:?}",
                        p.file_name().unwrap(),
                        im.doc.objects.len(),
                        im.doc.used_layers(),
                        b.width(),
                        b.height(),
                        b.min.x,
                        b.min.y,
                        im.warnings
                    );
                }
            }
        }
    }
}
