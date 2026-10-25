//! Zahnrad-Generator: Stirnrad mit Evolventenverzahnung, wahlweise mit passendem Gegenrad. Die Vorschau zeigt das
//! Paar im Eingriff; auf die Arbeitsfläche kommen die Räder mit Abstand nebeneinander, damit sich die Schnitte
//! nicht berühren.

use crate::app::{ToastKind, VectorLumeApp};
use crate::i18n::{tr, trs};
use crate::theme;
use crate::units::{self, UnitField};
use egui::{Align2, Button, DragValue, RichText, Sense, Shape, Stroke, vec2};
use egui_phosphor::regular as icon;
use vl_core::gear::GearSpec;
use vl_core::{Affine, Point};

/// Abstand der Räder beim Auslegen (mm).
const GAP: f64 = 4.0;
/// Abstand zum Rand der Arbeitsfläche (mm).
const MARGIN: f64 = 5.0;

pub struct GearTool {
    pub open: bool,
    pub spec: GearSpec,
    /// Gegenrad mit gleichem Modul mitzeichnen.
    pub pair: bool,
    pub teeth2: u32,
    pub bore2: f64,
}

impl Default for GearTool {
    fn default() -> Self {
        Self { open: false, spec: GearSpec::default(), pair: true, teeth2: 30, bore2: 5.0 }
    }
}

impl GearTool {
    fn second(&self) -> Option<GearSpec> {
        self.pair.then_some(GearSpec { teeth: self.teeth2, bore: self.bore2, ..self.spec })
    }

    fn check(&self) -> Result<(), &'static str> {
        self.spec.check()?;
        self.second().map_or(Ok(()), |g| g.check())
    }
}

