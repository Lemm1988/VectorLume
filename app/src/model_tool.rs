//! 3D-Modell (STL/OBJ) als Tiefenbild einfügen: Blickrichtung, Breite und Auflösung wählen, Vorschau ansehen.
//! Das Bild landet auf der aktuellen Ebene; eine bisher leere Ebene graviert es gleich in Graustufen.

use crate::app::{ToastKind, VectorLumeApp};
use crate::i18n::{tr, trs};
use crate::theme;
use crate::units::{self, UnitField};
use egui::{Align2, Button, DragValue, RichText, TextureHandle, vec2};
use egui_phosphor::regular as icon;
use std::path::Path;
use std::sync::Arc;
use vl_core::{DitherMode, ImageAdjust, ImageObject};
use vl_io::model3d::{self, DepthOptions, Mesh, View};

/// Kantenlänge der Vorschau in Pixeln.
const PREVIEW_PX: f64 = 240.0;

pub struct ModelTool {
    pub open: bool,
    pub name: String,
    pub mesh: Option<Arc<Mesh>>,
    pub view: View,
    /// mm je Modelleinheit (STL/OBJ haben keine Einheit; meist mm).
    pub scale: f64,
    pub px_per_mm: f64,
    pub invert: bool,
    preview: Option<((View, bool), TextureHandle)>,
}

impl Default for ModelTool {
    fn default() -> Self {
        Self {
            open: false,
            name: String::new(),
            mesh: None,
            view: View::Top,
            scale: 1.0,
            px_per_mm: 10.0,
            invert: false,
            preview: None,
        }
    }
}

impl ModelTool {
    fn options(&self, mesh: &Mesh) -> DepthOptions {
        let width_mm = mesh.extent(self.view)[0] as f64 * self.scale;
        DepthOptions { view: self.view, width_mm, px_per_mm: self.px_per_mm, invert: self.invert }
    }
}

fn view_name(v: View) -> &'static str {
    match v {
        View::Top => tr("Oben"),
        View::Bottom => tr("Unten"),
        View::Front => tr("Vorne"),
        View::Back => tr("Hinten"),
        View::Left => tr("Links"),
        View::Right => tr("Rechts"),
    }
}

impl VectorLumeApp {
    /// Lädt ein 3D-Modell und öffnet den Dialog; zu große Modelle werden auf die Arbeitsfläche verkleinert.
    pub(crate) fn open_model(&mut self, path: &Path) {
        let mesh = match std::fs::read(path) {
            Ok(data) => model3d::parse_model(&data),
            Err(e) => Err(format!("Datei konnte nicht gelesen werden: {e}")),
        };
        let mesh = match mesh {
            Ok(m) => m,
            Err(e) => {
                self.toast(ToastKind::Error, format!("{}: {}", crate::app::file_name(path), trs(&e)));
                return;
            }
        };
        let bed = self.settings.profile.bed_rect();
        let [w, h, _] = mesh.extent(View::Top);
        let fit = (bed.width() / w as f64).min(bed.height() / h as f64);
        let t = &mut self.model;
        t.name = path.file_stem().map_or_else(|| tr("3D-Modell").to_string(), |s| s.to_string_lossy().into_owned());
        t.mesh = Some(Arc::new(mesh));
        t.view = View::Top;
        t.scale = if fit.is_finite() { fit.min(1.0) } else { 1.0 };
        t.preview = None;
        t.open = true;
    }

