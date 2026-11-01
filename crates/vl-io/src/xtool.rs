//! xTool-Projekte übernehmen: `.xcs` (xTool Creative Space, JSON) und `.xs` (xTool Studio, ZIP-Archiv mit JSON und
//! Bildern). Gelesen wird nur das Dateiformat der Nutzerdaten.
//!
//! Übernommen werden Pfade, Rechtecke, Ellipsen, Bilder, Texte (als Umrisse aus den mitgespeicherten
//! Schriftzeichen), Gruppen und je xTool-Ebene die Bearbeitungsart (Schneiden, Linie gravieren, Füllen, Bild).
//! Laserwerte (Leistung, Geschwindigkeit, Durchgänge …) werden bewusst **nicht** übernommen: Sie gelten nur für den
//! xTool-Laser, für den die Datei angelegt wurde. Jede Ebene bekommt die Standardwerte von VectorLume und muss für
//! den eigenen Laser eingestellt werden (darauf weist die Oberfläche nach dem Öffnen hin).
//!
//! Lage der Objekte wie in PixiJS: Drehung (`angle`, Grad), Scherung (`skew`, Bogenmaß; `x = π` spiegelt) und
//! Skalierung ergeben die Matrix, verschoben um `graphicX/Y` (Pfade) bzw. `offsetX/Y` (Bilder, Formen, Texte).
//! xTool Studio speichert dieselbe Matrix zusätzlich als `transform`. Die gespeicherte Umrandung (`x`, `y` = Ecke
//! oben links der unverdrehten Form, `width`, `height`) dient als Gegenprobe; Abweichungen werden gemeldet.

use crate::zipread::{self, Archive};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use vl_core::path::PathBuilder;
use vl_core::primitive::Primitive;
use vl_core::{
    Affine, CutSettings, Document, ImageObject, LAYER_COUNT, LayerMode, ObjectId, Point, RasterImage, Rect, Shape,
    VectorPath,
};

pub const XTOOL_EXTENSIONS: &[&str] = &["xcs", "xs"];

pub fn is_xtool_extension(ext: &str) -> bool {
    XTOOL_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e))
}

/// Abstand zwischen nebeneinander gelegten Zeichenflächen.
const CANVAS_GAP: f64 = 20.0;
/// Erlaubte Abweichung zur gespeicherten Umrandung (mm, zuzüglich 0,2 % der Größe).
const TOLERANCE: f64 = 0.05;
/// Längster übernommener Text (Zeichen).
const MAX_TEXT: usize = 5000;

// Meldungen; `{}` wird der Reihe nach ersetzt. Alle stehen in `MESSAGES`, damit die Oberfläche sie übersetzen kann.
const ERR_NO_PROJECT: &str = "Die Datei ist kein xTool-Projekt.";
const ERR_NO_STUDIO: &str = "Das Archiv ist kein xTool-Studio-Projekt.";
const ERR_EMPTY: &str = "Das xTool-Projekt enthält keine übernehmbaren Objekte.";
const W_CANVASES: &str = "Das Projekt hat {} Zeichenflächen ({}). Die erste liegt an ihrer Stelle, die weiteren rechts daneben – zum Lasern jeweils auf die Arbeitsfläche schieben.";
const W_MODE: &str =
    "Zeichenfläche „{}“: der xTool-Modus „{}“ (z. B. Drehachse) wird nicht übernommen, nur die Zeichnung.";
const W_PROCESSING: &str = "Bearbeitungsart „{}“ aus xTool unbekannt – als „{}“ übernommen.";
const W_LAYERS_FULL: &str = "Mehr Ebenen-Kombinationen als Ebenen – einige Objekte liegen gemeinsam auf einer Ebene.";
const S_TYPE: &str = "Objektart „{}“ nicht unterstützt – übersprungen";
const S_PATH: &str = "Pfad ohne lesbare Daten übersprungen";
const S_INVALID: &str = "Objekt mit ungültiger Lage oder Größe übersprungen";
const S_IMAGE: &str = "Bild nicht lesbar – übersprungen";
const S_TEXT: &str = "Text ohne mitgespeicherte Schriftzeichen übersprungen (in xTool in Pfade umwandeln)";
const S_TEXT_OUTLINE: &str = "Text als Umrisse übernommen (nicht mehr als Text bearbeitbar)";
const S_GLYPH: &str = "Zeichen ohne Schriftdaten im Text ausgelassen";
const S_FILTER: &str = "Bildfilter aus xTool (Helligkeit, Kontrast, Umkehren …) nicht übernommen";
const S_MASK: &str = "Maske eines Bildes nicht übernommen – ganzes Bild";
const S_MISMATCH: &str =
    "Lage oder Größe weicht um mehr als 0,05 mm von den Angaben in der Datei ab – bitte nachmessen";

/// Alle Meldungen dieses Moduls (Vorlagen mit `{}`); Hinweise zu übersprungenen Objekten erscheinen als
/// „{Anzahl}× {Text}“.
pub const MESSAGES: &[&str] = &[
    ERR_NO_PROJECT,
    ERR_NO_STUDIO,
    ERR_EMPTY,
    W_CANVASES,
    W_MODE,
    W_PROCESSING,
    W_LAYERS_FULL,
    S_TYPE,
    S_PATH,
    S_INVALID,
    S_IMAGE,
    S_TEXT,
    S_TEXT_OUTLINE,
    S_GLYPH,
    S_FILTER,
    S_MASK,
    S_MISMATCH,
];

fn fill(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len() + 32);
    let mut rest = template;
    let mut args = args.iter();
    while let Some(at) = rest.find("{}") {
        out.push_str(&rest[..at]);
        out.push_str(args.next().copied().unwrap_or(""));
        rest = &rest[at + 2..];
    }
    out.push_str(rest);
    out
}

/// Ergebnis: neues Dokument, Hinweise und die angelegten Ebenen.
#[derive(Debug)]
pub struct XToolImport {
    pub doc: Document,
    pub warnings: Vec<String>,
    /// Ebenen mit übernommenen Objekten in Ausgabereihenfolge. Ihre Laserwerte sind die Standardwerte von
    /// VectorLume und müssen für den eigenen Laser eingestellt werden.
    pub layers: Vec<usize>,
    /// Gerät, für das die Datei angelegt wurde (z. B. „P2“, „S1“), falls angegeben.
    pub device: Option<String>,
    /// „xTool Creative Space“ oder „xTool Studio“.
    pub format: &'static str,
}

