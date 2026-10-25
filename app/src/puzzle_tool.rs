//! Puzzle-Generator: Schnittlinien für ein Puzzle, z. B. über ein graviertes Foto. Ist beim Öffnen etwas
//! ausgewählt, übernimmt das Puzzle dessen Lage und Größe.

use crate::app::{ToastKind, VectorLumeApp};
use crate::i18n::{tr, trs};
use crate::theme;
use crate::units::UnitField;
use egui::{Align2, Button, DragValue, RichText, Sense, Shape, Stroke, vec2};
use egui_phosphor::regular as icon;
use vl_core::puzzle::PuzzleSpec;
use vl_core::{Affine, Point};

pub struct PuzzleTool {
    pub open: bool,
    pub spec: PuzzleSpec,
    /// Linke obere Ecke im Dokument.
    pub origin: Point,
}

impl Default for PuzzleTool {
    fn default() -> Self {
        Self { open: false, spec: PuzzleSpec::default(), origin: Point::new(10.0, 10.0) }
    }
}

impl VectorLumeApp {
    /// Öffnet den Generator; mit Auswahl (z. B. einem Foto) über deren Fläche.
    pub(crate) fn open_puzzle(&mut self) {
        let sel = self.selection_bounds();
        if !sel.is_empty() && sel.width() >= 10.0 && sel.height() >= 10.0 {
            self.puzzle.origin = sel.min;
            self.puzzle.spec.width = sel.width();
            self.puzzle.spec.height = sel.height();
        } else {
            let bed = self.settings.profile.bed_rect();
            self.puzzle.origin = bed.min + Point::new(10.0, 10.0);
        }
        self.puzzle.open = true;
    }

    pub(crate) fn insert_puzzle(&mut self) {
        let spec = self.puzzle.spec;
        if let Err(e) = spec.check() {
            self.toast(ToastKind::Error, tr(e));
            return;
        }
        let (cuts, border) = spec.build();
        let at = Affine::translate(self.puzzle.origin.x, self.puzzle.origin.y);
        self.checkpoint();
        let mut ids = Vec::new();
        if !cuts.is_empty() {
            ids.push(self.doc.add_path(tr("Puzzle"), self.current_layer, cuts.transformed(&at)));
        }
        if let Some(b) = border {
            ids.push(self.doc.add_path(tr("Puzzle-Rahmen"), self.current_layer, b.transformed(&at)));
        }
        if ids.len() > 1 {
            self.doc.group(&ids);
        }
        self.selection = ids;
        self.mark_changed();
        self.toast(ToastKind::Success, trs(&format!("Puzzle mit {} Teilen erzeugt.", spec.cols * spec.rows)));
    }
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.puzzle.open {
        return;
    }
    let mut open = true;
    let mut create = false;
    egui::Window::new(trs(&format!("{} Puzzle", icon::PUZZLE_PIECE)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(420.0)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let p = &mut app.puzzle;
            egui::Grid::new("puzzle").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                ui.label(tr("Größe"));
                ui.horizontal(|ui| {
                    ui.add(DragValue::new(&mut p.spec.width).speed(0.5).range(10.0..=2000.0).mm());
                    ui.label("×");
                    ui.add(DragValue::new(&mut p.spec.height).speed(0.5).range(10.0..=2000.0).mm());
                });
                ui.end_row();
                ui.label(tr("Teile"));
                ui.horizontal(|ui| {
                    ui.add(DragValue::new(&mut p.spec.cols).range(1..=100).suffix(tr(" Spalten")));
                    ui.label("×");
                    ui.add(DragValue::new(&mut p.spec.rows).range(1..=100).suffix(tr(" Zeilen")));
                });
                ui.end_row();
                ui.label(tr("Nasengröße"));
                let mut pct = p.spec.tab * 100.0;
                if ui.add(egui::Slider::new(&mut pct, 10.0..=32.0).suffix(" %").fixed_decimals(0)).changed() {
                    p.spec.tab = pct / 100.0;
                }
                ui.end_row();
                ui.label(tr("Unregelmäßigkeit"));
                let mut j = p.spec.jitter * 100.0;
                if ui.add(egui::Slider::new(&mut j, 0.0..=100.0).suffix(" %").fixed_decimals(0)).changed() {
                    p.spec.jitter = j / 100.0;
                }
                ui.end_row();
                ui.label(tr("Rahmen"));
                ui.horizontal(|ui| {
                    ui.checkbox(&mut p.spec.border, tr("mitschneiden"));
                    if p.spec.border {
                        ui.label(tr("Eckenradius"));
                        ui.add(DragValue::new(&mut p.spec.corner_radius).speed(0.1).range(0.0..=100.0).mm());
                    }
                });
                ui.end_row();
            });
            let spec = p.spec;
            let check = spec.check();
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().max(380.0), 200.0), Sense::hover());
            ui.painter().rect_filled(rect, 6.0, pal.canvas);
            if check.is_ok() {
                let (cuts, border) = spec.build();
                let inner = rect.shrink(10.0);
                let scale = (inner.width() as f64 / spec.width).min(inner.height() as f64 / spec.height);
                let off = inner.center() - vec2((spec.width * scale) as f32 / 2.0, (spec.height * scale) as f32 / 2.0);
                let mut polys = cuts.flatten(&Affine::IDENTITY, 0.5 / scale);
                if let Some(b) = border {
                    polys.extend(b.flatten(&Affine::IDENTITY, 0.5 / scale));
                }
                for q in polys {
                    let mut pts: Vec<egui::Pos2> =
                        q.points.iter().map(|p| off + vec2((p.x * scale) as f32, (p.y * scale) as f32)).collect();
                    if q.closed {
                        pts.push(pts[0]);
                    }
                    ui.painter().add(Shape::line(pts, Stroke::new(1.2, pal.accent)));
                }
            }
            if let Err(e) = check {
                ui.label(RichText::new(tr(e)).color(pal.danger));
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui
                    .button(trs(&format!("{} Neu würfeln", icon::DICE_FIVE)))
                    .on_hover_text(tr("Andere zufällige Nasen bei gleichen Einstellungen"))
                    .clicked()
                {
                    app.puzzle.spec.seed = app.puzzle.spec.seed.wrapping_add(1);
                }
                if ui.add_enabled(check.is_ok(), Button::new(trs(&format!("{} Puzzle erzeugen", icon::PUZZLE_PIECE)))).clicked()
                {
                    create = true;
                }
            });
            ui.label(
                RichText::new(tr(
                    "Tipp: Foto zuerst gravieren, dann das Puzzle auf einer Linienebene schneiden. Das Puzzle übernimmt beim Öffnen Lage und Größe der Auswahl.",
                ))
                .small()
                .color(pal.text_weak),
            );
        });
    if create {
        app.insert_puzzle();
        app.puzzle.open = false;
    }
    if !open {
        app.puzzle.open = false;
    }
}
