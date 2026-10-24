//! Arbeitsfläche: Darstellung von Bett, Objekten und Vorschau sowie Maus-Interaktion.

use crate::app::{TextDialog, Tool, VectorLumeApp};
use crate::theme;
use egui::epaint::Vertex;
use egui::{
    Color32, CursorIcon, Mesh, PointerButton, Pos2, Sense, Shape, Stroke, TextureHandle, TextureId, Vec2, pos2, vec2,
};
use std::collections::HashMap;
use std::sync::Arc;
use vl_cam::{BlockKind, Job, Op};
use vl_core::{Affine, ImageObject, Object, ObjectId, Point, Polyline, Rect as DRect, Shape as DocShape, VectorPath};

const RULER: f32 = 22.0;
const HANDLE: f32 = 5.0;
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 400.0;

// ------------------------------------------------------------------ Ansicht

pub struct View {
    pub zoom: f32,
    pub pan: Vec2,
    fit_request: Option<Option<DRect>>,
    initialized: bool,
    /// Mausposition im Dokument (für die Statusleiste).
    pub cursor: Option<Point>,
    /// Zoom aus Menü/Tastatur, wird im nächsten Frame um die Bildmitte angewendet.
    pending_zoom: f32,
    /// Solange der Benutzer die Ansicht nicht verändert hat, wird bei Größenänderung neu eingepasst.
    user_moved: bool,
    last_size: Vec2,
}

impl Default for View {
    fn default() -> Self {
        Self {
            zoom: 2.0,
            pan: Vec2::ZERO,
            fit_request: Some(None),
            initialized: false,
            cursor: None,
            pending_zoom: 1.0,
            user_moved: false,
            last_size: Vec2::ZERO,
        }
    }
}

impl View {
    /// `None` = ganze Arbeitsfläche.
    pub fn request_fit(&mut self, r: Option<DRect>) {
        self.fit_request = Some(r);
        self.user_moved = false;
    }

    pub fn zoom_by(&mut self, factor: f32) {
        self.pending_zoom *= factor;
        self.user_moved = true;
    }
}

#[derive(Clone, Copy)]
struct ViewTf {
    origin: Pos2,
    zoom: f32,
}

impl ViewTf {
    fn to_screen(self, p: Point) -> Pos2 {
        pos2(self.origin.x + p.x as f32 * self.zoom, self.origin.y + p.y as f32 * self.zoom)
    }

    fn to_doc(self, p: Pos2) -> Point {
        Point::new(((p.x - self.origin.x) / self.zoom) as f64, ((p.y - self.origin.y) / self.zoom) as f64)
    }

    fn affine(&self) -> Affine {
        let z = self.zoom as f64;
        Affine([z, 0.0, 0.0, z, self.origin.x as f64, self.origin.y as f64])
    }

    fn rect(&self, r: &DRect) -> egui::Rect {
        egui::Rect::from_two_pos(self.to_screen(r.min), self.to_screen(r.max))
    }
}

fn p2(p: Point) -> Pos2 {
    pos2(p.x as f32, p.y as f32)
}

// ------------------------------------------------------------------ Caches

struct CachedGeom {
    scale: f64,
    key: (usize, usize),
    polys: Arc<Vec<Polyline>>,
    bounds: DRect,
}

/// Abgeflachte Pfade im lokalen Objektsystem (werden nur bei Formänderung neu berechnet).
#[derive(Default)]
pub struct GeomCache {
    map: HashMap<ObjectId, CachedGeom>,
}

impl GeomCache {
    pub fn clear(&mut self) {
        self.map.clear();
    }

    fn local(&mut self, obj: &Object) -> Option<(Arc<Vec<Polyline>>, DRect)> {
        let DocShape::Path(path) = &obj.shape else { return None };
        let scale = obj.transform.mean_scale();
        let key = (path.subpaths.len(), path.segment_count());
        let valid =
            self.map.get(&obj.id).is_some_and(|c| c.key == key && scale / c.scale < 2.0 && c.scale / scale < 2.0);
        if !valid {
            let polys = path.flatten(&Affine::IDENTITY, 0.02 / scale);
            let bounds = vl_core::path::bounds_of(&polys);
            self.map.insert(obj.id, CachedGeom { scale, key, polys: Arc::new(polys), bounds });
        }
        self.map.get(&obj.id).map(|c| (c.polys.clone(), c.bounds))
    }

    fn retain(&mut self, ids: impl Fn(&ObjectId) -> bool) {
        self.map.retain(|k, _| ids(k));
    }
}

#[derive(Default)]
pub struct TextureCache {
    images: HashMap<(usize, [u64; 3]), (TextureHandle, u64)>,
    preview: Vec<TextureHandle>,
    preview_generation: u64,
    frame: u64,
}

/// Höchstens so viele Pixel pro Seite in einer Textur (Grafikkarten-Grenzen).
const MAX_TEXTURE_SIDE: usize = 4096;

fn downsample_step(w: usize, h: usize) -> usize {
    w.max(h).div_ceil(MAX_TEXTURE_SIDE).max(1)
}