/// Liest ein xTool-Projekt; das Format wird am Inhalt erkannt (ZIP = xTool Studio, sonst JSON).
pub fn import_xtool(data: &[u8]) -> Result<XToolImport, String> {
    if zipread::is_zip(data) {
        let zip = Archive::open(data)?;
        let project = read_xs(&zip)?;
        build(project, "xTool Studio", &|path: &str| zip.read(path).ok())
    } else {
        let project = read_xcs(data)?;
        build(project, "xTool Creative Space", &|_: &str| None)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Einlesen der beiden Dateiformate in ein gemeinsames Modell

struct Project {
    canvases: Vec<Canvas>,
    device: Option<String>,
    warnings: Vec<String>,
}

struct Canvas {
    title: String,
    /// Ebenen nach Farbe („#rrggbb“, klein geschrieben).
    layers: HashMap<String, XLayer>,
    displays: Vec<Value>,
    /// Bearbeitung je Objekt-ID.
    procs: HashMap<String, Proc>,
}

struct XLayer {
    name: String,
    order: i64,
    visible: bool,
}

#[derive(Default)]
struct Proc {
    kind: Option<String>,
    ignored: bool,
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64).filter(|f| f.is_finite())
}

/// Einträge einer gespeicherten JavaScript-`Map` (`{"dataType":"Map","value":[[k, v], …]}`) oder eines Objekts.
fn map_entries(v: Option<&Value>) -> Vec<(&str, &Value)> {
    let Some(v) = v else { return Vec::new() };
    if let Some(list) = v.get("value").and_then(Value::as_array) {
        return list
            .iter()
            .filter_map(|e| {
                let pair = e.as_array()?;
                Some((pair.first()?.as_str()?, pair.get(1)?))
            })
            .collect();
    }
    v.as_object().map(|o| o.iter().map(|(k, v)| (k.as_str(), v)).collect()).unwrap_or_default()
}

fn read_layers(v: Option<&Value>) -> HashMap<String, XLayer> {
    map_entries(v)
        .into_iter()
        .map(|(color, l)| {
            let layer = XLayer {
                name: str_of(l, "name").unwrap_or(color).trim().to_string(),
                order: l.get("order").and_then(Value::as_i64).unwrap_or(i64::MAX),
                visible: l.get("visible").and_then(Value::as_bool).unwrap_or(true),
            };
            (color.to_ascii_lowercase(), layer)
        })
        .collect()
}

/// „{panel}1“ ist ein Platzhalter von xTool für „Zeichenfläche 1“.
fn canvas_title(v: &Value, index: usize) -> String {
    let t = str_of(v, "title").unwrap_or("").trim();
    let t = t.strip_prefix("{panel}").unwrap_or(t).trim();
    if t.is_empty() { (index + 1).to_string() } else { t.to_string() }
}

fn device_name(v: &Value) -> Option<String> {
    str_of(v, "extName").or_else(|| str_of(v, "extId")).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn mode_warning(warnings: &mut Vec<String>, title: &str, mode: Option<&str>) {
    if let Some(m) = mode.filter(|m| !m.is_empty() && *m != "LASER_PLANE") {
        warnings.push(fill(W_MODE, &[title, m]));
    }
}

/// xTool Creative Space: alles in einer JSON-Datei, Bilder als Base64.
fn read_xcs(data: &[u8]) -> Result<Project, String> {
    let mut v: Value = serde_json::from_slice(data).map_err(|_| ERR_NO_PROJECT.to_string())?;
    if v.get("canvas").and_then(Value::as_array).is_none() {
        return Err(ERR_NO_PROJECT.into());
    }
    let mut warnings = Vec::new();
    // Bearbeitung je Zeichenfläche und Objekt
    let mut procs: HashMap<String, (Option<String>, HashMap<String, Proc>)> = HashMap::new();
    for (cid, c) in map_entries(v.pointer("/device/data")) {
        let mut by_display = HashMap::new();
        for (did, d) in map_entries(c.get("displays")) {
            let p = Proc {
                kind: str_of(d, "processingType").map(str::to_string),
                ignored: d.get("processIgnore").and_then(Value::as_bool) == Some(true),
            };
            by_display.insert(did.to_string(), p);
        }
        procs.insert(cid.to_string(), (str_of(c, "mode").map(str::to_string), by_display));
    }
    let active = str_of(&v, "canvasId").map(str::to_string);
    let device = device_name(&v);
    let mut canvases = Vec::new();
    let list = v.get_mut("canvas").and_then(Value::as_array_mut).map(std::mem::take).unwrap_or_default();
    for (i, mut c) in list.into_iter().enumerate() {
        let id = str_of(&c, "id").unwrap_or("").to_string();
        let title = canvas_title(&c, i);
        let (mode, by_display) = procs.remove(&id).unwrap_or_default();
        mode_warning(&mut warnings, &title, mode.as_deref());
        let displays = c.get_mut("displays").and_then(Value::as_array_mut).map(std::mem::take).unwrap_or_default();
        let canvas = Canvas { title, layers: read_layers(c.get("layerData")), displays, procs: by_display };
        if active.as_deref() == Some(id.as_str()) {
            canvases.insert(0, canvas);
        } else {
            canvases.push(canvas);
        }
    }
    Ok(Project { canvases, device, warnings })
}

fn json(zip: &Archive, name: &str) -> Option<Value> {
    zip.read(name).ok().and_then(|d| serde_json::from_slice(&d).ok())
}

/// Nummer im Dateinamen „…-<n>.json“ (zum Sortieren der Teile).
fn chunk_number(name: &str) -> u64 {
    name.rsplit('-').next().and_then(|s| s.strip_suffix(".json")).and_then(|s| s.parse().ok()).unwrap_or(u64::MAX)
}

/// xTool Studio: Archiv mit `project.json`, `canvases/<id>.json`, Objekten in `canvases/<id>/displays-<n>.json`,
/// Pfaddaten in `vectors/…` (über `vectorRef`), Bildern in `resources/` und Bearbeitung in `devices/` +
/// `profiles.json`.
fn read_xs(zip: &Archive) -> Result<Project, String> {
    let names: Vec<String> = zip.names().map(str::to_string).collect();
    if !names.iter().any(|n| n.starts_with("canvases/")) {
        return Err(ERR_NO_STUDIO.into());
    }
    let project = json(zip, "project.json").unwrap_or(Value::Null);
    let profiles = json(zip, "profiles.json").unwrap_or(Value::Null);
    let profile_kind = |id: &str| {
        profiles.get("profiles").and_then(|p| p.get(id)).and_then(|p| str_of(p, "processingType")).map(str::to_string)
    };
    // Ausgelagerte Pfaddaten: Prüfsumme → Inhalt
    let mut vectors: HashMap<String, Value> = HashMap::new();
    for n in names.iter().filter(|n| n.starts_with("vectors/") && n.contains("/data-") && n.ends_with(".json")) {
        if let Some(Value::Object(entries)) = json(zip, n).and_then(|mut v| v.get_mut("entries").map(Value::take)) {
            vectors.extend(entries);
        }
    }
    // Gerät: das aktive, sonst das erste
    let device_file = str_of(&project, "activeDeviceId")
        .map(|id| format!("devices/device-{id}.json"))
        .filter(|f| zip.contains(f))
        .or_else(|| names.iter().find(|n| n.starts_with("devices/") && n.ends_with(".json")).cloned());
    let device = device_file.and_then(|f| json(zip, &f)).unwrap_or(Value::Null);
    // Reihenfolge der Zeichenflächen: aktive zuerst, dann wie im Projekt, dann übrige im Archiv
    let mut ids: Vec<String> = Vec::new();
    let mut push = |id: &str| {
        if !id.is_empty() && !ids.iter().any(|i| i == id) {
            ids.push(id.to_string());
        }
    };
    for key in ["activeCanvasId", "currentCanvasId"] {
        if let Some(id) = str_of(&project, key) {
            push(id);
        }
    }
    for id in project.pointer("/modules/canvases").and_then(Value::as_array).into_iter().flatten() {
        if let Some(id) = id.as_str() {
            push(id);
        }
    }
    for n in &names {
        if let Some(rest) = n.strip_prefix("canvases/") {
            push(rest.split('/').next().unwrap_or("").trim_end_matches(".json"));
        }
    }
    let mut warnings = Vec::new();
    let mut canvases = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        let meta = json(zip, &format!("canvases/{id}.json")).unwrap_or(Value::Null);
        let prefix = format!("canvases/{id}/displays-");
        let mut parts: Vec<&String> = names.iter().filter(|n| n.starts_with(&prefix)).collect();
        parts.sort_by_key(|n| chunk_number(n));
        let mut displays = Vec::new();
        for p in parts {
            if let Some(Value::Array(list)) = json(zip, p).and_then(|mut v| v.get_mut("displays").map(Value::take)) {
                displays.extend(list);
            }
        }
        if displays.is_empty() {
            continue;
        }
        for d in &mut displays {
            resolve_vector(d, &vectors);
        }
        let title = canvas_title(&meta, i);
        let mut procs: HashMap<String, Proc> = HashMap::new();
        if let Some(p) = device.get("processing").and_then(|p| p.get(id.as_str())) {
            let active = str_of(p, "activeMode");
            mode_warning(&mut warnings, &title, active);
            if let Some(m) = active.and_then(|a| p.get("modes").and_then(|m| m.get(a))) {
                for b in m.get("bindings").and_then(Value::as_array).into_iter().flatten() {
                    let kind = str_of(b, "baseProfileId").and_then(profile_kind);
                    for d in
                        b.get("displayIds").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str)
                    {
                        procs.entry(d.to_string()).or_default().kind = kind.clone();
                    }
                }
                for d in m.get("ignoredDisplayIds").and_then(Value::as_array).into_iter().flatten() {
                    if let Some(d) = d.as_str() {
                        procs.entry(d.to_string()).or_default().ignored = true;
                    }
                }
            }
        }
        canvases.push(Canvas { title, layers: read_layers(meta.get("layerData")), displays, procs });
    }
    Ok(Project { canvases, device: device_name(&project).or_else(|| device_name(&device)), warnings })
}

