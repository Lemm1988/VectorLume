//! 3D-Modelle (STL binär/ASCII, OBJ) als Tiefenbild: Blick entlang einer Achse, je Pixel die nächstgelegene
//! Oberfläche. Hell = hoch (wenig Leistung), dunkel = tief – zusammen mit dem Bildmodus „3D-Schichten“ oder
//! Graustufen wird daraus ein Relief.

use vl_core::RasterImage;

/// Dreiecksnetz in Modelleinheiten (STL/OBJ haben keine feste Einheit; meist mm).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub triangles: Vec<[[f32; 3]; 3]>,
}

/// Größere Tiefenbilder werden auf diese Kantenlänge und Pixelzahl begrenzt (Speicher, Rechenzeit).
pub const MAX_SIDE_PX: u32 = 8000;
pub const MAX_PIXELS: f64 = 25e6;

/// Blickrichtung: von wo auf das Modell geschaut wird.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Top,
    Bottom,
    Front,
    Back,
    Left,
    Right,
}

impl View {
    pub const ALL: [View; 6] = [View::Top, View::Bottom, View::Front, View::Back, View::Left, View::Right];

    /// Punkt in Bildkoordinaten: (rechts, oben, zum Betrachter).
    fn project(self, [x, y, z]: [f32; 3]) -> [f32; 3] {
        match self {
            View::Top => [x, y, z],
            View::Bottom => [-x, y, -z],
            View::Front => [x, z, -y],
            View::Back => [-x, z, y],
            View::Left => [-y, z, -x],
            View::Right => [y, z, x],
        }
    }
}

impl Mesh {
    /// Begrenzungsquader (min, max) in Modellkoordinaten; `None` ohne Dreiecke.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        bounds(self.triangles.iter().flatten().copied())
    }

    /// Ausdehnung in Bildrichtung (Breite, Höhe, Tiefe) für die Ansicht `view`.
    pub fn extent(&self, view: View) -> [f32; 3] {
        bounds(self.triangles.iter().flatten().map(|p| view.project(*p)))
            .map_or([0.0; 3], |(lo, hi)| [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]])
    }
}

fn bounds(points: impl Iterator<Item = [f32; 3]>) -> Option<([f32; 3], [f32; 3])> {
    points.fold(None, |acc, p| {
        let (mut lo, mut hi) = acc.unwrap_or((p, p));
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
        Some((lo, hi))
    })
}

/// Liest STL (binär oder ASCII) oder OBJ – erkannt am Inhalt, nicht an der Endung.
pub fn parse_model(data: &[u8]) -> Result<Mesh, String> {
    let mesh = if is_binary_stl(data) {
        parse_binary_stl(data)
    } else {
        let text =
            std::str::from_utf8(data).map_err(|_| "Unbekanntes 3D-Format (erwartet STL oder OBJ).".to_string())?;
        if text.trim_start().starts_with("solid") { parse_ascii_stl(text)? } else { parse_obj(text)? }
    };
    if mesh.triangles.is_empty() {
        return Err("Die Datei enthält keine Dreiecke.".into());
    }
    if mesh.triangles.iter().flatten().flatten().any(|v| !v.is_finite()) {
        return Err("Die Datei enthält ungültige Koordinaten.".into());
    }
    Ok(mesh)
}

/// Binäre STL: 80 Byte Kopf, Anzahl, je Dreieck 50 Byte. Manche Programme beginnen auch binäre Dateien mit
/// „solid“, daher entscheidet die Länge.
fn is_binary_stl(data: &[u8]) -> bool {
    data.len() >= 84 && {
        let n = u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as usize;
        n.checked_mul(50).and_then(|b| b.checked_add(84)) == Some(data.len())
    }
}

fn parse_binary_stl(data: &[u8]) -> Mesh {
    let f = |b: &[u8]| f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let triangles = data[84..]
        .as_chunks::<50>()
        .0
        .iter()
        .map(|t| {
            let v = |i: usize| {
                let o = 12 + 12 * i;
                [f(&t[o..]), f(&t[o + 4..]), f(&t[o + 8..])]
            };
            [v(0), v(1), v(2)]
        })
        .collect();
    Mesh { triangles }
}