    pub(crate) fn insert_model(&mut self) {
        let Some(mesh) = self.model.mesh.clone() else { return };
        let opts = self.model.options(&mesh);
        let map = match model3d::depth_map(&mesh, &opts) {
            Ok(m) => m,
            Err(e) => {
                self.toast(ToastKind::Error, trs(&e));
                return;
            }
        };
        let image = ImageObject {
            image: Arc::new(map.image),
            width_mm: map.width_mm,
            height_mm: map.height_mm,
            adjust: ImageAdjust::default(),
            clip: Vec::new(),
        };
        let layer = self.current_layer;
        let fresh = self.doc.objects_on_layer(layer).next().is_none();
        self.checkpoint();
        let id = self.doc.add_image(self.model.name.clone(), layer, image, vl_core::Point::ZERO);
        if fresh {
            // Tiefenbilder sind Höhen, keine Fotos: Leistung folgt direkt dem Grauwert
            let l = &mut self.doc.layers[layer];
            let op = l.image_operation();
            if let Some(cs) = l.operation_mut(op) {
                cs.dither = DitherMode::Grayscale;
            }
        }
        self.place_imported(&[id]);
        self.selection = vec![id];
        self.mark_changed();
        self.model.open = false;
        self.model.mesh = None;
        self.model.preview = None;
        self.toast(
            ToastKind::Success,
            trs(&format!(
                "Tiefenbild „{}“ eingefügt. Für ein Relief Bildmodus „Graustufen“ oder „3D-Schichten“ verwenden.",
                self.doc.get(id).map_or("", |o| o.name.as_str())
            )),
        );
    }
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.model.open {
        return;
    }
    let Some(mesh) = app.model.mesh.clone() else {
        app.model.open = false;
        return;
    };
    let mut open = true;
    let mut insert = false;
    egui::Window::new(trs(&format!("{} 3D-Modell als Tiefenbild", icon::CUBE)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(460.0)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let t = &mut app.model;
            let [mx, my, mz] = mesh.extent(View::Top);
            ui.label(
                RichText::new(trs(&format!(
                    "{} · {} Dreiecke · Modell {} × {} × {} {}",
                    t.name,
                    mesh.triangles.len(),
                    units::num(mx as f64, 1),
                    units::num(my as f64, 1),
                    units::num(mz as f64, 1),
                    units::unit()
                )))
                .small()
                .color(pal.text_weak),
            );
            ui.add_space(4.0);
            ui.horizontal_top(|ui| {
                egui::Grid::new("model3d").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                    ui.label(tr("Ansicht"));
                    egui::ComboBox::from_id_salt("model_view").selected_text(view_name(t.view)).show_ui(ui, |ui| {
                        for v in View::ALL {
                            ui.selectable_value(&mut t.view, v, view_name(v));
                        }
                    });
                    ui.end_row();
                    let [ew, eh, ed] = mesh.extent(t.view).map(|v| v as f64);
                    let mut width = ew * t.scale;
                    ui.label(tr("Breite"));
                    if ui.add(DragValue::new(&mut width).speed(0.5).range(1.0..=2000.0).mm()).changed() && ew > 0.0 {
                        t.scale = width / ew;
                    }
                    ui.end_row();
                    ui.label(tr("Höhe"));
                    ui.label(format!("{} {}", units::num(eh * t.scale, 1), units::unit()));
                    ui.end_row();
                    ui.label(tr("Auflösung"));
                    ui.add(DragValue::new(&mut t.px_per_mm).speed(0.1).range(1.0..=40.0).max_decimals(1).suffix(" px/mm"))
                        .on_hover_text(tr("10 px/mm = 0,1 mm je Pixel (254 dpi). Zeilenabstand der Ebene passend wählen."));
                    ui.end_row();
                    ui.label(tr("Invertieren"));
                    ui.checkbox(&mut t.invert, tr("Hoch = dunkel"))
                        .on_hover_text(tr("Für Formen und Stempel: erhabene Stellen werden am tiefsten gelasert."));
                    ui.end_row();
                    let opts = t.options(&mesh);
                    let (cols, rows) = model3d::depth_size(&mesh, &opts);
                    ui.label(tr("Bild"));
                    ui.label(trs(&format!("{cols} × {rows} Pixel")));
                    ui.end_row();
                    ui.label(tr("Modelltiefe"));
                    ui.label(format!("{} {}", units::num(ed * t.scale, 1), units::unit()))
                        .on_hover_text(tr("Höhenunterschied im Modell bei dieser Breite. Die Gravurtiefe ergibt sich aus Leistung und Durchgängen."));
                    ui.end_row();
                });
                ui.add_space(8.0);
                let key = (t.view, t.invert);
                if t.preview.as_ref().is_none_or(|(k, _)| *k != key) {
                    let [ew, eh, _] = mesh.extent(t.view);
                    let side = ew.max(eh) as f64;
                    let opts = DepthOptions {
                        view: t.view,
                        width_mm: ew as f64,
                        px_per_mm: if side > 0.0 { PREVIEW_PX / side } else { 1.0 },
                        invert: t.invert,
                    };
                    t.preview = model3d::depth_map(&mesh, &opts).ok().map(|m| {
                        let img = egui::ColorImage::from_gray(
                            [m.image.width as usize, m.image.height as usize],
                            &m.image.pixels,
                        );
                        (key, ui.ctx().load_texture("tiefenbild", img, egui::TextureOptions::LINEAR))
                    });
                }
                match &t.preview {
                    Some((_, tex)) => {
                        let size = tex.size_vec2();
                        let f = PREVIEW_PX as f32 / size.x.max(size.y);
                        ui.add(egui::Image::new((tex.id(), size * f)).bg_fill(pal.canvas));
                    }
                    None => {
                        ui.label(
                            RichText::new(tr("Das Modell ist aus dieser Richtung flach – bitte eine andere Ansicht wählen."))
                                .color(pal.warning),
                        );
                    }
                }
            });
            ui.add_space(6.0);
            ui.label(
                RichText::new(tr(
                    "Hell = hoch, dunkel = tief; außerhalb des Modells weiß. Graviert wird mit dem Bildmodus der Ebene – \
                     „Graustufen“ oder „3D-Schichten“ ergeben ein Relief.",
                ))
                .small()
                .color(pal.text_weak),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let ok = t.preview.is_some();
                if ui
                    .add_enabled(
                        ok,
                        Button::new(RichText::new(tr("Tiefenbild einfügen")).color(egui::Color32::WHITE)).fill(pal.accent),
                    )
                    .clicked()
                {
                    insert = true;
                }
            });
        });
    if insert {
        app.insert_model();
    }
    if !open {
        app.model.open = false;
        app.model.mesh = None;
        app.model.preview = None;
    }
}