/// Ausgelagerte Daten (`vectorRef`) wieder an ihr Feld hängen.
fn resolve_vector(d: &mut Value, vectors: &HashMap<String, Value>) {
    let Some(r) = d.get("vectorRef") else { return };
    let Some(data) = str_of(r, "vectorHash").and_then(|h| vectors.get(h)) else { return };
    let field = str_of(r, "originalField").unwrap_or("dPath").to_string();
    let value = match data {
        // andere Felder als Pfaddaten können als JSON-Text abgelegt sein
        Value::String(s) if field != "dPath" => serde_json::from_str(s).unwrap_or_else(|_| data.clone()),
        _ => data.clone(),
    };
    if let Some(o) = d.as_object_mut()
        && !o.get(&field).is_some_and(|v| v.as_str().is_some_and(|s| !s.is_empty()) || v.is_object())
    {
        o.insert(field, value);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Aufbau des Dokuments

/// Bearbeitungsart einer xTool-Ebene; bestimmt Modus und Name der VectorLume-Ebene.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Image,
    Fill,
    Score,
    Line,
    Cut,
}

impl Kind {
    fn mode(self) -> LayerMode {
        match self {
            Kind::Image => LayerMode::Image,
            Kind::Fill => LayerMode::Fill,
            Kind::Score | Kind::Line | Kind::Cut => LayerMode::Line,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Kind::Image => "Bild",
            Kind::Fill => "Füllen",
            Kind::Score => "Gravieren",
            Kind::Line => "Linie",
            Kind::Cut => "Schneiden",
        }
    }

    /// Ausgabereihenfolge: erst gravieren, zuletzt schneiden (ausgeschnittene Teile können sich verschieben).
    fn rank(self) -> u8 {
        self as u8
    }
}

/// Bearbeitungsart aus der xTool-Bezeichnung; `None` als zweiter Wert, wenn sie bekannt ist.
fn kind_of(processing: Option<&str>, is_image: bool, is_fill: bool) -> (Kind, Option<String>) {
    let default = if is_image {
        Kind::Image
    } else if is_fill {
        Kind::Fill
    } else {
        Kind::Line
    };
    let (kind, unknown) = match processing.map(str::trim).filter(|p| !p.is_empty()) {
        None => (default, None),
        Some("VECTOR_CUTTING") => (Kind::Cut, None),
        Some("VECTOR_ENGRAVING") => (Kind::Score, None),
        Some("FILL_VECTOR_ENGRAVING") => (Kind::Fill, None),
        Some("BITMAP_ENGRAVING") => (Kind::Image, None),
        Some(p) => {
            let u = p.to_ascii_uppercase();
            let k = if u.contains("CUT") {
                Kind::Cut
            } else if u.contains("FILL") {
                Kind::Fill
            } else if u.contains("BITMAP") {
                Kind::Image
            } else if u.contains("ENGRAV") || u.contains("SCORE") {
                Kind::Score
            } else {
                default
            };
            (k, Some(p.to_string()))
        }
    };
    // Bilder lassen sich nur rastern, Vektorformen nicht als Bild
    let kind = match (is_image, kind) {
        (true, _) => Kind::Image,
        (false, Kind::Image) => Kind::Fill,
        (false, k) => k,
    };
    (kind, unknown)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum State {
    On,
    /// In xTool von der Bearbeitung ausgenommen.
    Ignored,
    /// In xTool ausgeblendet (Objekt oder Ebene).
    Hidden,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct LayerKey {
    color: String,
    kind: Kind,
    state: State,
}

struct Builder<'r> {
    doc: Document,
    warnings: Vec<String>,
    resource: &'r dyn Fn(&str) -> Option<Vec<u8>>,
    /// Vergebene Ebenen: Schlüssel, Ebene, Reihenfolge der xTool-Ebene.
    layers: Vec<(LayerKey, usize, i64)>,
    layers_full: bool,
    /// Zusammengefasste Hinweise (Text → Anzahl), in der Reihenfolge des ersten Auftretens.
    notes: Vec<(String, usize)>,
    images: HashMap<String, Arc<RasterImage>>,
}

impl Builder<'_> {
    fn note(&mut self, what: impl Into<String>) {
        let what = what.into();
        match self.notes.iter_mut().find(|(w, _)| *w == what) {
            Some((_, n)) => *n += 1,
            None => self.notes.push((what, 1)),
        }
    }

    fn warn_once(&mut self, w: String) {
        if !self.warnings.contains(&w) {
            self.warnings.push(w);
        }
    }

    /// Ebene für eine Kombination aus xTool-Ebene, Bearbeitungsart und Zustand; beim ersten Mal angelegt.
    fn layer_for(&mut self, key: LayerKey, x: Option<&XLayer>) -> usize {
        if let Some((_, l, _)) = self.layers.iter().find(|(k, ..)| *k == key) {
            return *l;
        }
        let taken = |l: usize| self.layers.iter().any(|(_, x, _)| *x == l);
        let preferred = parse_color(&key.color).and_then(crate::layer_for_color).filter(|l| !taken(*l));
        let Some(layer) = preferred.or_else(|| (0..LAYER_COUNT).find(|l| !taken(*l))) else {
            if !self.layers_full {
                self.layers_full = true;
                self.warnings.push(W_LAYERS_FULL.into());
            }
            let same = |k: &LayerKey| k.kind.mode() == key.kind.mode() && k.state == key.state;
            return self.layers.iter().find(|(k, ..)| same(k)).map_or(LAYER_COUNT - 1, |e| e.1);
        };
        // Namen wie „{Black}“ oder „{layer} 1“ sind Übersetzungs-Platzhalter von xTool → Farbcode wie in xTool
        let base = x
            .map(|x| x.name.as_str())
            .filter(|n| !n.is_empty() && !n.contains('{'))
            .map_or_else(|| key.color.to_ascii_uppercase(), str::to_string);
        let l = &mut self.doc.layers[layer];
        l.settings = CutSettings::default_for(key.kind.mode());
        l.settings.output = key.state == State::On;
        l.visible = key.state != State::Hidden;
        l.name = match key.state {
            State::On => format!("{} {base}", key.kind.label()),
            State::Ignored => format!("{} {base} (nicht bearbeiten)", key.kind.label()),
            State::Hidden => format!("{} {base} (ausgeblendet)", key.kind.label()),
        };
        self.layers.push((key, layer, x.map_or(i64::MAX, |x| x.order)));
        layer
    }

    fn canvas(&mut self, cv: &Canvas) -> Vec<ObjectId> {
        let mut ids = Vec::new();
        let mut groups: Vec<(String, Vec<ObjectId>)> = Vec::new();
        // Zeichenreihenfolge: höheres `zOrder` liegt oben (die Reihenfolge in der Datei ist eine andere)
        let mut displays: Vec<&Value> = cv.displays.iter().collect();
        let z = |d: &Value| d.get("zOrder").and_then(Value::as_f64);
        if displays.iter().all(|d| z(d).is_some()) {
            displays.sort_by(|a, b| z(a).unwrap_or(0.0).total_cmp(&z(b).unwrap_or(0.0)));
        }
        for d in displays {
            let Some(id) = self.display(d, cv) else { continue };
            ids.push(id);
            // äußerste Gruppe (VectorLume kennt nur eine Gruppenebene)
            let tag = d
                .get("groupTags")
                .and_then(Value::as_array)
                .and_then(|t| t.iter().rev().find_map(|t| t.as_str().filter(|s| !s.is_empty())))
                .or_else(|| str_of(d, "groupTag").filter(|s| !s.is_empty()));
            if let Some(tag) = tag {
                match groups.iter_mut().find(|(t, _)| t == tag) {
                    Some((_, members)) => members.push(id),
                    None => groups.push((tag.to_string(), vec![id])),
                }
            }
        }
        for (_, members) in groups {
            self.doc.group(&members);
        }
        ids
    }

    fn display(&mut self, d: &Value, cv: &Canvas) -> Option<ObjectId> {
        let ty = str_of(d, "type").unwrap_or("").to_ascii_uppercase();
        let has_path = str_of(d, "dPath").is_some_and(|p| !p.trim().is_empty());
        let made = match ty.as_str() {
            "BITMAP" => self.bitmap(d),
            "TEXT" => self.text(d),
            "RECT" => self.rect(d),
            "CIRCLE" | "ELLIPSE" => self.ellipse(d),
            _ if has_path => self.path(d),
            "PATH" => {
                self.note(S_PATH);
                None
            }
            _ => {
                let name = if ty.is_empty() { "?" } else { ty.as_str() };
                self.note(fill(S_TYPE, &[name]));
                None
            }
        };
        let (name, transform, shape, primitive) = made?;
        // Ebene: xTool-Ebene (Farbe) + Bearbeitungsart + Zustand
        let color = str_of(d, "layerColor")
            .or_else(|| str_of(d, "layerTag"))
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| "#000000".into());
        let xlayer = cv.layers.get(&color);
        let did = str_of(d, "id").unwrap_or("");
        let proc = cv.procs.get(did);
        let processing = proc
            .and_then(|p| p.kind.as_deref())
            .or_else(|| d.pointer("/customData/processingType").and_then(Value::as_str));
        let is_image = matches!(shape, Shape::Image(_));
        let is_fill = d.get("isFill").and_then(Value::as_bool) == Some(true);
        let (kind, unknown) = kind_of(processing, is_image, is_fill);
        if let Some(p) = unknown {
            self.warn_once(fill(W_PROCESSING, &[&p, kind.label()]));
        }
        let hidden = d.get("visible").and_then(Value::as_bool) == Some(false) || xlayer.is_some_and(|l| !l.visible);
        let ignored = proc.is_some_and(|p| p.ignored);
        let state = if hidden {
            State::Hidden
        } else if ignored {
            State::Ignored
        } else {
            State::On
        };
        let layer = self.layer_for(LayerKey { color, kind, state }, xlayer);
        let id = self.doc.add(name, layer, transform, shape);
        if let Some(o) = self.doc.get_mut(id) {
            o.primitive = primitive;
        }
        Some(id)
    }

    /// Vergleicht die berechnete Lage mit der in der Datei gespeicherten Umrandung.
    fn check(&mut self, d: &Value, m: &Affine, local: Rect, scale: (f64, f64)) {
        let (Some(x), Some(y), Some(w), Some(h)) = (num(d, "x"), num(d, "y"), num(d, "width"), num(d, "height")) else {
            return;
        };
        let o = m.apply(local.min);
        let (lw, lh) = (local.width() * scale.0.abs(), local.height() * scale.1.abs());
        let tol = |size: f64| TOLERANCE + 0.002 * size.abs();
        if (o.x - x).abs() > tol(w) || (o.y - y).abs() > tol(h) || (lw - w).abs() > tol(w) || (lh - h).abs() > tol(h) {
            self.note(S_MISMATCH);
        }
    }

    fn path(&mut self, d: &Value) -> Option<Made> {
        let Some(path) = str_of(d, "dPath").and_then(parse_dpath) else {
            self.note(S_PATH);
            return None;
        };
        let (Some(m), scale) = (matrix(d, false, true), scale_of(d)) else {
            self.note(S_INVALID);
            return None;
        };
        self.check(d, &m, path.bounds(&Affine::IDENTITY), scale);
        Some((display_name(d, "Pfad"), m, Shape::Path(path), None))
    }

    fn rect(&mut self, d: &Value) -> Option<Made> {
        let (Some(m), Some((w, h))) = (matrix(d, true, false), size_of(d)) else {
            self.note(S_INVALID);
            return None;
        };
        let radius = num(d, "radius").unwrap_or(0.0).clamp(0.0, w.min(h) / 2.0);
        let rect = Primitive::Rect { x: 0.0, y: 0.0, width: w, height: h, radius };
        Some((display_name(d, "Rechteck"), m, Shape::Path(rect.path()), Some(rect)))
    }

    fn ellipse(&mut self, d: &Value) -> Option<Made> {
        let (Some(m), Some((w, h))) = (matrix(d, true, false), size_of(d)) else {
            self.note(S_INVALID);
            return None;
        };
        let path = VectorPath::ellipse(Point::new(w / 2.0, h / 2.0), w / 2.0, h / 2.0);
        Some((display_name(d, "Ellipse"), m, Shape::Path(path), None))
    }

    fn bitmap(&mut self, d: &Value) -> Option<Made> {
        let Some(image) = self.image_data(d) else {
            self.note(S_IMAGE);
            return None;
        };
        let (sx, sy) = scale_of(d);
        let ow = num(d, "originWidth").filter(|v| *v > 0.0).unwrap_or(image.width as f64);
        let oh = num(d, "originHeight").filter(|v| *v > 0.0).unwrap_or(image.height as f64);
        let (w, h) = (ow * sx.abs(), oh * sy.abs());
        let Some(m) = matrix(d, true, false).filter(|_| w > 1e-6 && h > 1e-6 && w < 1e6 && h < 1e6) else {
            self.note(S_INVALID);
            return None;
        };
        self.check(d, &m, Rect::from_xywh(0.0, 0.0, w, h), (1.0, 1.0));
        let filtered = num(d, "brightness").is_some_and(|v| v != 0.0)
            || num(d, "contrast").is_some_and(|v| v != 0.0)
            || d.get("colorInverted").and_then(Value::as_bool) == Some(true);
        if filtered {
            self.note(S_FILTER);
        }
        if d.get("mask").is_some_and(|m| !m.is_null()) {
            self.note(S_MASK);
        }
        let obj = ImageObject { image, width_mm: w, height_mm: h, adjust: Default::default(), clip: Vec::new() };
        Some((display_name(d, "Bild"), m, Shape::Image(obj), None))
    }

    /// Bilddaten: eingebettet als Base64 (Creative Space) oder als Datei im Archiv (Studio).
    fn image_data(&mut self, d: &Value) -> Option<Arc<RasterImage>> {
        let (key, bytes) = if let Some(path) = str_of(d, "resourcePath").filter(|p| !p.is_empty()) {
            if let Some(img) = self.images.get(path) {
                return Some(img.clone());
            }
            (path.to_string(), (self.resource)(path)?)
        } else {
            let b64 = str_of(d, "base64")?;
            let data = b64.find("base64,").map_or(b64, |i| &b64[i + 7..]).trim();
            let key = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                data.hash(&mut h);
                format!("base64:{}:{:x}", data.len(), h.finish())
            };
            if let Some(img) = self.images.get(&key) {
                return Some(img.clone());
            }
            use base64::Engine as _;
            (key, base64::engine::general_purpose::STANDARD.decode(data.as_bytes()).ok()?)
        };
        let img = Arc::new(RasterImage::from_encoded(&bytes).ok()?);
        if img.width == 0 || img.height == 0 {
            return None;
        }
        self.images.insert(key, img.clone());
        Some(img)
    }

    /// Text aus den mitgespeicherten Schriftzeichen (Umriss, Vorschub je Zeichen, Zeilenabstand).
    fn text(&mut self, d: &Value) -> Option<Made> {
        let text: String = str_of(d, "text").unwrap_or("").chars().take(MAX_TEXT).collect();
        let fd = d.get("fontData");
        let Some(glyphs) = fd.and_then(|f| f.get("glyphData")).and_then(Value::as_object) else {
            self.note(S_TEXT);
            return None;
        };
        let style = d.get("style").cloned().unwrap_or(Value::Null);
        let em = glyphs.values().find_map(|g| num(g, "advanceHeight")).filter(|v| *v > 0.0).unwrap_or(25.4);
        let line_height = fd
            .and_then(|f| f.get("fontInfo"))
            .and_then(|i| num(i, "lineHeight"))
            .filter(|v| *v > 0.0)
            .unwrap_or(em * 1.2)
            + num(&style, "leading").unwrap_or(0.0);
        let spacing = num(&style, "letterSpacing").unwrap_or(0.0);
        let mut cache: HashMap<char, Option<VectorPath>> = HashMap::new();
        let mut lines: Vec<(Vec<(VectorPath, f64)>, f64)> = Vec::new();
        let mut missing = false;
        for line in text.split('\n') {
            let mut pen = 0.0;
            let mut parts = Vec::new();
            let mut count = 0;
            for ch in line.trim_end_matches('\r').chars() {
                let Some(g) = glyphs.get(ch.encode_utf8(&mut [0; 4]) as &str) else {
                    if !ch.is_whitespace() {
                        missing = true;
                    }
                    pen += em * 0.25 + spacing;
                    count += 1;
                    continue;
                };
                let outline = cache.entry(ch).or_insert_with(|| str_of(g, "dPath").and_then(parse_dpath));
                if let Some(p) = outline {
                    parts.push((p.clone(), pen));
                }
                pen += num(g, "advanceWidth").unwrap_or(em * 0.5) + spacing;
                count += 1;
            }
            let width = if count > 0 { pen - spacing } else { 0.0 };
            lines.push((parts, width));
        }
        if missing {
            self.note(S_GLYPH);
        }
        let widest = lines.iter().map(|l| l.1).fold(0.0, f64::max);
        let align = str_of(&style, "align").unwrap_or("left");
        let mut path = VectorPath::default();
        for (i, (parts, width)) in lines.into_iter().enumerate() {
            let dx = match align {
                "center" => (widest - width) / 2.0,
                "right" => widest - width,
                _ => 0.0,
            };
            for (p, x) in parts {
                path.append(p.transformed(&Affine::translate(x + dx, i as f64 * line_height)));
            }
        }
        if path.is_empty() {
            self.note(S_TEXT);
            return None;
        }
        let Some(m) = matrix(d, false, false) else {
            self.note(S_INVALID);
            return None;
        };
        self.check(d, &m, path.bounds(&Affine::IDENTITY), scale_of(d));
        self.note(S_TEXT_OUTLINE);
        let name: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(40).collect();
        let name = if name.is_empty() { "Text".to_string() } else { name };
        Some((name, m, Shape::Path(path), None))
    }
}