fn parse_ascii_stl(text: &str) -> Result<Mesh, String> {
    let mut triangles = Vec::new();
    let mut corners = Vec::with_capacity(3);
    for (n, line) in text.lines().enumerate() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("vertex") => corners.push(vec3(words, n + 1)?),
            Some("endloop") => {
                // Vielecke als Fächer zerlegen (die Norm verlangt Dreiecke, manche Exporte halten sich nicht daran)
                for i in 1..corners.len().saturating_sub(1) {
                    triangles.push([corners[0], corners[i], corners[i + 1]]);
                }
                corners.clear();
            }
            _ => {}
        }
    }
    Ok(Mesh { triangles })
}

fn parse_obj(text: &str) -> Result<Mesh, String> {
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut triangles = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("v") => vertices.push(vec3(words, n + 1)?),
            Some("f") => {
                let idx = words
                    .map(|w| {
                        // „v“, „v/vt“, „v//vn“ oder „v/vt/vn“; negative Indizes zählen vom Ende
                        let i: i64 = w.split('/').next().unwrap_or("").parse().map_err(|_| bad_line(n + 1))?;
                        let i = if i < 0 { vertices.len() as i64 + i } else { i - 1 };
                        usize::try_from(i).ok().filter(|i| *i < vertices.len()).ok_or_else(|| bad_line(n + 1))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                for i in 1..idx.len().saturating_sub(1) {
                    triangles.push([vertices[idx[0]], vertices[idx[i]], vertices[idx[i + 1]]]);
                }
            }
            _ => {}
        }
    }
    if vertices.is_empty() {
        return Err("Unbekanntes 3D-Format (erwartet STL oder OBJ).".into());
    }
    Ok(Mesh { triangles })
}

fn bad_line(n: usize) -> String {
    format!("Ungültige Angabe in Zeile {n}.")
}

fn vec3<'a>(mut words: impl Iterator<Item = &'a str>, n: usize) -> Result<[f32; 3], String> {
    let mut c = || words.next().and_then(|w| w.parse::<f32>().ok()).ok_or_else(|| bad_line(n));
    Ok([c()?, c()?, c()?])
}

/// Einstellungen für das Tiefenbild.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthOptions {
    pub view: View,
    /// Breite des Bildes in mm (das Modell wird gleichmäßig skaliert).
    pub width_mm: f64,
    /// Pixel je mm.
    pub px_per_mm: f64,
    /// Hoch = dunkel (z. B. für Gussformen und Stempel, die spiegelverkehrt in die Tiefe gehen).
    pub invert: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DepthMap {
    pub image: RasterImage,
    pub width_mm: f64,
    pub height_mm: f64,
}

/// Pixelgröße, die `opts` ergeben würde (nach Begrenzung auf [`MAX_SIDE_PX`] und [`MAX_PIXELS`]).
pub fn depth_size(mesh: &Mesh, opts: &DepthOptions) -> (u32, u32) {
    let [w, h, _] = mesh.extent(opts.view);
    if w <= 0.0 || opts.width_mm <= 0.0 {
        return (0, 0);
    }
    let height_mm = opts.width_mm * (h / w) as f64;
    let px = |mm: f64| (mm * opts.px_per_mm).round().max(1.0);
    let (pw, ph) = (px(opts.width_mm), px(height_mm));
    let f = (MAX_SIDE_PX as f64 / pw.max(ph)).min((MAX_PIXELS / (pw * ph)).sqrt()).min(1.0);
    (((pw * f).round() as u32).max(1), ((ph * f).round() as u32).max(1))
}

