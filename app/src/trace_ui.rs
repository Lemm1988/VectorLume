//! Dialog „Bild nachzeichnen“: Vektorkonturen aus einem Bild erzeugen, mit Live-Vorschau auf der Arbeitsfläche.

use crate::app::{ToastKind, VectorLumeApp};
use crate::i18n::{tr, trs};
use crate::theme;
use egui::{Align2, Button, Color32, DragValue, RichText, vec2};
use egui_phosphor::regular as icon;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use vl_core::{Affine, ObjectId, Shape, VectorPath};
use vl_raster::trace::{TraceOptions, trace};

pub struct TraceUi {
    pub open: bool,
    pub target: Option<ObjectId>,
    pub opts: TraceOptions,
    pub remove_image: bool,
    /// Ergebnis in Dokumentkoordinaten (für Vorschau und Übernahme).
    pub result: Option<Arc<Vec<VectorPath>>>,
    /// Einstellungen, zu denen `result` bzw. die laufende Berechnung gehört.
    computed: Option<(ObjectId, TraceOptions)>,
    busy: bool,
    tx: Sender<(ObjectId, TraceOptions, Vec<VectorPath>)>,
    rx: Receiver<(ObjectId, TraceOptions, Vec<VectorPath>)>,
}

impl Default for TraceUi {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            open: false,
            target: None,
            opts: TraceOptions::default(),
            remove_image: false,
            result: None,
            computed: None,
            busy: false,
            tx,
            rx,
        }
    }
}

impl VectorLumeApp {
    pub fn open_trace(&mut self) {
        let target = self.selection.iter().copied().find(|id| self.doc.get(*id).is_some_and(|o| o.is_image()));
        let Some(target) = target else {
            self.toast(ToastKind::Info, tr("Bitte zuerst ein Bild auswählen."));
            return;
        };
        let t = &mut self.trace;
        t.open = true;
        t.target = Some(target);
        t.result = None;
        t.computed = None;
    }

    /// Startet die Berechnung im Hintergrund, wenn sich Bild oder Einstellungen geändert haben.
    fn refresh_trace(&mut self, ctx: &egui::Context) {
        while let Ok((id, opts, paths)) = self.trace.rx.try_recv() {
            self.trace.busy = false;
            if Some((id, opts.clone())) == self.trace.computed {
                self.trace.result = Some(Arc::new(paths));
            }
        }
        let Some(id) = self.trace.target else { return };
        let Some(obj) = self.doc.get(id) else {
            self.trace.open = false;
            return;
        };
        let Shape::Image(img) = &obj.shape else { return };
        let key = (id, self.trace.opts.clone());
        if self.trace.busy || self.trace.computed.as_ref() == Some(&key) {
            return;
        }
        self.trace.computed = Some(key);
        self.trace.busy = true;
        let (mut image, adjust, opts, tx, ctx) =
            (img.image.clone(), img.adjust, self.trace.opts.clone(), self.trace.tx.clone(), ctx.clone());
        // Zugeschnittene Bilder: außerhalb weiß, damit dort nichts nachgezeichnet wird
        if let Some(mask) = img.clip_mask(image.width as usize, image.height as usize) {
            let pixels = image.pixels.iter().zip(mask).map(|(&v, inside)| if inside { v } else { 255 }).collect();
            image = std::sync::Arc::new(vl_core::RasterImage::new(image.width, image.height, pixels));
        }
        // Pixel → lokale mm → Dokument
        let to_doc =
            Affine::scale(img.width_mm / image.width.max(1) as f64, img.height_mm / image.height.max(1) as f64)
                .then(&obj.transform);
        std::thread::spawn(move || {
            let paths: Vec<VectorPath> = trace(&image, &adjust, &opts).iter().map(|p| p.transformed(&to_doc)).collect();
            let _ = tx.send((id, opts, paths));
            ctx.request_repaint();
        });
    }

    /// Ziel-Ebene: die aktuelle, außer sie ist eine Bild-Ebene – dann die erste Vektorebene.
    fn trace_layer(&self) -> usize {
        let vector = |i: usize| self.doc.layers[i].settings.mode != vl_core::LayerMode::Image;
        if vector(self.current_layer) {
            self.current_layer
        } else {
            (0..self.doc.layers.len()).find(|&i| vector(i)).unwrap_or(0)
        }
    }