impl VectorLumeApp {
    pub(crate) fn insert_gears(&mut self) {
        let tool = &self.gear;
        if let Err(e) = tool.check() {
            self.toast(ToastKind::Error, tr(e));
            return;
        }
        let g1 = tool.spec;
        let mut gears = vec![(g1, 0.0)];
        let distance = tool.second().map(|g2| {
            gears.push((g2, g1.mesh_rotation(&g2)));
            g1.center_distance(&g2)
        });
        let bed = self.settings.profile.bed_rect();
        let y = bed.min.y + MARGIN + gears.iter().map(|(g, _)| g.tip_radius()).fold(0.0, f64::max);
        let mut x = bed.min.x + MARGIN;
        self.checkpoint();
        let mut ids = Vec::new();
        for (g, rot) in &gears {
            x += g.tip_radius();
            let at = Affine::rotate(*rot).then(&Affine::translate(x, y));
            ids.push(self.doc.add_path(
                trs(&format!("Zahnrad {} Zähne", g.teeth)),
                self.current_layer,
                g.outline().transformed(&at),
            ));
            x += g.tip_radius() + GAP;
        }
        if ids.len() > 1 {
            self.doc.group(&ids);
        }
        self.selection = ids;
        self.mark_changed();
        let msg = match distance {
            Some(a) => trs(&format!("Zahnräder erzeugt. Achsabstand {} {}.", units::num(a, 2), units::unit())),
            None => tr("Zahnrad erzeugt.").to_string(),
        };
        self.toast(ToastKind::Success, msg);
    }
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.gear.open {
        return;
    }
    let mut open = true;
    let mut create = false;
    egui::Window::new(trs(&format!("{} Zahnräder", icon::GEAR_FINE)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(420.0)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let t = &mut app.gear;
            egui::Grid::new("gear").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                ui.label(tr("Modul"));
                ui.add(DragValue::new(&mut t.spec.module).speed(0.05).range(0.3..=50.0).max_decimals(2).mm())
                    .on_hover_text(tr("Teilkreisdurchmesser geteilt durch die Zähnezahl. Nur Räder mit gleichem Modul passen zusammen."));
                ui.end_row();
                ui.label(tr("Zähne"));
                ui.add(DragValue::new(&mut t.spec.teeth).range(6..=300));
                ui.end_row();
                ui.label(tr("Bohrung"));
                ui.add(DragValue::new(&mut t.spec.bore).speed(0.1).range(0.0..=500.0).mm());
                ui.end_row();
                ui.label(tr("Gegenrad"));
                ui.horizontal(|ui| {
                    ui.checkbox(&mut t.pair, "");
                    ui.add_enabled_ui(t.pair, |ui| {
                        ui.add(DragValue::new(&mut t.teeth2).range(6..=300).suffix(tr(" Zähne")));
                        ui.label(tr("Bohrung"));
                        ui.add(DragValue::new(&mut t.bore2).speed(0.1).range(0.0..=500.0).mm());
                    });
                });
                ui.end_row();
                ui.label(tr("Eingriffswinkel"));
                ui.horizontal(|ui| {
                    for a in [14.5, 20.0, 25.0] {
                        ui.selectable_value(&mut t.spec.pressure_angle, a, format!("{a}°"));
                    }
                    ui.add(DragValue::new(&mut t.spec.pressure_angle).speed(0.1).range(10.0..=35.0).suffix("°"));
                });
                ui.end_row();
                ui.label(tr("Flankenspiel"));
                ui.add(DragValue::new(&mut t.spec.backlash).speed(0.01).range(0.0..=5.0).max_decimals(2).mm())
                    .on_hover_text(tr("Luft zwischen den Zähnen am Teilkreis; jedes Rad wird um die Hälfte dünner. Etwas mehr als die Ungenauigkeit von Laser und Material."));
                ui.end_row();
            });
            let check = t.check();
            let g1 = t.spec;
            let g2 = t.second();
            let num = |v: f64| units::num(v, 2);
            let mut info = trs(&format!(
                "Teilkreis-Ø {} · Kopfkreis-Ø {} {}",
                num(2.0 * g1.pitch_radius()),
                num(2.0 * g1.tip_radius()),
                units::unit()
            ));
            if let Some(g2) = g2 {
                info = trs(&format!(
                    "{info} · Achsabstand {} {} · Übersetzung 1 : {}",
                    num(g1.center_distance(&g2)),
                    units::unit(),
                    units::num(g2.teeth as f64 / g1.teeth as f64, 2)
                ));
            }
            ui.label(RichText::new(info).small().color(pal.text_weak));
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().max(380.0), 200.0), Sense::hover());
            ui.painter().rect_filled(rect, 6.0, pal.canvas);
            if check.is_ok() {
                // Paar im Eingriff: Rad 1 links, Gegenrad rechts davon
                let mut parts = vec![(g1, Affine::IDENTITY)];
                if let Some(g2) = g2 {
                    let a = g1.center_distance(&g2);
                    parts.push((g2, Affine::rotate(g1.mesh_rotation(&g2)).then(&Affine::translate(a, 0.0))));
                }
                let (left, right) = (-g1.tip_radius(), parts.last().map_or(0.0, |(g, t)| t.apply(Point::ZERO).x + g.tip_radius()));
                let half_h = parts.iter().map(|(g, _)| g.tip_radius()).fold(0.0, f64::max);
                let inner = rect.shrink(10.0);
                let scale = (inner.width() as f64 / (right - left)).min(inner.height() as f64 / (2.0 * half_h));
                let center = inner.center();
                let mid = (left + right) / 2.0;
                let to_screen = |p: Point| center + vec2(((p.x - mid) * scale) as f32, (p.y * scale) as f32);
                for (g, at) in &parts {
                    let c = at.apply(Point::ZERO);
                    ui.painter().circle_stroke(
                        to_screen(c),
                        (g.pitch_radius() * scale) as f32,
                        Stroke::new(0.6, pal.text_weak),
                    );
                    for q in g.outline().flatten(at, 0.3 / scale) {
                        let mut pts: Vec<egui::Pos2> = q.points.iter().map(|p| to_screen(*p)).collect();
                        if q.closed {
                            pts.push(pts[0]);
                        }
                        ui.painter().add(Shape::line(pts, Stroke::new(1.2, pal.accent)));
                    }
                }
            }
            match check {
                Err(e) => {
                    ui.label(RichText::new(tr(e)).color(pal.danger));
                }
                Ok(()) if g2.is_some_and(|g2| g1.interferes(&g2)) => {
                    ui.label(
                        RichText::new(tr(
                            "Achtung: Bei so wenigen Zähnen können die Zahnköpfe in den Zahnfuß des anderen Rades greifen und klemmen – mehr Zähne oder 25° Eingriffswinkel wählen.",
                        ))
                        .color(pal.warning),
                    );
                }
                Ok(()) => {}
            }
            ui.add_space(4.0);
            let label = if t.pair {
                trs(&format!("{} Zahnräder erzeugen", icon::GEAR_FINE))
            } else {
                trs(&format!("{} Zahnrad erzeugen", icon::GEAR_FINE))
            };
            if ui.add_enabled(check.is_ok(), Button::new(label)).clicked() {
                create = true;
            }
        });
    if create {
        app.insert_gears();
        app.gear.open = false;
    }
    if !open {
        app.gear.open = false;
    }
}