/// Rendert das Tiefenbild: Weiß außerhalb des Modells, sonst Grauwert nach Höhe (höchste Stelle weiß, tiefste
/// sichtbare Stelle schwarz; mit `invert` umgekehrt).
pub fn depth_map(mesh: &Mesh, opts: &DepthOptions) -> Result<DepthMap, String> {
    let projected: Vec<[[f32; 3]; 3]> = mesh
        .triangles
        .iter()
        .map(|t| [opts.view.project(t[0]), opts.view.project(t[1]), opts.view.project(t[2])])
        .collect();
    let Some((lo, hi)) = bounds(projected.iter().flatten().copied()) else {
        return Err("Das Modell enthält keine Dreiecke.".into());
    };
    let (w, h) = (hi[0] - lo[0], hi[1] - lo[1]);
    if w <= 0.0 || h <= 0.0 {
        return Err("Das Modell ist aus dieser Richtung flach – bitte eine andere Ansicht wählen.".into());
    }
    let (cols, rows) = depth_size(mesh, opts);
    let height_mm = opts.width_mm * (h / w) as f64;
    // Pixel je Modelleinheit in beiden Richtungen (nach der Begrenzung nicht mehr genau `px_per_mm`)
    let (sx, sy) = (cols as f32 / w, rows as f32 / h);
    let mut zbuf = vec![f32::NEG_INFINITY; cols as usize * rows as usize];
    for t in &projected {
        // Pixelkoordinaten: x nach rechts, y nach unten (Zeile 0 = oberer Bildrand)
        let p = t.map(|[u, v, d]| [(u - lo[0]) * sx, (hi[1] - v) * sy, d]);
        raster_triangle(&p, cols, rows, &mut zbuf);
    }
    let (dmin, dmax) = zbuf
        .iter()
        .filter(|d| d.is_finite())
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), d| (a.min(*d), b.max(*d)));
    let range = (dmax - dmin).max(f32::EPSILON);
    let pixels = zbuf
        .iter()
        .map(|d| {
            if !d.is_finite() {
                return 255;
            }
            let t = (d - dmin) / range;
            let t = if opts.invert { 1.0 - t } else { t };
            (t * 255.0).round() as u8
        })
        .collect();
    Ok(DepthMap { image: RasterImage::new(cols, rows, pixels), width_mm: opts.width_mm, height_mm })
}

