//! Tangentialkreise: aus drei ausgewählten Kreisen alle berührenden Kreise berechnen und als helle Vorschläge auf
//! der Arbeitsfläche zeigen. Ein Klick übernimmt einen (mit Umschalt mehrere), Esc beendet.

use crate::app::{ToastKind, VectorLumeApp};
use crate::canvas::ViewTf;
use crate::i18n::tr;
use crate::theme;
use egui::{CursorIcon, PointerButton, Stroke};
use vl_core::tangent::{Circle, tangent_circles};
use vl_core::{Object, Shape, VectorPath};

/// Vorschläge, die auf einen Klick warten.
pub struct TangentPick {
    pub candidates: Vec<Circle>,
    pub hover: Option<usize>,
}

/// Mittelpunkt und Radius, falls das Objekt (mit seiner Transformation) ein Kreis ist – auch importierte Kreise
/// ohne Grundform-Angabe, aber keine Ellipsen.
pub fn as_circle(o: &Object) -> Option<Circle> {
    let Shape::Path(path) = &o.shape else { return None };
    let polys = path.flatten(&o.transform, 0.005);
    let [poly] = polys.as_slice() else { return None };
    if !poly.closed || poly.points.len() < 8 {
        return None;
    }
    let (c, r) = vl_core::geom::fit_circle(&poly.points)?;
    let worst = poly.points.iter().map(|p| (p.distance(c) - r).abs()).fold(0.0, f64::max);
    (r > 0.0 && worst <= (0.002 * r).max(0.01)).then_some((c, r))
}

impl VectorLumeApp {
    /// Die drei ausgewählten Kreise, sonst `None`.
    pub(crate) fn selected_circles(&self) -> Option<[Circle; 3]> {
        if self.selection.len() != 3 {
            return None;
        }
        let circles: Vec<Circle> = self.doc.objects_in(&self.selection).filter_map(as_circle).collect();
        circles.try_into().ok()
    }

    pub(crate) fn start_tangent_circles(&mut self) {
        let Some(circles) = self.selected_circles() else {
            self.toast(ToastKind::Info, tr("Bitte genau drei Kreise auswählen."));
            return;
        };
        let candidates = tangent_circles(circles);
        if candidates.is_empty() {
            self.toast(ToastKind::Info, tr("Zu diesen Kreisen gibt es keinen berührenden Kreis."));
            return;
        }
        self.tangent = Some(TangentPick { candidates, hover: None });
        self.toast(
            ToastKind::Info,
            tr("Vorschlag anklicken, um ihn zu übernehmen – mit Umschalt mehrere. Esc beendet."),
        );
    }

    fn add_tangent_circle(&mut self, (c, r): Circle) {
        self.checkpoint();
        let id = self.doc.add_path(tr("Kreis"), self.current_layer, VectorPath::ellipse(c, r, r));
        self.selection = vec![id];
        self.mark_changed();
    }

    /// Bedienung, solange Vorschläge angezeigt werden; `true` = Ereignis verbraucht.
    pub(crate) fn tangent_interaction(
        &mut self,
        ctx: &egui::Context,
        resp: &egui::Response,
        tf: &ViewTf,
        space: bool,
    ) -> bool {
        let Some(pick) = self.tangent.as_mut() else { return false };
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.tangent = None;
            return true;
        }
        if space {
            return true;
        }
        pick.hover = resp.hover_pos().and_then(|h| {
            let q = tf.to_doc(h);
            let px = tf.affine().mean_scale();
            // nächste Kontur bis 10 Pixel, sonst der kleinste Kreis unter dem Zeiger
            let near = pick
                .candidates
                .iter()
                .enumerate()
                .map(|(i, (c, r))| (i, (q.distance(*c) - r).abs() * px))
                .filter(|(_, d)| *d <= 10.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i);
            near.or_else(|| pick.candidates.iter().position(|(c, r)| q.distance(*c) <= *r))
        });
        if pick.hover.is_some() {
            ctx.set_cursor_icon(CursorIcon::PointingHand);
        }
        if resp.clicked_by(PointerButton::Primary)
            && let Some(i) = pick.hover
        {
            let circle = pick.candidates[i];
            let keep = ctx.input(|i| i.modifiers.shift);
            if !keep {
                self.tangent = None;
            }
            self.add_tangent_circle(circle);
        }
        true
    }

    pub(crate) fn draw_tangent(&self, painter: &egui::Painter, tf: &ViewTf, pal: &theme::Palette) {
        let Some(pick) = &self.tangent else { return };
        let px = tf.affine().mean_scale() as f32;
        for (i, (c, r)) in pick.candidates.iter().enumerate() {
            let hovered = pick.hover == Some(i);
            let color = if hovered { pal.danger } else { pal.danger.gamma_multiply(0.45) };
            painter.circle_stroke(
                tf.to_screen(*c),
                *r as f32 * px,
                Stroke::new(if hovered { 2.0 } else { 1.0 }, color),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::{Affine, Document, Point};

    #[test]
    fn recognizes_circles_but_not_ellipses() {
        let mut doc = Document::new();
        let a = doc.add_path("a", 0, VectorPath::ellipse(Point::new(10.0, 10.0), 5.0, 5.0));
        let b = doc.add_path("b", 0, VectorPath::ellipse(Point::new(10.0, 10.0), 5.0, 3.0));
        let c = doc.add_path("c", 0, VectorPath::rect(0.0, 0.0, 10.0, 10.0, 0.0));
        // gleichmäßig skaliert und verschoben bleibt es ein Kreis
        doc.transform_objects(&[a], &Affine::scale(2.0, 2.0).then(&Affine::translate(1.0, 0.0)));
        let (center, r) = as_circle(doc.get(a).unwrap()).unwrap();
        // Kreise aus vier Bézierkurven weichen um weniger als 0,03 % vom echten Kreis ab
        assert!(center.distance(Point::new(21.0, 20.0)) < 1e-6 && (r - 10.0).abs() < 0.003, "{center:?} {r}");
        assert!(as_circle(doc.get(b).unwrap()).is_none());
        assert!(as_circle(doc.get(c).unwrap()).is_none());
    }
}
