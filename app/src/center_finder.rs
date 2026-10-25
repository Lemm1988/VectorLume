//! Mittelpunkt finden: den Laser an Punkte am Werkstückrand fahren, die Positionen übernehmen, daraus die Mitte
//! (Kreis durch die Randpunkte oder Mitte zweier Punkte) berechnen und anfahren, als Ursprung setzen oder als
//! Markierung ins Projekt übernehmen.

use crate::app::VectorLumeApp;
use crate::i18n::{tr, trs};
use crate::theme;
use egui::{Align2, Button, RichText, vec2};
use egui_phosphor::regular as icon;
use vl_cam::StartFrom;
use vl_core::{Point, VectorPath};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CenterMode {
    /// Kreis durch drei oder mehr Randpunkte (Ausgleichskreis).
    #[default]
    Circle,
    /// Mitte zwischen zwei Punkten, z. B. gegenüberliegenden Ecken.
    Midpoint,
}

#[derive(Default)]
pub struct CenterFinder {
    pub open: bool,
    pub mode: CenterMode,
    /// Übernommene Kopfpositionen in Arbeitskoordinaten.
    pub points: Vec<Point>,
}

impl CenterFinder {
    fn needed(&self) -> usize {
        match self.mode {
            CenterMode::Circle => 3,
            CenterMode::Midpoint => 2,
        }
    }

    /// Mitte in Arbeitskoordinaten und beim Kreis der Radius.
    pub fn result(&self) -> Option<(Point, Option<f64>)> {
        match self.mode {
            CenterMode::Circle => vl_core::geom::fit_circle(&self.points).map(|(c, r)| (c, Some(r))),
            CenterMode::Midpoint => match self.points[..] {
                [a, b] => Some((a.lerp(b, 0.5), None)),
                _ => None,
            },
        }
    }
}

