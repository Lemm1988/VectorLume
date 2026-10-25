//! Schnittreihenfolge per Klick: Formen nacheinander anklicken legt fest, in welcher Reihenfolge sie innerhalb
//! ihrer Ebene gelasert werden. Nummern auf der Arbeitsfläche zeigen das Ergebnis schon während des Klickens;
//! Eingabe übernimmt, Rücktaste nimmt den letzten Klick zurück, Esc bricht ab.

use crate::app::{ToastKind, VectorLumeApp};
use crate::canvas::ViewTf;
use crate::i18n::{tr, trs};
use crate::theme;
use egui::{Align2, Color32, CursorIcon, FontId, PointerButton, Stroke};
use std::collections::HashMap;
use vl_core::{Document, ObjectId};

/// Laufende Auswahl der Reihenfolge.
#[derive(Default)]
pub struct OrderPick {
    /// Angeklickte Formen in Klickfolge.
    pub picked: Vec<ObjectId>,
    pub hover: Option<ObjectId>,
}

/// Objekte, die gelasert werden (sichtbare Ebenen mit Ausgabe).
fn lasered(doc: &Document) -> impl Iterator<Item = &vl_core::Object> {
    doc.objects.iter().filter(|o| {
        let layer = &doc.layers[o.layer];
        layer.visible && layer.settings.output
    })
}

/// Reihenfolge aller Objekte nach dem Übernehmen: angeklickte zuerst (in Klickfolge), dann die übrigen wie bisher.
/// Ohne Optimierung bestimmt diese Reihenfolge, was innerhalb einer Ebene zuerst gelasert wird.
pub fn planned_order(doc: &Document, picked: &[ObjectId]) -> Vec<ObjectId> {
    let mut order: Vec<ObjectId> = picked.iter().copied().filter(|&id| doc.get(id).is_some()).collect();
    order.extend(doc.objects.iter().map(|o| o.id).filter(|id| !picked.contains(id)));
    order
}

/// Nummer jedes gelaserten Objekts innerhalb seiner Ebene (1 = zuerst).
pub fn numbers(doc: &Document, picked: &[ObjectId]) -> HashMap<ObjectId, usize> {
    let pos: HashMap<ObjectId, usize> =
        planned_order(doc, picked).into_iter().enumerate().map(|(i, id)| (id, i)).collect();
    let mut objs: Vec<_> = lasered(doc).collect();
    objs.sort_by_key(|o| pos[&o.id]);
    let mut next: HashMap<usize, usize> = HashMap::new();
    objs.iter()
        .map(|o| {
            let n = next.entry(o.layer).or_insert(0);
            *n += 1;
            (o.id, *n)
        })
        .collect()
}

impl VectorLumeApp {
    pub(crate) fn start_cut_order(&mut self) {
        if lasered(&self.doc).next().is_none() {
            self.toast(ToastKind::Info, tr("Es gibt keine Formen auf Ebenen mit Ausgabe."));
            return;
        }
        self.tool = crate::app::Tool::Select;
        self.cut_order = Some(OrderPick::default());
        self.toast(
            ToastKind::Info,
            tr(
                "Formen in der gewünschten Reihenfolge anklicken (erneut klicken nimmt sie heraus). Eingabe übernimmt, \
                Rücktaste nimmt den letzten Klick zurück, Esc bricht ab.",
            ),
        );
    }

    /// Übernimmt die angeklickte Reihenfolge und schaltet die Optimierung aus, damit sie gilt.
    pub(crate) fn apply_cut_order(&mut self) {
        let Some(pick) = self.cut_order.take() else { return };
        if pick.picked.is_empty() {
            return;
        }
        let pos: HashMap<ObjectId, usize> =
            planned_order(&self.doc, &pick.picked).into_iter().enumerate().map(|(i, id)| (id, i)).collect();
        self.checkpoint();
        self.doc.objects.sort_by_key(|o| pos[&o.id]);
        self.mark_changed();
        if self.settings.job.optimize_order {
            self.settings.job.optimize_order = false;
            self.settings.save();
            self.toast(
                ToastKind::Info,
                tr("Reihenfolge übernommen. „Schnittreihenfolge optimieren“ wurde ausgeschaltet, damit sie beim Lasern gilt."),
            );
        } else {
            self.toast(ToastKind::Success, tr("Reihenfolge übernommen."));
        }
    }