impl TextureCache {
    fn image(&mut self, ctx: &egui::Context, img: &ImageObject) -> TextureId {
        let key = (
            Arc::as_ptr(&img.image) as usize,
            [img.adjust.brightness.to_bits(), img.adjust.contrast.to_bits(), img.adjust.gamma.to_bits()],
        );
        let frame = self.frame;
        if let Some((tex, used)) = self.images.get_mut(&key) {
            *used = frame;
            return tex.id();
        }
        let src = &img.image;
        let (w, h) = (src.width as usize, src.height as usize);
        let step = downsample_step(w, h);
        let (tw, th) = (w.div_ceil(step), h.div_ceil(step));
        let lut: Vec<u8> = (0..=255u8).map(|v| (img.adjust.apply(v as f32 / 255.0) * 255.0).round() as u8).collect();
        let pixels = (0..th)
            .flat_map(|y| (0..tw).map(move |x| (x, y)))
            .map(|(x, y)| lut[src.pixels[(y * step).min(h - 1) * w + (x * step).min(w - 1)] as usize]);
        let color = egui::ColorImage::from_gray_iter([tw, th], pixels);
        let tex = ctx.load_texture("bild", color, egui::TextureOptions::LINEAR);
        let id = tex.id();
        self.images.insert(key, (tex, frame));
        id
    }

    fn preview(&mut self, ctx: &egui::Context, job: &Job, generation: u64) -> &[TextureHandle] {
        if self.preview_generation != generation {
            self.preview_generation = generation;
            self.preview = job
                .toolpath
                .rasters
                .iter()
                .map(|r| {
                    let r = &r.result;
                    let step = downsample_step(r.cols, r.rows);
                    let (tw, th) = (r.cols.div_ceil(step), r.rows.div_ceil(step));
                    let mut rgba = Vec::with_capacity(tw * th * 4);
                    for y in 0..th {
                        for x in 0..tw {
                            let v = r.get((x * step).min(r.cols - 1), (y * step).min(r.rows - 1));
                            rgba.extend_from_slice(&[255, 255, 255, (v.clamp(0.0, 1.0) * 255.0) as u8]);
                        }
                    }
                    let img = egui::ColorImage::from_rgba_unmultiplied([tw, th], &rgba);
                    ctx.load_texture("vorschau", img, egui::TextureOptions::NEAREST)
                })
                .collect();
        }
        &self.preview
    }

    fn end_frame(&mut self) {
        self.frame += 1;
        let frame = self.frame;
        self.images.retain(|_, (_, used)| frame - *used < 600);
    }
}