type Made = (String, Affine, Shape, Option<Primitive>);

fn display_name(d: &Value, fallback: &str) -> String {
    str_of(d, "name").map(str::trim).filter(|n| !n.is_empty()).unwrap_or(fallback).to_string()
}

fn scale_of(d: &Value) -> (f64, f64) {
    let s = |k: &str| d.get("scale").and_then(|s| num(s, k)).unwrap_or(1.0);
    (s("x"), s("y"))
}

fn size_of(d: &Value) -> Option<(f64, f64)> {
    let (w, h) = (num(d, "width")?, num(d, "height")?);
    (w > 1e-6 && h > 1e-6 && w < 1e6 && h < 1e6).then_some((w, h))
}

/// Lage wie in PixiJS. `unit`: nur Drehung, Scherung und Spiegelung (für Formen, deren Größe in mm gespeichert ist);
/// `use_graphic`: Verschiebung aus `graphicX/Y` (Pfade).
fn matrix(d: &Value, unit: bool, use_graphic: bool) -> Option<Affine> {
    let r = num(d, "angle").unwrap_or(0.0).to_radians();
    let skew = |k: &str| d.get("skew").and_then(|s| num(s, k)).unwrap_or(0.0);
    let (kx, ky) = (skew("x"), skew("y"));
    let (mut sx, mut sy) = scale_of(d);
    if unit {
        sx = if sx < 0.0 { -1.0 } else { 1.0 };
        sy = if sy < 0.0 { -1.0 } else { 1.0 };
    }
    let (a, b) = ((r + ky).cos() * sx, (r + ky).sin() * sx);
    let (c, dd) = (-(r - kx).sin() * sy, (r - kx).cos() * sy);
    let pos = |g: &str, o: &str, p: &str| {
        use_graphic.then(|| num(d, g)).flatten().or_else(|| num(d, o)).or_else(|| num(d, p)).unwrap_or(0.0)
    };
    let pivot = |k: &str| d.get("pivot").and_then(|s| num(s, k)).unwrap_or(0.0);
    let (px, py) = (pivot("x"), pivot("y"));
    let tx = pos("graphicX", "offsetX", "x") - (px * a + py * c);
    let ty = pos("graphicY", "offsetY", "y") - (px * b + py * dd);
    let m = Affine([a, b, c, dd, tx, ty]);
    let ok = m.0.iter().all(|v| v.is_finite() && v.abs() < 1e7) && (a * dd - b * c).abs() > 1e-12;
    ok.then_some(m)
}