    /// Bedienung auf der Arbeitsfläche; `true` = Ereignis verbraucht. Tasten behandelt `ui_main::shortcuts`.
    pub(crate) fn cut_order_interaction(
        &mut self,
        ctx: &egui::Context,
        resp: &egui::Response,
        tf: &ViewTf,
        space: bool,
        tol: f64,
    ) -> bool {
        if self.cut_order.is_none() {
            return false;
        }
        if space {
            return true;
        }
        let hover = resp
            .hover_pos()
            .and_then(|h| self.hit_test(tf.to_doc(h), tol))
            .filter(|&id| self.doc.get(id).is_some_and(|o| self.doc.layers[o.layer].settings.output));
        let Some(pick) = self.cut_order.as_mut() else { return true };
        pick.hover = hover;
        if hover.is_some() {
            ctx.set_cursor_icon(CursorIcon::PointingHand);
        }
        if resp.clicked_by(PointerButton::Primary)
            && let Some(id) = hover
        {
            match pick.picked.iter().position(|&p| p == id) {
                Some(i) => {
                    pick.picked.remove(i);
                }
                None => pick.picked.push(id),
            }
        }
        true
    }

    pub(crate) fn draw_cut_order(&self, painter: &egui::Painter, tf: &ViewTf, pal: &theme::Palette) {
        let Some(pick) = &self.cut_order else { return };
        let numbers = numbers(&self.doc, &pick.picked);
        let font = FontId::proportional(12.0);
        for o in lasered(&self.doc) {
            let Some(&n) = numbers.get(&o.id) else { continue };
            let center = tf.to_screen(o.bounds().center());
            let color = theme::layer_color(self.doc.layers[o.layer].color);
            let picked = pick.picked.contains(&o.id);
            let radius = if n >= 100 { 13.0 } else { 10.0 };
            if pick.hover == Some(o.id) {
                painter.circle_stroke(center, radius + 3.0, Stroke::new(2.0, pal.accent));
            }
            // angeklickt: gefüllt in der Ebenenfarbe; noch offen: nur umrandet
            let (fill, text) = if picked { (color, readable_on(color)) } else { (pal.panel, pal.text_weak) };
            painter.circle(center, radius, fill, Stroke::new(1.5, color));
            painter.text(center, Align2::CENTER_CENTER, n.to_string(), font.clone(), text);
        }
        // dauerhafter Hinweis oben auf der Arbeitsfläche (die Meldung verschwindet nach einigen Sekunden)
        let hint = trs(&format!(
            "Schnittreihenfolge: {} von {} angeklickt · Eingabe übernimmt · Rücktaste zurück · Esc bricht ab",
            pick.picked.len(),
            numbers.len()
        ));
        let top = painter.clip_rect().center_top() + egui::vec2(0.0, 12.0);
        let galley = painter.layout_no_wrap(hint, FontId::proportional(13.0), pal.text);
        let rect = Align2::CENTER_TOP.anchor_size(top, galley.size()).expand2(egui::vec2(10.0, 5.0));
        painter.rect(rect, 6.0, pal.panel, Stroke::new(1.0, pal.accent), egui::StrokeKind::Outside);
        painter.galley(rect.min + egui::vec2(10.0, 5.0), galley, pal.text);
    }
}

/// Schwarz oder Weiß, je nachdem was auf `bg` besser lesbar ist.
fn readable_on(bg: Color32) -> Color32 {
    let luma = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if luma > 150.0 { Color32::BLACK } else { Color32::WHITE }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::VectorPath;

    #[test]
    fn picked_shapes_come_first_and_numbers_count_per_layer() {
        let mut doc = Document::new();
        let a = doc.add_path("a", 0, VectorPath::rect(0.0, 0.0, 5.0, 5.0, 0.0));
        let b = doc.add_path("b", 1, VectorPath::rect(10.0, 0.0, 5.0, 5.0, 0.0));
        let c = doc.add_path("c", 0, VectorPath::rect(20.0, 0.0, 5.0, 5.0, 0.0));
        let d = doc.add_path("d", 0, VectorPath::rect(30.0, 0.0, 5.0, 5.0, 0.0));
        assert_eq!(planned_order(&doc, &[d, b]), vec![d, b, a, c]);
        let n = numbers(&doc, &[d, b]);
        // Ebene 0: d, a, c; Ebene 1: b
        assert_eq!((n[&d], n[&a], n[&c], n[&b]), (1, 2, 3, 1));
        // Ebenen ohne Ausgabe bekommen keine Nummer
        doc.layers[1].settings.output = false;
        assert!(!numbers(&doc, &[d, b]).contains_key(&b));
    }
}