enum Action {
    Take(Point),
    Remove(usize),
    Clear,
    MoveTo(Point),
    Origin(Point),
    Insert(Point, Option<f64>),
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.center_finder.open {
        return;
    }
    let mut open = true;
    let head = app.machine.head_position();
    let can_move =
        app.machine.connected && !app.machine.job_running && app.settings.profile.rotary.active_scale().is_none();
    let absolute = app.settings.job.start_from == StartFrom::Absolute;
    let mut action = None;
    egui::Window::new(trs(&format!("{} Mittelpunkt finden", icon::CROSSHAIR)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(380.0)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let cf = &mut app.center_finder;
            ui.label(
                RichText::new(tr(
                    "Laser im Reiter „Bewegen“ nacheinander an den Rand des Werkstücks fahren (Pilotlicht oder \
                     Fokuspunkt genau auf die Kante) und jede Stelle mit „Punkt übernehmen“ merken.",
                ))
                .small()
                .color(pal.text_weak),
            );
            ui.add_space(6.0);
            let before = cf.mode;
            ui.radio_value(
                &mut cf.mode,
                CenterMode::Circle,
                tr("Kreis: 3 oder mehr Punkte am Rand, möglichst rundum verteilt"),
            );
            ui.radio_value(&mut cf.mode, CenterMode::Midpoint, tr("Mitte zwischen 2 Punkten (z. B. Ecken)"));
            if cf.mode != before {
                cf.points.clear();
            }
            ui.add_space(6.0);
            match head {
                Some(p) => ui.label(trs(&format!(
                    "Laserkopf: X {} Y {}",
                    crate::units::num(p.x, 2),
                    crate::units::len(p.y, 2)
                ))),
                None => ui.label(RichText::new(tr("Position unbekannt – ist der Laser verbunden?")).color(pal.warning)),
            };
            let room = cf.mode == CenterMode::Circle || cf.points.len() < 2;
            if let Some(p) = head
                && ui
                    .add_enabled(can_move && room, Button::new(trs(&format!("{} Punkt übernehmen", icon::PLUS))))
                    .clicked()
            {
                action = Some(Action::Take(p));
            }
            if !cf.points.is_empty() {
                ui.add_space(4.0);
                egui::Grid::new("center_points").num_columns(3).spacing(vec2(12.0, 2.0)).show(ui, |ui| {
                    for (i, p) in cf.points.iter().enumerate() {
                        ui.label(format!("{}.", i + 1));
                        ui.label(format!("X {}   Y {}", crate::units::num(p.x, 2), crate::units::num(p.y, 2)));
                        if ui.small_button(icon::X).on_hover_text(tr("Punkt entfernen")).clicked() {
                            action = Some(Action::Remove(i));
                        }
                        ui.end_row();
                    }
                });
            }
            ui.separator();
            match cf.result() {
                Some((c, r)) => {
                    let (x, y) = (crate::units::num(c.x, 2), crate::units::len(c.y, 2));
                    ui.label(RichText::new(trs(&format!("Mitte: X {x} Y {y}"))).strong());
                    if let Some(r) = r {
                        ui.label(trs(&format!("Durchmesser: {}", crate::units::len(2.0 * r, 2))));
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(can_move, Button::new(trs(&format!("{} Zur Mitte fahren", icon::CROSSHAIR))))
                            .clicked()
                        {
                            action = Some(Action::MoveTo(c));
                        }
                        if ui
                            .button(trs(&format!("{} Als Benutzer-Ursprung", icon::MAP_PIN)))
                            .on_hover_text(tr("Jobs starten danach an dieser Stelle („Start von: Benutzer-Ursprung“)"))
                            .clicked()
                        {
                            action = Some(Action::Origin(c));
                        }
                        if ui
                            .add_enabled(absolute, Button::new(trs(&format!("{} Ins Projekt", icon::PLUS_SQUARE))))
                            .on_hover_text(tr("Gefundenen Kreis bzw. ein Mittelkreuz auf der aktuellen Ebene einfügen"))
                            .on_disabled_hover_text(tr("Nur mit „Start von: Absolute Koordinaten“"))
                            .clicked()
                        {
                            action = Some(Action::Insert(c, r));
                        }
                    });
                }
                None if cf.mode == CenterMode::Circle && cf.points.len() >= 3 => {
                    ui.label(RichText::new(tr("Die Punkte liegen auf einer Geraden.")).color(pal.warning));
                }
                None => {
                    let missing = cf.needed().saturating_sub(cf.points.len());
                    ui.label(RichText::new(trs(&format!("Noch {missing} Punkt(e) nötig"))).color(pal.text_weak));
                }
            }
            if !cf.points.is_empty() && ui.button(tr("Zurücksetzen")).clicked() {
                action = Some(Action::Clear);
            }
        });
    if !open {
        app.center_finder.open = false;
    }
    match action {
        Some(Action::Take(p)) => app.center_finder.points.push(p),
        Some(Action::Remove(i)) => {
            app.center_finder.points.remove(i);
        }
        Some(Action::Clear) => app.center_finder.points.clear(),
        Some(Action::MoveTo(c)) => {
            let (x, y) = (vl_cam::gcode::fmt_mm(c.x), vl_cam::gcode::fmt_mm(c.y));
            app.send_jog(false, &format!("X{x} Y{y}"));
        }
        Some(Action::Origin(c)) => app.set_user_origin(c),
        Some(Action::Insert(c, r)) => {
            let at = app.settings.profile.machine_to_doc(c);
            let (name, path) = match r {
                Some(r) => (tr("Gefundener Kreis"), VectorPath::ellipse(at, r, r)),
                None => (tr("Mittelpunkt"), cross(at, 5.0)),
            };
            app.checkpoint();
            let id = app.doc.add_path(name, app.current_layer, path);
            app.selection = vec![id];
            app.mark_changed();
        }
        None => {}
    }
}

/// Mittelkreuz aus zwei Linien mit halber Länge `half`.
fn cross(c: Point, half: f64) -> VectorPath {
    let mut pb = vl_core::PathBuilder::new();
    pb.move_to(Point::new(c.x - half, c.y));
    pb.line_to(Point::new(c.x + half, c.y));
    pb.move_to(Point::new(c.x, c.y - half));
    pb.line_to(Point::new(c.x, c.y + half));
    pb.finish()
}

impl VectorLumeApp {
    /// Öffnet „Mittelpunkt finden“ und zeigt rechts die Bewegungssteuerung.
    pub fn open_center_finder(&mut self) {
        self.center_finder.open = true;
        self.right_tab = crate::ui_side::RightTab::Jog;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_need_their_points() {
        let mut cf = CenterFinder { points: vec![Point::new(10.0, 0.0), Point::new(0.0, 10.0)], ..Default::default() };
        assert!(cf.result().is_none(), "Kreis braucht drei Punkte");
        cf.points.push(Point::new(-10.0, 0.0));
        let (c, r) = cf.result().unwrap();
        assert!(c.distance(Point::ZERO) < 1e-9 && (r.unwrap() - 10.0).abs() < 1e-9);
        cf.mode = CenterMode::Midpoint;
        assert!(cf.result().is_none(), "Mitte nur aus genau zwei Punkten");
        cf.points.pop();
        assert_eq!(cf.result(), Some((Point::new(5.0, 5.0), None)));
    }
}