/// SVG-Pfaddaten; bei einem Fehler zählt der bis dahin gelesene Teil. `None` ohne verwertbare Segmente oder bei
/// unsinnigen Koordinaten.
fn parse_dpath(d: &str) -> Option<VectorPath> {
    use svgtypes::SimplePathSegment as S;
    let mut pb = PathBuilder::new();
    let ok = |v: f64| v.is_finite() && v.abs() < 1e7;
    for seg in svgtypes::SimplifyingPathParser::from(d) {
        let Ok(seg) = seg else { break };
        let p = |x: f64, y: f64| (ok(x) && ok(y)).then(|| Point::new(x, y));
        match seg {
            S::MoveTo { x, y } => pb.move_to(p(x, y)?),
            S::LineTo { x, y } => pb.line_to(p(x, y)?),
            S::CurveTo { x1, y1, x2, y2, x, y } => pb.cubic_to(p(x1, y1)?, p(x2, y2)?, p(x, y)?),
            S::Quadratic { x1, y1, x, y } => pb.quad_to(p(x1, y1)?, p(x, y)?),
            S::ClosePath => pb.close(),
        }
    }
    let path = pb.finish();
    (!path.is_empty()).then_some(path)
}

fn parse_color(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([c(0)?, c(2)?, c(4)?])
}