    fn apply_trace(&mut self) {
        let Some(paths) = self.trace.result.clone() else { return };
        let Some(id) = self.trace.target else { return };
        if paths.is_empty() {
            self.toast(ToastKind::Info, tr("Mit diesen Einstellungen wurden keine Konturen gefunden."));
            return;
        }
        let mut all = VectorPath::default();
        for p in paths.iter() {
            all.append(p.clone());
        }
        self.checkpoint();
        let new_id = self.doc.add_path(tr("Nachgezeichnet"), self.trace_layer(), all);
        if self.trace.remove_image {
            self.doc.remove(&[id]);
        }
        self.selection = vec![new_id];
        self.mark_changed();
        self.trace.open = false;
        self.toast(ToastKind::Success, trs(&format!("{} Konturen erzeugt.", paths.len())));
    }
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.trace.open {
        return;
    }
    app.refresh_trace(ctx);
    let mut open = true;
    let mut apply = false;
    let layer_name = app.doc.layers[app.trace_layer()].name.clone();
    egui::Window::new(trs(&format!("{} Bild nachzeichnen", icon::SCAN)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::RIGHT_TOP, vec2(-380.0, 90.0))
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let t = &mut app.trace;
            egui::Grid::new("trace").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label(tr("Schwellwert"));
                ui.add(egui::Slider::new(&mut t.opts.threshold, 1..=254))
                    .on_hover_text(tr("Grauwerte darunter (dunkler) werden nachgezeichnet"));
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut t.opts.invert, tr("Helle Bereiche nachzeichnen"));
                ui.end_row();
                ui.label(tr("Flecken ignorieren"));
                ui.add(DragValue::new(&mut t.opts.min_area).speed(0.5).range(0.0..=10000.0).suffix(" px²"))
                    .on_hover_text(tr("Konturen mit kleinerer Fläche werden verworfen (Staub, Rauschen)"));
                ui.end_row();
                ui.label(tr("Vereinfachen"));
                ui.add(egui::Slider::new(&mut t.opts.tolerance, 0.0..=5.0).suffix(" px").fixed_decimals(1))
                    .on_hover_text(tr("Erlaubte Abweichung: höher = weniger Knoten"));
                ui.end_row();
                ui.label(tr("Glätten"));
                ui.add(egui::Slider::new(&mut t.opts.smoothing, 0.0..=1.0).fixed_decimals(2))
                    .on_hover_text(tr("0 = gerade Linien, 1 = runde Kurven"));
                ui.end_row();
                ui.label(tr("Eckenwinkel"));
                ui.add(egui::Slider::new(&mut t.opts.corner_angle, 20.0..=150.0).suffix("°").fixed_decimals(0))
                    .on_hover_text(tr("Stärkere Knicke bleiben beim Glätten spitze Ecken"));
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut t.remove_image, tr("Bild danach entfernen"));
                ui.end_row();
            });
            ui.add_space(4.0);
            let status = match (&t.result, t.busy) {
                (_, true) => trs(&format!("{} Berechne …", icon::HOURGLASS)),
                (Some(r), false) => {
                    let nodes: usize = r.iter().map(|p| p.segment_count()).sum();
                    trs(&format!("{} Konturen · {} Segmente · Ziel-Ebene „{}“", r.len(), nodes, layer_name))
                }
                (None, false) => String::new(),
            };
            ui.label(RichText::new(status).small().color(pal.text_weak));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let ready = t.result.as_ref().is_some_and(|r| !r.is_empty()) && !t.busy;
                if ui
                    .add_enabled(
                        ready,
                        Button::new(RichText::new(tr("Übernehmen")).color(Color32::WHITE)).fill(pal.accent),
                    )
                    .clicked()
                {
                    apply = true;
                }
                if ui.button(tr("Abbrechen")).clicked() {
                    t.open = false;
                }
            });
        });
    if apply {
        app.apply_trace();
    }
    if !open {
        app.trace.open = false;
    }
}