/// Füllt alle Pixel, deren Mitte im Dreieck liegt, mit der größten Tiefe (dem Betrachter am nächsten).
fn raster_triangle(p: &[[f32; 3]; 3], cols: u32, rows: u32, zbuf: &mut [f32]) {
    let [a, b, c] = *p;
    let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    if area.abs() < 1e-12 {
        return;
    }
    let x0 = a[0].min(b[0]).min(c[0]).floor().max(0.0) as u32;
    let x1 = (a[0].max(b[0]).max(c[0]).ceil() as u32).min(cols);
    let y0 = a[1].min(b[1]).min(c[1]).floor().max(0.0) as u32;
    let y1 = (a[1].max(b[1]).max(c[1]).ceil() as u32).min(rows);
    let edge = |p: [f32; 3], q: [f32; 3], x: f32, y: f32| (q[0] - p[0]) * (y - p[1]) - (q[1] - p[1]) * (x - p[0]);
    for y in y0..y1 {
        let py = y as f32 + 0.5;
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            // baryzentrische Gewichte; Kanten zählen dazu, damit an Nahtstellen keine Lücken bleiben
            let (w0, w1, w2) = (edge(b, c, px, py) / area, edge(c, a, px, py) / area, edge(a, b, px, py) / area);
            if w0 < -1e-6 || w1 < -1e-6 || w2 < -1e-6 {
                continue;
            }
            let d = w0 * a[2] + w1 * b[2] + w2 * c[2];
            let slot = &mut zbuf[y as usize * cols as usize + x as usize];
            if d > *slot {
                *slot = d;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pyramide über einer 10×10-Grundfläche, Spitze 5 hoch (vier Seiten, Boden aus zwei Dreiecken).
    fn pyramid() -> Mesh {
        let (a, b, c, d, top) =
            ([0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0], [0.0, 10.0, 0.0], [5.0, 5.0, 5.0]);
        Mesh { triangles: vec![[a, b, top], [b, c, top], [c, d, top], [d, a, top], [a, c, b], [a, d, c]] }
    }

    fn to_ascii(m: &Mesh) -> String {
        let mut s = String::from("solid test\n");
        for t in &m.triangles {
            s.push_str("facet normal 0 0 0\n outer loop\n");
            for v in t {
                s.push_str(&format!("  vertex {} {} {}\n", v[0], v[1], v[2]));
            }
            s.push_str(" endloop\nendfacet\n");
        }
        s + "endsolid test\n"
    }

    fn to_binary(m: &Mesh) -> Vec<u8> {
        // Kopf beginnt absichtlich mit „solid“ – wie bei manchen Exporten
        let mut out = b"solid binary".to_vec();
        out.resize(80, 0);
        out.extend((m.triangles.len() as u32).to_le_bytes());
        for t in &m.triangles {
            out.extend([0u8; 12]);
            for v in t {
                for c in v {
                    out.extend(c.to_le_bytes());
                }
            }
            out.extend([0u8; 2]);
        }
        out
    }

    #[test]
    fn reads_stl_binary_ascii_and_obj() {
        let m = pyramid();
        assert_eq!(parse_model(&to_binary(&m)).unwrap(), m);
        assert_eq!(parse_model(to_ascii(&m).as_bytes()).unwrap(), m);
        // OBJ mit Viereck als Boden (wird zum Fächer), Texturindizes und negativem Index
        let obj = "# Pyramide\nv 0 0 0\nv 10 0 0\nv 10 10 0\nv 0 10 0\nv 5 5 5\n\
                   f 1/1 2/2 5/3\nf 2 3 5\nf 3//1 4//1 5//1\nf 4 1 -1\nf 1 4 3 2\n";
        let o = parse_model(obj.as_bytes()).unwrap();
        assert_eq!(o.triangles.len(), 6);
        assert_eq!(o.bounds(), Some(([0.0, 0.0, 0.0], [10.0, 10.0, 5.0])));
        assert!(parse_model(b"hallo").is_err());
        assert!(parse_model(b"solid leer\nendsolid\n").is_err());
        assert!(parse_model(b"v 0 0 0\nf 1 2 9\n").is_err(), "Index außerhalb");
    }

    #[test]
    fn depth_map_of_pyramid() {
        let opts = DepthOptions { view: View::Top, width_mm: 20.0, px_per_mm: 5.0, invert: false };
        let d = depth_map(&pyramid(), &opts).unwrap();
        assert_eq!((d.image.width, d.image.height), (100, 100));
        assert_eq!((d.width_mm, d.height_mm), (20.0, 20.0));
        // Mitte hell (Spitze), Rand dunkel (Grundfläche), monoton nach außen
        assert!(d.image.get(50, 50) >= 250, "{}", d.image.get(50, 50));
        assert!(d.image.get(1, 50) <= 6 && d.image.get(50, 98) <= 6);
        let row: Vec<u8> = (50..100).map(|x| d.image.get(x, 50)).collect();
        assert!(row.windows(2).all(|w| w[0] >= w[1]), "{row:?}");
        // Halbe Höhe auf halbem Weg
        assert!((d.image.get(75, 50) as i32 - 128).abs() <= 8, "{}", d.image.get(75, 50));

        let inv = depth_map(&pyramid(), &DepthOptions { invert: true, ..opts }).unwrap();
        assert!(inv.image.get(50, 50) <= 5 && inv.image.get(1, 50) >= 249);

        // Von vorne: Dreieck mit Spitze oben; außerhalb weiß, Bild halb so hoch wie breit
        let front = depth_map(&pyramid(), &DepthOptions { view: View::Front, ..opts }).unwrap();
        assert_eq!((front.image.width, front.image.height), (100, 50));
        assert_eq!(front.image.get(2, 2), 255, "außerhalb des Modells");
        assert!(front.image.get(50, 48) < 255);
        // Von unten: nur die flache Grundfläche – einheitlich
        let bottom = depth_map(&pyramid(), &DepthOptions { view: View::Bottom, ..opts }).unwrap();
        assert_eq!(bottom.image.get(10, 10), bottom.image.get(90, 90));
    }

    #[test]
    fn size_limit_and_flat_view() {
        let opts = DepthOptions { view: View::Top, width_mm: 2000.0, px_per_mm: 10.0, invert: false };
        assert_eq!(depth_size(&pyramid(), &opts), (5000, 5000), "höchstens 25 Mio. Pixel");
        let strip = Mesh { triangles: vec![[[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [0.0, 1.0, 1.0]]] };
        assert_eq!(depth_size(&strip, &opts).0, MAX_SIDE_PX, "höchstens 8000 Pixel Kantenlänge");
        let flat = Mesh { triangles: vec![[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]] };
        assert!(depth_map(&flat, &DepthOptions { view: View::Front, ..opts }).is_err());
    }
}
