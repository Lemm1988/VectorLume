//! Laschen von Hand setzen: Klick auf eine geschlossene Kontur setzt eine Lasche, Klick auf eine vorhandene entfernt
//! sie. Die Laschen sitzen in Objektkoordinaten und wandern mit dem Objekt; ihre Breite kommt von der Ebene.

use crate::app::{ToastKind, Tool, VectorLumeApp};
use crate::canvas::ViewTf;
use crate::i18n::{tr, trs};
use crate::theme;
use egui::{Painter, RichText, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;
use vl_core::{ObjectId, Point};

/// Was ein Klick an der Mausposition bewirkt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TabHit {
    /// Neue Lasche an `at` (Dokument), im Objekt bei `local`.
    Add { id: ObjectId, at: Point, local: Point },
    /// Vorhandene Lasche entfernen.
    Remove { id: ObjectId, index: usize, at: Point },
}

impl VectorLumeApp {
    /// Treffer im Abstand `tol` (mm); vorhandene Laschen haben Vorrang vor neuen.
    pub(crate) fn tab_hit(&mut self, p: Point, tol: f64) -> Option<TabHit> {
        let mut remove: Option<(f64, TabHit)> = None;
        for obj in self.doc.objects.iter().filter(|o| self.doc.layers[o.layer].selectable()) {
            for (index, t) in obj.tabs.iter().enumerate() {
                let at = obj.transform.apply(*t);
                let d = at.distance(p);
                if d <= tol * 1.5 && remove.is_none_or(|r| d < r.0) {
                    remove = Some((d, TabHit::Remove { id: obj.id, index, at }));
                }
            }
        }
        if let Some((_, hit)) = remove {
            return Some(hit);
        }
        let mut best: Option<(f64, TabHit)> = None;
        for obj in &self.doc.objects {
            if !self.doc.layers[obj.layer].selectable() {
                continue;
            }
            let Some((polys, bounds)) = self.geom.local(obj) else { continue };
            if !obj.transform.transform_rect(&bounds).expanded(tol).contains(p) {
                continue;
            }
            let Some(inv) = obj.transform.inverse() else { continue };
            let local = inv.apply(p);
            for poly in polys.iter().filter(|q| q.closed) {
                let Some((q, _)) = poly.closest(local) else { continue };
                let at = obj.transform.apply(q);
                let d = at.distance(p);
                if d <= tol && best.is_none_or(|b| d < b.0) {
                    best = Some((d, TabHit::Add { id: obj.id, at, local: q }));
                }
            }
        }
        best.map(|b| b.1)
    }

    pub(crate) fn apply_tab_hit(&mut self, hit: TabHit) {
        self.checkpoint();
        match hit {
            TabHit::Add { id, local, .. } => {
                if let Some(o) = self.doc.get_mut(id) {
                    o.tabs.push(local);
                }
            }
            TabHit::Remove { id, index, .. } => {
                if let Some(o) = self.doc.get_mut(id)
                    && index < o.tabs.len()
                {
                    o.tabs.remove(index);
                }
            }
        }
        self.tab_hover = None;
        self.mark_changed();
    }

    /// Objekte, deren Laschen „Entfernen“ betrifft: die Auswahl, ohne Auswahl alle.
    fn tab_targets(&self) -> Vec<ObjectId> {
        let ids = if self.selection.is_empty() {
            self.doc.objects.iter().map(|o| o.id).collect()
        } else {
            self.selection.clone()
        };
        ids.into_iter().filter(|id| self.doc.get(*id).is_some_and(|o| !o.tabs.is_empty())).collect()
    }

    pub(crate) fn clear_tabs(&mut self) {
        let ids = self.tab_targets();
        if ids.is_empty() {
            return;
        }
        self.checkpoint();
        let mut count = 0;
        for id in ids {
            if let Some(o) = self.doc.get_mut(id) {
                count += o.tabs.len();
                o.tabs.clear();
            }
        }
        self.mark_changed();
        self.toast(ToastKind::Info, trs(&format!("{count} Laschen entfernt.")));
    }

    /// Laschen sichtbarer Objekte als Lücke mit Ring (Durchmesser = Laschenbreite der Ebene, mindestens 6 Pixel);
    /// im Laschenwerkzeug dazu die Vorschau unter dem Mauszeiger.
    pub(crate) fn draw_tabs(&self, painter: &Painter, tf: &ViewTf, pal: &theme::Palette) {
        let radius =
            |layer: usize| (self.doc.layers[layer].settings.tab_width * self.view.zoom as f64 / 2.0).max(3.0) as f32;
        for obj in &self.doc.objects {
            if obj.tabs.is_empty() || !self.doc.layers[obj.layer].visible {
                continue;
            }
            let r = radius(obj.layer);
            for t in &obj.tabs {
                painter.circle(tf.to_screen(obj.transform.apply(*t)), r, pal.bed, Stroke::new(1.5, pal.warning));
            }
        }
        if self.tool != Tool::Tabs {
            return;
        }
        match self.tab_hover.as_ref().and_then(|h| h.2) {
            Some(TabHit::Add { id, at, .. }) => {
                let r = self.doc.get(id).map_or(3.0, |o| radius(o.layer));
                painter.circle_stroke(tf.to_screen(at), r + 1.5, Stroke::new(2.0, pal.accent));
            }
            Some(TabHit::Remove { id, at, .. }) => {
                let (c, r) = (tf.to_screen(at), self.doc.get(id).map_or(3.0, |o| radius(o.layer)) + 2.0);
                let stroke = Stroke::new(2.0, pal.danger);
                painter.circle_stroke(c, r, stroke);
                let d = r * 0.6;
                painter.line_segment([c - vec2(d, d), c + vec2(d, d)], stroke);
                painter.line_segment([c - vec2(d, -d), c + vec2(d, -d)], stroke);
            }
            None => {}
        }
    }
}

/// Werkzeugleiste des Laschenwerkzeugs.
pub fn options(app: &mut VectorLumeApp, ui: &mut Ui) {
    let targets = app.tab_targets();
    let width = app.doc.layers[app.current_layer].settings.tab_width;
    ui.label(
        RichText::new(trs(&format!(
            "Klick auf Kontur: Lasche setzen · Klick auf Lasche: entfernen · Breite {}",
            crate::units::len(width, 2)
        )))
        .color(theme::of(ui).text_weak),
    )
    .on_hover_text(tr(
        "Die Breite stellt die Ebene ein (Laschen). Gesetzte Laschen gelten im Modus Linie und ersetzen an ihrer Kontur die automatisch verteilten; sie wandern mit dem Objekt.",
    ));
    let text =
        if app.selection.is_empty() { tr("Alle Laschen entfernen") } else { tr("Laschen der Auswahl entfernen") };
    if ui.add_enabled(!targets.is_empty(), egui::Button::new(format!("{} {text}", icon::TRASH))).clicked() {
        app.clear_tabs();
    }
}