fn build(p: Project, format: &'static str, resource: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<XToolImport, String> {
    let mut b = Builder {
        doc: Document::new(),
        warnings: p.warnings,
        resource,
        layers: Vec::new(),
        layers_full: false,
        notes: Vec::new(),
        images: HashMap::new(),
    };
    let mut right: Option<f64> = None;
    let mut titles = Vec::new();
    for cv in &p.canvases {
        let ids = b.canvas(cv);
        if ids.is_empty() {
            continue;
        }
        // weitere Zeichenflächen rechts neben die vorigen legen
        if let Some(edge) = right {
            let bb = b.doc.bounds_of(&ids);
            b.doc.transform_objects(&ids, &Affine::translate(edge + CANVAS_GAP - bb.min.x, 0.0));
        }
        right = Some(b.doc.bounds_of(&ids).max.x);
        titles.push(format!("„{}“", cv.title));
    }
    let mut warnings = std::mem::take(&mut b.warnings);
    if titles.len() > 1 {
        warnings.insert(0, fill(W_CANVASES, &[&titles.len().to_string(), &titles.join(", ")]));
    }
    warnings.extend(b.notes.iter().map(|(what, n)| format!("{n}× {what}")));
    if b.doc.objects.is_empty() {
        let mut e = ERR_EMPTY.to_string();
        for w in &warnings {
            e.push_str("\n• ");
            e.push_str(w);
        }
        return Err(e);
    }
    // Ausgabereihenfolge: Bearbeitungsart, dann Reihenfolge der xTool-Ebenen
    let mut used: Vec<(u8, i64, usize, usize)> =
        b.layers.iter().enumerate().map(|(i, (k, l, order))| (k.kind.rank(), *order, i, *l)).collect();
    used.sort();
    let mut layers: Vec<usize> = Vec::new();
    for (.., l) in used {
        if !layers.contains(&l) {
            layers.push(l);
        }
    }
    let mut order = layers.clone();
    order.extend((0..LAYER_COUNT).filter(|l| !layers.contains(l)));
    b.doc.layer_order = order;
    Ok(XToolImport { doc: b.doc, warnings, layers, device: p.device, format })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn png_base64(w: u32, h: u32) -> String {
        use base64::Engine as _;
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 255]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(out))
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    /// Kleines Creative-Space-Projekt: Pfad (verschoben, skaliert), gedrehtes Rechteck, Kreis, Bild, Text.
    fn xcs() -> Value {
        json!({
            "canvasId": "c1",
            "extId": "D1Pro",
            "version": "2.2.23",
            "canvas": [{
                "id": "c1",
                "title": "{panel}1",
                "layerData": {
                    "#fe0002": {"name": "Rot", "order": 2, "visible": true},
                    "#000000": {"name": "{Black}", "order": 1, "visible": true}
                },
                "displays": [
                    {"id": "p1", "type": "PATH", "zOrder": 9, "x": 15.0, "y": 25.0, "width": 10.0, "height": 5.0, "angle": 0,
                     "scale": {"x": 0.5, "y": 0.5}, "skew": {"x": 0, "y": 0}, "pivot": {"x": 0, "y": 0},
                     "offsetX": 10.0, "offsetY": 20.0, "graphicX": 10.0, "graphicY": 20.0,
                     "dPath": "M10 10 h20 v10 h-20 z", "layerColor": "#fe0002", "groupTag": "g1"},
                    {"id": "r1", "type": "RECT", "zOrder": 1, "x": 100.0, "y": 50.0, "width": 20.0, "height": 10.0, "angle": 90,
                     "scale": {"x": 2.0, "y": 2.0}, "offsetX": 100.0, "offsetY": 50.0, "radius": 1.0,
                     "layerColor": "#fe0002", "groupTag": "g1"},
                    {"id": "e1", "type": "CIRCLE", "zOrder": 2, "x": 40.0, "y": 40.0, "width": 6.0, "height": 6.0, "angle": 0,
                     "scale": {"x": 0.01, "y": 0.01}, "offsetX": 40.0, "offsetY": 40.0, "layerColor": "#fe0002"},
                    {"id": "b1", "type": "BITMAP", "zOrder": 0, "x": 0.0, "y": 0.0, "width": 40.0, "height": 20.0, "angle": 0,
                     "scale": {"x": 0.2, "y": 0.2}, "offsetX": 0.0, "offsetY": 0.0, "originWidth": 200,
                     "originHeight": 100, "base64": png_base64(4, 2), "layerColor": "#000000"},
                    {"id": "t1", "type": "TEXT", "zOrder": 3, "x": 50.0, "y": 118.0, "width": 3.0, "height": 2.0, "angle": 0,
                     "scale": {"x": 0.1, "y": 0.1}, "offsetX": 50.0, "offsetY": 120.0, "isFill": true,
                     "text": "AA", "style": {"align": "left", "letterSpacing": 0, "leading": 0},
                     "layerColor": "#000000",
                     "fontData": {"fontInfo": {"lineHeight": 30}, "glyphData": {
                         "A": {"dPath": "M0 -20 L10 -20 L10 0 L0 0 Z", "advanceWidth": 20, "advanceHeight": 25.4}}}},
                    {"id": "x1", "type": "QRCODE", "zOrder": 4, "x": 0, "y": 0, "width": 1, "height": 1, "layerColor": "#000000"}
                ]
            }],
            "device": {"data": {"dataType": "Map", "value": [["c1", {"mode": "LASER_PLANE", "displays": {
                "dataType": "Map", "value": [
                    ["p1", {"processingType": "VECTOR_CUTTING", "data": {"VECTOR_CUTTING": {"parameter": {
                        "customize": {"power": 100, "speed": 5}}}}}],
                    ["r1", {"processingType": "VECTOR_ENGRAVING"}],
                    ["e1", {"processingType": "VECTOR_CUTTING", "processIgnore": true}],
                    ["b1", {"processingType": "BITMAP_ENGRAVING"}],
                    ["t1", {"processingType": "FILL_VECTOR_ENGRAVING"}]
                ]}}]]}}
        })
    }

    fn by_name<'a>(doc: &'a Document, name: &str) -> &'a vl_core::Object {
        doc.objects.iter().find(|o| o.name == name).unwrap_or_else(|| panic!("{name} fehlt"))
    }

    #[test]
    fn creative_space_project() {
        let im = import_xtool(&serde_json::to_vec(&xcs()).unwrap()).unwrap();
        let doc = &im.doc;
        assert_eq!(im.format, "xTool Creative Space");
        assert_eq!(im.device.as_deref(), Some("D1Pro"));
        assert_eq!(doc.objects.len(), 5, "{:?}", im.warnings);
        // Zeichenreihenfolge nach zOrder: Bild unten, Pfad oben
        let order: Vec<&str> = doc.objects.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(order, ["Bild", "Rechteck", "Ellipse", "AA", "Pfad"]);
        // Pfad: 0,5-fach, verschoben um graphicX/Y → genau die gespeicherte Umrandung
        let p = by_name(doc, "Pfad");
        let b = p.bounds();
        assert!(
            close(b.min.x, 15.0) && close(b.min.y, 25.0) && close(b.width(), 10.0) && close(b.height(), 5.0),
            "{b:?}"
        );
        // Rechteck um 90° gedreht: Ecke (100, 50), 20 mm in Y, 10 mm in −X
        let r = by_name(doc, "Rechteck");
        let b = r.bounds();
        assert!(close(b.min.x, 90.0) && close(b.max.x, 100.0) && close(b.min.y, 50.0) && close(b.max.y, 70.0), "{b:?}");
        assert!(matches!(r.primitive, Some(Primitive::Rect { radius, .. }) if close(radius, 1.0)));
        // Kreis: x/y ist die Ecke oben links
        let b = by_name(doc, "Ellipse").bounds();
        assert!(close(b.min.x, 40.0) && close(b.max.x, 46.0), "{b:?}");
        // Bild: 200 × 100 Pixel × 0,2 = 40 × 20 mm
        let img = by_name(doc, "Bild");
        let b = img.bounds();
        assert!(close(b.width(), 40.0) && close(b.height(), 20.0), "{b:?}");
        // Text: zwei Zeichen, 10 Einheiten breit, Vorschub 20 → 30 Einheiten × 0,1
        let t = by_name(doc, "AA");
        let b = t.bounds();
        assert!(
            close(b.min.x, 50.0) && close(b.width(), 3.0) && close(b.min.y, 118.0) && close(b.height(), 2.0),
            "{b:?}"
        );
        // Gruppe aus Pfad und Rechteck
        assert!(p.group.is_some() && p.group == r.group);
        // Ebenen je Farbe und Bearbeitungsart, Laserwerte = Standardwerte
        let layer = |o: &vl_core::Object| &doc.layers[o.layer];
        assert_eq!(layer(p).name, "Schneiden Rot");
        assert_eq!(layer(p).settings, CutSettings::default_for(LayerMode::Line));
        assert_eq!(layer(r).name, "Gravieren Rot");
        assert_ne!(p.layer, r.layer);
        assert_eq!(layer(img).settings.mode, LayerMode::Image);
        assert_eq!(layer(img).name, "Bild #000000", "Platzhalter-Name ersetzt");
        assert_eq!(layer(t).settings.mode, LayerMode::Fill);
        let e = layer(by_name(doc, "Ellipse"));
        assert!(!e.settings.output && e.name.contains("nicht bearbeiten"));
        // die erste rote xTool-Ebene landet auf der roten Ebene von VectorLume
        assert!([p.layer, r.layer].contains(&crate::layer_for_color([0xfe, 0, 2]).unwrap()));
        // Ausgabe: Bild, Füllen, Gravieren, Schneiden
        let modes: Vec<&str> = im.layers.iter().map(|&l| doc.layers[l].name.split(' ').next().unwrap()).collect();
        assert_eq!(modes, ["Bild", "Füllen", "Gravieren", "Schneiden", "Schneiden"]);
        assert_eq!(&doc.layer_order[..5], &im.layers[..]);
        assert!(im.warnings.iter().any(|w| w.contains("„QRCODE“")), "{:?}", im.warnings);
        assert!(im.warnings.iter().any(|w| w.contains("Umrisse")));
        assert!(!im.warnings.iter().any(|w| w.contains("weicht")), "{:?}", im.warnings);
    }

    #[test]
    fn mirrored_and_rotated_objects_match_stored_bounds() {
        // 180° gedreht und gespiegelt (Scherung π) = waagerecht gespiegelt: x' = 10 − 0,5·x, y' = 20 + 0,5·y.
        // x/y ist die Ecke (10, 10) der unverdrehten Form → (5, 25); die Form reicht von x = −5 bis 5.
        let mut v = xcs();
        let d = &mut v["canvas"][0]["displays"][0];
        d["angle"] = json!(180);
        d["skew"] = json!({"x": std::f64::consts::PI, "y": 0});
        d["x"] = json!(5.0);
        d["y"] = json!(25.0);
        let im = import_xtool(&serde_json::to_vec(&v).unwrap()).unwrap();
        let p = by_name(&im.doc, "Pfad");
        assert!(p.transform.determinant() < 0.0, "gespiegelt");
        let b = p.bounds();
        assert!(close(b.min.x, -5.0) && close(b.max.x, 5.0) && close(b.min.y, 25.0) && close(b.height(), 5.0), "{b:?}");
        assert!(!im.warnings.iter().any(|w| w.contains("weicht")), "{:?}", im.warnings);
        // falsche Angaben in der Datei werden gemeldet
        let mut v = xcs();
        v["canvas"][0]["displays"][0]["width"] = json!(12.0);
        let im = import_xtool(&serde_json::to_vec(&v).unwrap()).unwrap();
        assert!(im.warnings.iter().any(|w| w == &format!("1× {S_MISMATCH}")), "{:?}", im.warnings);
    }

    #[test]
    fn studio_archive() {
        let png = {
            let mut out = Vec::new();
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(10, 5, image::Rgba([90, 90, 90, 255])))
                .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
                .unwrap();
            out
        };
        let project = json!({"projectName": "Probe", "activeCanvasId": "b", "activeDeviceId": "MD2-1",
            "extName": "S1", "modules": {"canvases": ["a", "b"]}});
        let canvas = |id: &str| {
            json!({"id": id, "title": format!("{{panel}}{id}"),
            "layerData": {"#000000": {"name": "#000000", "order": 1, "visible": true}}})
        };
        let displays_a = json!({"displays": [
            {"id": "pa", "type": "PATH", "x": 0, "y": 0, "width": 10, "height": 10, "scale": {"x": 1, "y": 1},
             "graphicX": 0, "graphicY": 0, "layerColor": "#000000", "groupTags": ["g"],
             "vectorRef": {"vectorHash": "h1", "bucketType": "svg", "originalField": "dPath"}}]});
        let displays_b = json!({"displays": [
            {"id": "pb", "type": "PATH", "x": 5, "y": 5, "width": 10, "height": 10, "scale": {"x": 1, "y": 1},
             "graphicX": 5, "graphicY": 5, "layerColor": "#000000",
             "vectorRef": {"vectorHash": "h1", "bucketType": "svg", "originalField": "dPath"}},
            {"id": "ib", "type": "BITMAP", "x": 20, "y": 0, "width": 10, "height": 5, "angle": 90,
             "skew": {"x": std::f64::consts::PI, "y": 0}, "scale": {"x": 1, "y": 1}, "offsetX": 20, "offsetY": 0,
             "originWidth": 10, "originHeight": 5, "resourcePath": "resources/x.png", "layerColor": "#000000",
             "customData": {"processingType": "BITMAP_ENGRAVING"}}]});
        let vectors = json!({"bucketType": "svg", "entries": {"h1": "M0 0 L10 0 L10 10 L0 10 Z"}});
        let device = json!({"processing": {
            "a": {"activeMode": "LASER_PLANE", "modes": {"LASER_PLANE": {"ignoredDisplayIds": [],
                "bindings": [{"baseProfileId": "p:cut", "displayIds": ["pa"]}]}}},
            "b": {"activeMode": "LASER_CYLINDER", "modes": {"LASER_CYLINDER": {"ignoredDisplayIds": ["pb"],
                "bindings": [{"baseProfileId": "p:score", "displayIds": ["pb"]}]}}}}});
        let profiles = json!({"profiles": {
            "p:cut": {"processingType": "VECTOR_CUTTING", "values": {"power": 100, "speed": 5}},
            "p:score": {"processingType": "VECTOR_ENGRAVING", "values": {"power": 10, "speed": 100}}}});
        let s = |v: &Value| serde_json::to_vec(v).unwrap();
        let files: Vec<(&str, Vec<u8>)> = vec![
            ("canvases/a.json", s(&canvas("a"))),
            ("canvases/a/displays-0.json", s(&displays_a)),
            ("canvases/b.json", s(&canvas("b"))),
            ("canvases/b/displays-0.json", s(&displays_b)),
            ("devices/device-MD2-1.json", s(&device)),
            ("profiles.json", s(&profiles)),
            ("project.json", s(&project)),
            ("resources/x.png", png),
            ("vectors/svg/data-0.json", s(&vectors)),
            (".format", b"v2".to_vec()),
        ];
        let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, d)| (*n, d.as_slice())).collect();
        let zip = zipread::write(&refs, true);
        let im = import_xtool(&zip).unwrap();
        assert_eq!(im.format, "xTool Studio");
        assert_eq!(im.device.as_deref(), Some("S1"));
        assert_eq!(im.doc.objects.len(), 3, "{:?}", im.warnings);
        // aktive Zeichenfläche „b“ bleibt an ihrer Stelle, „a“ liegt rechts daneben
        let pb = by_name(&im.doc, "Pfad");
        assert!(close(pb.bounds().min.x, 5.0));
        let img = by_name(&im.doc, "Bild");
        // gespiegelt + 90°: x-Achse des Bildes → +Y, y-Achse → +X
        let b = img.bounds();
        assert!(close(b.min.x, 20.0) && close(b.max.x, 25.0) && close(b.min.y, 0.0) && close(b.max.y, 10.0), "{b:?}");
        let pa = im.doc.objects.iter().filter(|o| o.name == "Pfad").map(|o| o.bounds().min.x).fold(0.0, f64::max);
        assert!(close(pa, 25.0 + CANVAS_GAP), "{pa}");
        let layer_of = |o: &vl_core::Object| &im.doc.layers[o.layer];
        assert!(!layer_of(pb).settings.output, "in xTool ausgenommen");
        assert!(layer_of(pb).name.starts_with("Gravieren"));
        assert_eq!(layer_of(img).settings.mode, LayerMode::Image);
        assert!(im.warnings[0].starts_with("Das Projekt hat 2 Zeichenflächen („b“, „a“)"), "{:?}", im.warnings);
        assert!(im.warnings.iter().any(|w| w.contains("LASER_CYLINDER")), "{:?}", im.warnings);
    }

    #[test]
    fn rejects_other_files() {
        assert!(import_xtool(b"{\"hallo\": 1}").is_err());
        assert!(import_xtool(b"kein json").is_err());
        assert!(import_xtool(&zipread::write(&[("a.txt", b"x")], false)).is_err());
        let empty = json!({"canvas": [{"id": "c", "displays": [{"type": "PATH", "dPath": "Q"}]}]});
        let e = import_xtool(&serde_json::to_vec(&empty).unwrap()).unwrap_err();
        assert!(e.starts_with(ERR_EMPTY) && e.contains(S_PATH), "{e}");
    }

    #[test]
    fn processing_types() {
        assert_eq!(kind_of(Some("VECTOR_CUTTING"), false, false), (Kind::Cut, None));
        assert_eq!(kind_of(None, true, false).0, Kind::Image);
        assert_eq!(kind_of(None, false, true).0, Kind::Fill);
        assert_eq!(kind_of(Some("BITMAP_ENGRAVING"), false, false).0, Kind::Fill);
        let (k, unknown) = kind_of(Some("INTAGLIO"), false, false);
        assert_eq!((k, unknown.as_deref()), (Kind::Line, Some("INTAGLIO")));
        assert_eq!(
            fill(W_PROCESSING, &["X", "Linie"]),
            "Bearbeitungsart „X“ aus xTool unbekannt – als „Linie“ übernommen."
        );
    }

    /// Prüft echte Projekte gegen die gespeicherten Umrandungen:
    /// `VL_XTOOL_DIR=… cargo test -p vl-io real_xtool_files -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_xtool_files() {
        let dir = std::env::var("VL_XTOOL_DIR").expect("VL_XTOOL_DIR setzen");
        let mut stack = vec![std::path::PathBuf::from(dir)];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().and_then(|e| e.to_str()).is_some_and(is_xtool_extension) {
                    let data = std::fs::read(&p).unwrap();
                    let start = std::time::Instant::now();
                    match import_xtool(&data) {
                        Ok(im) => {
                            let b = im.doc.bounds();
                            println!(
                                "{:?} ({}, {:?}): {} Objekte in {:?}, Ebenen {:?}, {:.1}×{:.1} mm bei ({:.1}, {:.1}), {:?}",
                                p.file_name().unwrap(),
                                im.format,
                                im.device,
                                im.doc.objects.len(),
                                start.elapsed(),
                                im.layers.iter().map(|&l| im.doc.layers[l].name.clone()).collect::<Vec<_>>(),
                                b.width(),
                                b.height(),
                                b.min.x,
                                b.min.y,
                                im.warnings
                            );
                        }
                        Err(e) => println!("{:?}: FEHLER {e}", p.file_name().unwrap()),
                    }
                }
            }
        }
    }
}