fn textured_quad(painter: &egui::Painter, tex: TextureId, corners: [Pos2; 4], tint: Color32) {
    let mut mesh = Mesh::with_texture(tex);
    let uvs = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
    for i in 0..4 {
        mesh.vertices.push(Vertex { pos: corners[i], uv: uvs[i], color: tint });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
}

// ------------------------------------------------------------------ Interaktion

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Handle {
    /// 0 = oben links, 1 = oben rechts, 2 = unten rechts, 3 = unten links
    Corner(u8),
    /// 0 = oben, 1 = rechts, 2 = unten, 3 = links
    Edge(u8),
    Rotate,
}

pub enum Drag {
    None,
    Pan,
    Move { start: Point, originals: Vec<(ObjectId, Affine)> },
    Scale { handle: Handle, bounds: DRect, originals: Vec<(ObjectId, Affine)> },
    Rotate { center: Point, start_angle: f64, originals: Vec<(ObjectId, Affine)> },
    Band { start: Point, additive: bool },
    Create { tool: Tool, start: Point },
}

impl Drag {
    pub fn is_idle(&self) -> bool {
        matches!(self, Drag::None)
    }
}

fn handle_positions(r: egui::Rect) -> Vec<(Handle, Pos2)> {
    let c = r.center();
    vec![
        (Handle::Corner(0), r.left_top()),
        (Handle::Corner(1), r.right_top()),
        (Handle::Corner(2), r.right_bottom()),
        (Handle::Corner(3), r.left_bottom()),
        (Handle::Edge(0), pos2(c.x, r.top())),
        (Handle::Edge(1), pos2(r.right(), c.y)),
        (Handle::Edge(2), pos2(c.x, r.bottom())),
        (Handle::Edge(3), pos2(r.left(), c.y)),
        (Handle::Rotate, pos2(c.x, r.top() - 24.0)),
    ]
}

fn nice_step(min_mm: f64) -> f64 {
    for s in [0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0] {
        if s >= min_mm {
            return s;
        }
    }
    1000.0
}

fn snap_angle(a: f64, step_deg: f64) -> f64 {
    let s = step_deg.to_radians();
    (a / s).round() * s
}

impl VectorLumeApp {
    fn snap(&self, p: Point) -> Point {
        if !self.settings.snap_to_grid {
            return p;
        }
        let g = self.settings.grid_snap_mm.max(0.01);
        Point::new((p.x / g).round() * g, (p.y / g).round() * g)
    }

    fn originals(&self) -> Vec<(ObjectId, Affine)> {
        self.doc.objects.iter().filter(|o| self.selection.contains(&o.id)).map(|o| (o.id, o.transform)).collect()
    }

    fn apply_originals(&mut self, originals: &[(ObjectId, Affine)], t: &Affine) {
        for (id, orig) in originals {
            if let Some(o) = self.doc.get_mut(*id) {
                o.transform = orig.then(t);
            }
        }
    }

    /// Oberstes Objekt unter dem Punkt (Toleranz in mm).
    fn hit_test(&mut self, p: Point, tol: f64) -> Option<ObjectId> {
        for idx in (0..self.doc.objects.len()).rev() {
            let obj = &self.doc.objects[idx];
            if !self.doc.layers[obj.layer].selectable() {
                continue;
            }
            match &obj.shape {
                DocShape::Image(img) => {
                    if let Some(inv) = obj.transform.inverse() {
                        if img.local_rect().expanded(tol).contains(inv.apply(p)) {
                            return Some(obj.id);
                        }
                    }
                }
                DocShape::Path(_) => {
                    let Some((polys, bounds)) = self.geom.local(obj) else { continue };
                    if !obj.transform.transform_rect(&bounds).expanded(tol).contains(p) {
                        continue;
                    }
                    let Some(inv) = obj.transform.inverse() else { continue };
                    let local = inv.apply(p);
                    let local_tol = tol / obj.transform.mean_scale();
                    if vl_core::path::point_in_polylines(local, &polys)
                        || polys.iter().any(|pl| pl.distance_to(local) <= local_tol)
                    {
                        return Some(obj.id);
                    }
                }
            }
        }
        None
    }

    fn create_shape(&mut self, tool: Tool, a: Point, b: Point, shift: bool) {
        let r = DRect::from_points(a, b);
        let o = &self.tool_options;
        let (name, path) = match tool {
            Tool::Rect => {
                let (w, h) = if shift {
                    (r.width().max(r.height()), r.width().max(r.height()))
                } else {
                    (r.width(), r.height())
                };
                let x = if b.x < a.x { a.x - w } else { a.x };
                let y = if b.y < a.y { a.y - h } else { a.y };
                ("Rechteck", VectorPath::rect(x, y, w, h, o.corner_radius))
            }
            Tool::Ellipse => {
                let (w, h) = if shift {
                    (r.width().max(r.height()), r.width().max(r.height()))
                } else {
                    (r.width(), r.height())
                };
                let x = if b.x < a.x { a.x - w } else { a.x };
                let y = if b.y < a.y { a.y - h } else { a.y };
                ("Ellipse", VectorPath::ellipse(Point::new(x + w / 2.0, y + h / 2.0), w / 2.0, h / 2.0))
            }
            Tool::Line => {
                let mut end = b;
                if shift {
                    let d = b - a;
                    let ang = snap_angle(d.y.atan2(d.x), 45.0);
                    end = a + Point::new(ang.cos(), ang.sin()) * d.length();
                }
                ("Linie", VectorPath::line(a, end))
            }
            Tool::Polygon => ("Vieleck", VectorPath::regular_polygon(a, a.distance(b), o.polygon_sides)),
            Tool::Star => ("Stern", VectorPath::star(a, a.distance(b), a.distance(b) * o.star_inner, o.star_points)),
            Tool::Select | Tool::Text => return,
        };
        let size = path.bounds(&Affine::IDENTITY);
        if size.width() < 0.2 && size.height() < 0.2 {
            return;
        }
        self.checkpoint();
        let id = self.doc.add_path(name, self.current_layer, path);
        self.selection = vec![id];
        self.mark_changed();
        self.tool = Tool::Select;
    }

    pub fn canvas_ui(&mut self, ui: &mut egui::Ui) {
        let pal = theme::of(ui);
        let dark = ui.visuals().dark_mode;
        let ctx = ui.ctx().clone();
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        painter.rect_filled(rect, 0.0, pal.canvas);
        let work = egui::Rect::from_min_max(rect.min + vec2(RULER, RULER), rect.max);
        let profile = self.settings.profile.clone();
        let bed = profile.bed_rect();

        // Einpassen (auch nach Größenänderung des Fensters, solange die Ansicht unverändert ist)
        if self.view.last_size != work.size() {
            if !self.view.user_moved && self.view.fit_request.is_none() {
                self.view.fit_request = Some(None);
            }
            self.view.last_size = work.size();
        }
        if let Some(target) = self.view.fit_request.take().or(if self.view.initialized { None } else { Some(None) }) {
            let r = target.filter(|r| !r.is_empty()).unwrap_or(bed);
            let margin = 0.06 * r.width().max(r.height()).max(10.0);
            let r = r.expanded(margin);
            let zoom =
                (work.width() / r.width() as f32).min(work.height() / r.height() as f32).clamp(MIN_ZOOM, MAX_ZOOM);
            self.view.zoom = zoom;
            let size = vec2(r.width() as f32 * zoom, r.height() as f32 * zoom);
            self.view.pan = (work.size() - size) / 2.0 - vec2(r.min.x as f32 * zoom, r.min.y as f32 * zoom);
            self.view.initialized = true;
        }

        // Zoom (Mausrad) und Verschieben (mittlere Maustaste oder Leertaste + ziehen)
        let space = ui.input(|i| i.key_down(egui::Key::Space)) && !ctx.egui_wants_keyboard_input();
        if let Some(hover) = resp.hover_pos() {
            let (scroll, zoom_delta) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = (scroll * 0.0018).exp() * zoom_delta;
            if (factor - 1.0).abs() > 1e-4 {
                let tf = ViewTf { origin: work.min + self.view.pan, zoom: self.view.zoom };
                let doc = tf.to_doc(hover);
                let new_zoom = (self.view.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
                let origin = hover - vec2(doc.x as f32 * new_zoom, doc.y as f32 * new_zoom);
                self.view.zoom = new_zoom;
                self.view.pan = origin - work.min;
                self.view.user_moved = true;
            }
        }
        if self.view.pending_zoom != 1.0 {
            let center = work.center();
            let tf = ViewTf { origin: work.min + self.view.pan, zoom: self.view.zoom };
            let doc = tf.to_doc(center);
            let new_zoom = (self.view.zoom * self.view.pending_zoom).clamp(MIN_ZOOM, MAX_ZOOM);
            self.view.zoom = new_zoom;
            self.view.pan = center - vec2(doc.x as f32 * new_zoom, doc.y as f32 * new_zoom) - work.min;
            self.view.pending_zoom = 1.0;
        }
        if resp.dragged_by(PointerButton::Middle) || (space && resp.dragged_by(PointerButton::Primary)) {
            self.view.pan += resp.drag_delta();
            self.view.user_moved = true;
            ctx.set_cursor_icon(CursorIcon::Grabbing);
        } else if space && resp.hovered() {
            ctx.set_cursor_icon(CursorIcon::Grab);
        }

        let tf = ViewTf { origin: work.min + self.view.pan, zoom: self.view.zoom };
        let vt = tf.affine();
        let clip = painter.with_clip_rect(work);
        self.view.cursor = resp.hover_pos().filter(|p| work.contains(*p)).map(|p| tf.to_doc(p));

        // Bett und Raster
        let bed_screen = tf.rect(&bed);
        clip.rect_filled(bed_screen, 2.0, pal.bed);
        let minor = nice_step(8.0 / tf.zoom as f64);
        let major = nice_step(minor * 4.9);
        let visible = DRect::from_points(tf.to_doc(work.min), tf.to_doc(work.max));
        let draw_lines = |step: f64, color: Color32| {
            if step * tf.zoom as f64 >= 6.0 {
                let x0 = (visible.min.x.max(0.0) / step).ceil() as i64;
                let x1 = (visible.max.x.min(bed.max.x) / step).floor() as i64;
                for i in x0..=x1 {
                    let x = i as f64 * step;
                    clip.line_segment(
                        [tf.to_screen(Point::new(x, 0.0)), tf.to_screen(Point::new(x, bed.max.y))],
                        Stroke::new(1.0, color),
                    );
                }
                let y0 = (visible.min.y.max(0.0) / step).ceil() as i64;
                let y1 = (visible.max.y.min(bed.max.y) / step).floor() as i64;
                for i in y0..=y1 {
                    let y = i as f64 * step;
                    clip.line_segment(
                        [tf.to_screen(Point::new(0.0, y)), tf.to_screen(Point::new(bed.max.x, y))],
                        Stroke::new(1.0, color),
                    );
                }
            }
        };
        draw_lines(minor, pal.grid_minor);
        draw_lines(major, pal.grid_major);
        clip.rect_stroke(bed_screen, 2.0, Stroke::new(1.0, pal.grid_major), egui::StrokeKind::Outside);
        // Nullpunkt der Maschine markieren
        let origin_doc = profile.machine_to_doc(Point::ZERO);
        let o = tf.to_screen(origin_doc);
        clip.circle_filled(o, 4.0, pal.accent);

        // Objekte
        let preview_active = self.preview.active;
        let ppm = 1.0 / tf.zoom as f64;
        let objects_alpha = if preview_active { 0.22 } else { 1.0 };
        for idx in 0..self.doc.objects.len() {
            let obj = &self.doc.objects[idx];
            let layer = &self.doc.layers[obj.layer];
            if !layer.visible {
                continue;
            }
            let mut color = theme::layer_color(layer.color);
            if !dark && layer.color.iter().all(|&c| c > 215) {
                color = Color32::from_gray(140);
            }
            let alpha = objects_alpha * if layer.settings.output { 1.0 } else { 0.35 };
            let color = color.gamma_multiply(alpha);
            let selected = self.selection.contains(&obj.id);
            match &obj.shape {
                DocShape::Image(img) => {
                    let corners = img.local_rect().corners().map(|c| p2(obj.transform.then(&vt).apply(c)));
                    let screen_bb = egui::Rect::from_points(&corners);
                    if !screen_bb.intersects(work) {
                        continue;
                    }
                    let tex = self.textures.image(&ctx, img);
                    textured_quad(&clip, tex, corners, Color32::WHITE.gamma_multiply(objects_alpha));
                    let mut pts = corners.to_vec();
                    pts.push(corners[0]);
                    clip.add(Shape::line(pts, Stroke::new(if selected { 1.5 } else { 1.0 }, color)));
                }
                DocShape::Path(_) => {
                    let Some((polys, bounds)) = self.geom.local(obj) else { continue };
                    let full = obj.transform.then(&vt);
                    let screen_bb = full.transform_rect(&bounds);
                    if !egui::Rect::from_min_max(p2(screen_bb.min), p2(screen_bb.max)).expand(2.0).intersects(work) {
                        continue;
                    }
                    let stroke = Stroke::new(
                        if selected { 1.8 } else { 1.2 },
                        if selected && !preview_active { pal.selection } else { color },
                    );
                    for poly in polys.iter() {
                        let pts: Vec<Pos2> = poly.points.iter().map(|p| p2(full.apply(*p))).collect();
                        if pts.len() < 2 {
                            continue;
                        }
                        clip.add(if poly.closed { Shape::closed_line(pts, stroke) } else { Shape::line(pts, stroke) });
                    }
                }
            }
        }

        if preview_active {
            if let Some(job) = self.preview.job.clone() {
                let generation = self.preview.texture_generation;
                let textures: Vec<TextureId> =
                    self.textures.preview(&ctx, &job, generation).iter().map(|t| t.id()).collect();
                self.draw_job(&clip, &tf, pal, dark, &job, self.preview.progress, &textures, true);
            }
        }
        if self.test.open && self.test.show_on_canvas {
            if let Some(job) = self.test.job.clone() {
                self.draw_job(&clip, &tf, pal, dark, &job, 1.0, &[], false);
            }
        }

        // Laserkopf und Benutzer-Ursprung
        if let Some(head) = self.machine.head_position() {
            let p = tf.to_screen(profile.machine_to_doc(head));
            let c = pal.danger;
            clip.circle_stroke(p, 7.0, Stroke::new(1.5, c));
            clip.line_segment([p - vec2(11.0, 0.0), p + vec2(11.0, 0.0)], Stroke::new(1.0, c));
            clip.line_segment([p - vec2(0.0, 11.0), p + vec2(0.0, 11.0)], Stroke::new(1.0, c));
        }
        if self.settings.job.start_from == vl_cam::StartFrom::UserOrigin {
            let p = tf.to_screen(profile.machine_to_doc(self.settings.job.user_origin));
            clip.circle_stroke(p, 5.0, Stroke::new(1.5, pal.success));
            clip.text(
                p + vec2(7.0, -7.0),
                egui::Align2::LEFT_BOTTOM,
                "U",
                egui::FontId::proportional(11.0),
                pal.success,
            );
        }

        // Auswahlrahmen und Griffe
        let sel_bounds = self.selection_bounds();
        let sel_screen = (!sel_bounds.is_empty() && !preview_active).then(|| tf.rect(&sel_bounds));
        if let Some(r) = sel_screen {
            let dash = Shape::dashed_line(
                &[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()],
                Stroke::new(1.0, pal.selection),
                5.0,
                4.0,
            );
            clip.extend(dash);
            if self.tool == Tool::Select {
                for (h, p) in handle_positions(r) {
                    match h {
                        Handle::Rotate => {
                            clip.line_segment([pos2(r.center().x, r.top()), p], Stroke::new(1.0, pal.selection));
                            clip.circle_filled(p, HANDLE + 0.5, pal.panel);
                            clip.circle_stroke(p, HANDLE + 0.5, Stroke::new(1.5, pal.selection));
                        }
                        _ => {
                            let hr = egui::Rect::from_center_size(p, vec2(HANDLE * 2.0, HANDLE * 2.0));
                            clip.rect_filled(hr, 2.0, pal.panel);
                            clip.rect_stroke(hr, 2.0, Stroke::new(1.5, pal.selection), egui::StrokeKind::Middle);
                        }
                    }
                }
            }
        }

        self.canvas_interaction(&ctx, &resp, &tf, sel_screen, space, ppm);

        // Gummiband / Vorschau der neuen Form
        if let (Some(cur), true) = (resp.interact_pointer_pos(), resp.dragged_by(PointerButton::Primary)) {
            match &self.drag {
                Drag::Band { start, .. } => {
                    let a = tf.to_screen(*start);
                    let r = egui::Rect::from_two_pos(a, cur);
                    let crossing = cur.x < a.x;
                    clip.rect_filled(r, 0.0, pal.selection.gamma_multiply(0.08));
                    if crossing {
                        let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
                        clip.extend(Shape::dashed_line(&pts, Stroke::new(1.0, pal.selection), 4.0, 3.0));
                    } else {
                        clip.rect_stroke(r, 0.0, Stroke::new(1.0, pal.selection), egui::StrokeKind::Middle);
                    }
                }
                Drag::Create { tool, start } => {
                    let a = *start;
                    let b = self.snap(tf.to_doc(cur));
                    let color = theme::layer_color(self.doc.layers[self.current_layer].color);
                    let stroke = Stroke::new(1.2, color);
                    let ghost = match tool {
                        Tool::Rect => VectorPath::rect(
                            a.x.min(b.x),
                            a.y.min(b.y),
                            (b.x - a.x).abs(),
                            (b.y - a.y).abs(),
                            self.tool_options.corner_radius,
                        ),
                        Tool::Ellipse => {
                            let r = DRect::from_points(a, b);
                            VectorPath::ellipse(r.center(), r.width() / 2.0, r.height() / 2.0)
                        }
                        Tool::Line => VectorPath::line(a, b),
                        Tool::Polygon => VectorPath::regular_polygon(a, a.distance(b), self.tool_options.polygon_sides),
                        Tool::Star => VectorPath::star(
                            a,
                            a.distance(b),
                            a.distance(b) * self.tool_options.star_inner,
                            self.tool_options.star_points,
                        ),
                        _ => VectorPath::default(),
                    };
                    for poly in ghost.flatten(&vt, 0.3) {
                        let pts: Vec<Pos2> = poly.points.iter().map(|p| p2(*p)).collect();
                        if pts.len() >= 2 {
                            clip.add(if poly.closed {
                                Shape::closed_line(pts, stroke)
                            } else {
                                Shape::line(pts, stroke)
                            });
                        }
                    }
                    let size = DRect::from_points(a, b);
                    clip.text(
                        cur + vec2(14.0, 14.0),
                        egui::Align2::LEFT_TOP,
                        format!("{:.1} × {:.1} mm", size.width(), size.height()),
                        egui::FontId::proportional(11.5),
                        pal.text,
                    );
                }
                _ => {}
            }
        }

        self.draw_rulers(&painter, rect, work, &tf, pal);
        self.textures.end_frame();
        let alive: Vec<ObjectId> = self.doc.objects.iter().map(|o| o.id).collect();
        if self.geom.map.len() > alive.len() + 64 {
            self.geom.retain(|id| alive.contains(id));
        }
    }

    fn canvas_interaction(
        &mut self,
        ctx: &egui::Context,
        resp: &egui::Response,
        tf: &ViewTf,
        sel_screen: Option<egui::Rect>,
        space: bool,
        ppm: f64,
    ) {
        let (shift, ctrl) = ctx.input(|i| (i.modifiers.shift, i.modifiers.command));
        let tol = 6.0 * ppm;

        // Mauszeiger über Griffen
        if let (Some(hover), Some(r), Tool::Select, true) =
            (resp.hover_pos(), sel_screen, self.tool, self.drag.is_idle())
        {
            if let Some((h, _)) = handle_positions(r).into_iter().find(|(_, p)| p.distance(hover) <= HANDLE + 3.0) {
                ctx.set_cursor_icon(match h {
                    Handle::Corner(0) | Handle::Corner(2) => CursorIcon::ResizeNwSe,
                    Handle::Corner(_) => CursorIcon::ResizeNeSw,
                    Handle::Edge(0) | Handle::Edge(2) => CursorIcon::ResizeVertical,
                    Handle::Edge(_) => CursorIcon::ResizeHorizontal,
                    Handle::Rotate => CursorIcon::Alias,
                });
            }
        }
        if self.tool != Tool::Select && resp.hovered() && !space {
            ctx.set_cursor_icon(if self.tool == Tool::Text { CursorIcon::Text } else { CursorIcon::Crosshair });
        }

        if resp.drag_started_by(PointerButton::Primary) && !space && !self.preview.active {
            let press = ctx.input(|i| i.pointer.press_origin()).unwrap_or_default();
            let start = tf.to_doc(press);
            self.drag = match self.tool {
                Tool::Select => {
                    let handle = sel_screen.and_then(|r| {
                        handle_positions(r).into_iter().find(|(_, p)| p.distance(press) <= HANDLE + 3.0).map(|(h, _)| h)
                    });
                    if let Some(h) = handle {
                        let bounds = self.selection_bounds();
                        let originals = self.originals();
                        self.checkpoint();
                        match h {
                            Handle::Rotate => {
                                let c = bounds.center();
                                Drag::Rotate { center: c, start_angle: (start.y - c.y).atan2(start.x - c.x), originals }
                            }
                            _ => Drag::Scale { handle: h, bounds, originals },
                        }
                    } else if let Some(id) = self.hit_test(start, tol) {
                        if !self.selection.contains(&id) {
                            let group = self.doc.expand_groups(&[id]);
                            if shift {
                                self.selection.extend(group);
                            } else {
                                self.selection = group;
                            }
                        }
                        let originals = self.originals();
                        self.checkpoint();
                        Drag::Move { start, originals }
                    } else {
                        Drag::Band { start, additive: shift }
                    }
                }
                Tool::Text => Drag::None,
                t => Drag::Create { tool: t, start: self.snap(start) },
            };
        } else if resp.drag_started_by(PointerButton::Middle) {
            self.drag = Drag::Pan;
        }

        if resp.dragged_by(PointerButton::Primary) && !space {
            if let Some(cur) = resp.interact_pointer_pos() {
                let p = tf.to_doc(cur);
                match std::mem::replace(&mut self.drag, Drag::None) {
                    Drag::Move { start, originals } => {
                        let mut d = p - start;
                        if shift {
                            if d.x.abs() > d.y.abs() { d.y = 0.0 } else { d.x = 0.0 }
                        }
                        if self.settings.snap_to_grid {
                            d = self.snap(d);
                        }
                        self.apply_originals(&originals, &Affine::translate(d.x, d.y));
                        ctx.set_cursor_icon(CursorIcon::Move);
                        self.drag = Drag::Move { start, originals };
                    }
                    Drag::Scale { handle, bounds, originals } => {
                        let (moving, anchor) = match handle {
                            Handle::Corner(0) => (bounds.min, bounds.max),
                            Handle::Corner(1) => {
                                (Point::new(bounds.max.x, bounds.min.y), Point::new(bounds.min.x, bounds.max.y))
                            }
                            Handle::Corner(2) => (bounds.max, bounds.min),
                            Handle::Corner(_) => {
                                (Point::new(bounds.min.x, bounds.max.y), Point::new(bounds.max.x, bounds.min.y))
                            }
                            Handle::Edge(0) => (bounds.min, bounds.max),
                            Handle::Edge(1) => (bounds.max, bounds.min),
                            Handle::Edge(2) => (bounds.max, bounds.min),
                            Handle::Edge(_) => (bounds.min, bounds.max),
                            Handle::Rotate => (bounds.max, bounds.min),
                        };
                        let fx = |p: f64, m: f64, a: f64| if (m - a).abs() < 1e-9 { 1.0 } else { (p - a) / (m - a) };
                        let mut sx = fx(p.x, moving.x, anchor.x);
                        let mut sy = fx(p.y, moving.y, anchor.y);
                        match handle {
                            Handle::Edge(0) | Handle::Edge(2) => sx = 1.0,
                            Handle::Edge(_) => sy = 1.0,
                            _ if !shift && self.lock_ratio => {
                                let s = if (sx - 1.0).abs() > (sy - 1.0).abs() { sx } else { sy };
                                sx = s;
                                sy = s;
                            }
                            _ => {}
                        }
                        let clamp = |s: f64| {
                            if s.abs() < 0.005 { 0.005_f64.copysign(if s == 0.0 { 1.0 } else { s }) } else { s }
                        };
                        self.apply_originals(&originals, &Affine::scale_about(clamp(sx), clamp(sy), anchor));
                        self.drag = Drag::Scale { handle, bounds, originals };
                    }
                    Drag::Rotate { center, start_angle, originals } => {
                        let mut a = (p.y - center.y).atan2(p.x - center.x) - start_angle;
                        if shift {
                            a = snap_angle(a, 15.0);
                        }
                        self.apply_originals(&originals, &Affine::rotate_about(a, center));
                        ctx.set_cursor_icon(CursorIcon::Alias);
                        self.drag = Drag::Rotate { center, start_angle, originals };
                    }
                    other => self.drag = other,
                }
            }
        }

        if resp.drag_stopped() {
            let end = resp.interact_pointer_pos().map(|p| tf.to_doc(p));
            match std::mem::replace(&mut self.drag, Drag::None) {
                Drag::Move { .. } | Drag::Scale { .. } | Drag::Rotate { .. } => {
                    self.history.discard_if_unchanged(&self.doc);
                    self.mark_changed();
                }
                Drag::Band { start, additive } => {
                    if let Some(end) = end {
                        let band = DRect::from_points(start, end);
                        let crossing = end.x < start.x;
                        let mut hits: Vec<ObjectId> = self
                            .doc
                            .objects
                            .iter()
                            .filter(|o| self.doc.layers[o.layer].selectable())
                            .filter(|o| {
                                let b = o.bounds();
                                if crossing { band.intersects(&b) } else { band.contains_rect(&b) }
                            })
                            .map(|o| o.id)
                            .collect();
                        hits = self.doc.expand_groups(&hits);
                        if additive {
                            for h in hits {
                                if !self.selection.contains(&h) {
                                    self.selection.push(h);
                                }
                            }
                        } else {
                            self.selection = hits;
                        }
                    }
                }
                Drag::Create { tool, start } => {
                    if let Some(end) = end {
                        let end = self.snap(end);
                        self.create_shape(tool, start, end, shift);
                    }
                }
                Drag::Pan | Drag::None => {}
            }
        }

        if resp.clicked() && !self.preview.active {
            if let Some(pos) = resp.interact_pointer_pos() {
                let p = tf.to_doc(pos);
                match self.tool {
                    Tool::Select => match self.hit_test(p, tol) {
                        Some(id) => {
                            let group = self.doc.expand_groups(&[id]);
                            if shift || ctrl {
                                if self.selection.contains(&id) {
                                    self.selection.retain(|s| !group.contains(s));
                                } else {
                                    self.selection.extend(group);
                                }
                            } else {
                                self.selection = group;
                            }
                            if let Some(o) = self.doc.get(id) {
                                self.current_layer = o.layer;
                            }
                        }
                        None => {
                            if !shift {
                                self.selection.clear();
                            }
                        }
                    },
                    Tool::Text => {
                        let at = self.snap(p);
                        let TextDialog { open, at: dialog_at, error, .. } = &mut self.text_dialog;
                        *open = true;
                        *dialog_at = at;
                        *error = None;
                    }
                    _ => {}
                }
            }
        }

        resp.context_menu(|ui| {
            let has = !self.selection.is_empty();
            use egui_phosphor::regular as i;
            if ui.add_enabled(has, egui::Button::new(format!("{} Duplizieren", i::COPY))).clicked() {
                self.duplicate();
                ui.close();
            }
            if ui.add_enabled(has, egui::Button::new(format!("{} Löschen", i::TRASH))).clicked() {
                self.delete_selection();
                ui.close();
            }
            ui.separator();
            if ui
                .add_enabled(self.selection.len() > 1, egui::Button::new(format!("{} Gruppieren", i::SELECTION)))
                .clicked()
            {
                self.group();
                ui.close();
            }
            if ui.add_enabled(has, egui::Button::new(format!("{} Gruppierung lösen", i::SELECTION_SLASH))).clicked() {
                self.ungroup();
                ui.close();
            }
            ui.separator();
            if ui.add_enabled(has, egui::Button::new(format!("{} Horizontal spiegeln", i::FLIP_HORIZONTAL))).clicked() {
                self.mirror(true);
                ui.close();
            }
            if ui.add_enabled(has, egui::Button::new(format!("{} Vertikal spiegeln", i::FLIP_VERTICAL))).clicked() {
                self.mirror(false);
                ui.close();
            }
            if ui.add_enabled(has, egui::Button::new(format!("{} 90° drehen", i::ARROW_CLOCKWISE))).clicked() {
                self.rotate_selection(90.0);
                ui.close();
            }
            if ui
                .add_enabled(has, egui::Button::new(format!("{} Auf Arbeitsfläche zentrieren", i::CROSSHAIR)))
                .clicked()
            {
                self.center_on_bed();
                ui.close();
            }
        });
    }

    /// Zeichnet den Werkzeugweg eines Jobs bis `progress` (0…1). Mit `layer_colors` erscheinen Linien in
    /// der Farbe ihrer Ebene, sonst in Brennfarbe (z. B. Materialtest, dessen Felder keine Ebenen sind).
    #[allow(clippy::too_many_arguments)]
    fn draw_job(
        &self,
        painter: &egui::Painter,
        tf: &ViewTf,
        pal: &theme::Palette,
        dark: bool,
        job: &Job,
        progress: f32,
        textures: &[TextureId],
        layer_colors: bool,
    ) {
        let burn = if dark { Color32::from_rgb(255, 196, 120) } else { Color32::from_rgb(40, 30, 25) };
        let total: usize = job.toolpath.op_count();
        let limit = if progress >= 0.999 { usize::MAX } else { (progress as f64 * total as f64) as usize };

        let mut count = 0usize;
        let mut cur = job.start;
        let mut cut: Vec<Pos2> = Vec::new();
        let mut cut_color = burn;
        let show_travel = self.settings.show_travel;
        let mut travel_shapes: Vec<Shape> = Vec::new();
        let flush = |cut: &mut Vec<Pos2>, color: Color32| {
            if cut.len() >= 2 {
                painter.add(Shape::line(std::mem::take(cut), Stroke::new(1.3, color)));
            }
            cut.clear();
        };
        'outer: for block in &job.toolpath.blocks {
            let layer_color = match self.doc.layers.get(block.layer) {
                Some(l) if layer_colors => theme::layer_color(l.color),
                _ => burn,
            };
            if block.kind == BlockKind::Raster {
                flush(&mut cut, cut_color);
                if count >= limit {
                    break;
                }
                if let (Some(idx), true) = (block.raster, block.pass == 0) {
                    let r = &job.toolpath.rasters[idx].result;
                    let r0 = r.origin;
                    let r1 = Point::new(r0.x + r.cols as f64 * r.dx, r0.y + r.rows as f64 * r.dy);
                    let c = [
                        tf.to_screen(r0),
                        tf.to_screen(Point::new(r1.x, r0.y)),
                        tf.to_screen(r1),
                        tf.to_screen(Point::new(r0.x, r1.y)),
                    ];
                    if let Some(&tex) = textures.get(idx) {
                        textured_quad(painter, tex, c, burn);
                    }
                }
                if let Some(first) = block.ops.first() {
                    if show_travel {
                        travel_shapes.push(Shape::line_segment(
                            [tf.to_screen(cur), tf.to_screen(first.target())],
                            Stroke::new(1.0, pal.travel),
                        ));
                    }
                }
                count += block.ops.len();
                if let Some(last) = block.ops.last() {
                    cur = last.target();
                }
                continue;
            }
            cut_color = if block.kind == BlockKind::Line { layer_color } else { burn };
            for op in &block.ops {
                if count >= limit {
                    flush(&mut cut, cut_color);
                    break 'outer;
                }
                count += 1;
                match *op {
                    Op::Move { to, power } if power > 0.0 => {
                        if cut.is_empty() {
                            cut.push(tf.to_screen(cur));
                        }
                        cut.push(tf.to_screen(to));
                    }
                    _ => {
                        flush(&mut cut, cut_color);
                        let to = op.target();
                        if show_travel && matches!(op, Op::Rapid(_)) {
                            travel_shapes.push(Shape::line_segment(
                                [tf.to_screen(cur), tf.to_screen(to)],
                                Stroke::new(1.0, pal.travel),
                            ));
                        }
                    }
                }
                cur = op.target();
            }
            flush(&mut cut, cut_color);
        }
        flush(&mut cut, cut_color);
        painter.extend(travel_shapes);
        if limit != usize::MAX {
            let p = tf.to_screen(cur);
            painter.circle_filled(p, 4.0, pal.danger);
        }
        // Job-Rahmen
        let r = tf.rect(&job.design_bounds);
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, pal.accent.gamma_multiply(0.6)), egui::StrokeKind::Outside);
    }

    fn draw_rulers(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        work: egui::Rect,
        tf: &ViewTf,
        pal: &theme::Palette,
    ) {
        let top = egui::Rect::from_min_max(pos2(work.left(), rect.top()), pos2(rect.right(), work.top()));
        let left = egui::Rect::from_min_max(pos2(rect.left(), work.top()), pos2(work.left(), rect.bottom()));
        painter.rect_filled(top, 0.0, pal.ruler);
        painter.rect_filled(left, 0.0, pal.ruler);
        painter.rect_filled(egui::Rect::from_min_max(rect.min, work.min), 0.0, pal.ruler);
        painter.line_segment([top.left_bottom(), top.right_bottom()], Stroke::new(1.0, pal.border));
        painter.line_segment([left.right_top(), left.right_bottom()], Stroke::new(1.0, pal.border));
        let profile = &self.settings.profile;
        let step = nice_step(60.0 / tf.zoom as f64);
        let sub = step / 5.0;
        let font = egui::FontId::proportional(10.0);
        let text_color = pal.text_weak;
        let tick = Stroke::new(1.0, pal.text_weak.gamma_multiply(0.6));
        let visible = DRect::from_points(tf.to_doc(work.min), tf.to_doc(work.max));
        let fmt = |v: f64| if step < 1.0 { format!("{v:.1}") } else { format!("{v:.0}") };

        let top_clip = painter.with_clip_rect(top);
        let i0 = (visible.min.x / sub).floor() as i64;
        let i1 = (visible.max.x / sub).ceil() as i64;
        for i in i0..=i1 {
            let x = i as f64 * sub;
            let sx = tf.to_screen(Point::new(x, 0.0)).x;
            let is_major = (i % 5) == 0;
            let h = if is_major { 9.0 } else { 4.0 };
            top_clip.line_segment([pos2(sx, top.bottom() - h), pos2(sx, top.bottom())], tick);
            if is_major {
                let mx = profile.doc_to_machine(Point::new(x, 0.0)).x;
                top_clip.text(
                    pos2(sx + 3.0, top.top() + 2.0),
                    egui::Align2::LEFT_TOP,
                    fmt(mx),
                    font.clone(),
                    text_color,
                );
            }
        }
        let left_clip = painter.with_clip_rect(left);
        let j0 = (visible.min.y / sub).floor() as i64;
        let j1 = (visible.max.y / sub).ceil() as i64;
        for j in j0..=j1 {
            let y = j as f64 * sub;
            let sy = tf.to_screen(Point::new(0.0, y)).y;
            let is_major = (j % 5) == 0;
            let w = if is_major { 9.0 } else { 4.0 };
            left_clip.line_segment([pos2(left.right() - w, sy), pos2(left.right(), sy)], tick);
            if is_major {
                let my = profile.doc_to_machine(Point::new(0.0, y)).y;
                left_clip.text(
                    pos2(left.left() + 2.0, sy + 2.0),
                    egui::Align2::LEFT_TOP,
                    fmt(my),
                    font.clone(),
                    text_color,
                );
            }
        }
        // Mausposition in den Linealen
        if let Some(c) = self.view.cursor {
            let s = tf.to_screen(c);
            painter.line_segment([pos2(s.x, top.top()), pos2(s.x, top.bottom())], Stroke::new(1.0, pal.accent));
            painter.line_segment([pos2(left.left(), s.y), pos2(left.right(), s.y)], Stroke::new(1.0, pal.accent));
        }
    }
}
