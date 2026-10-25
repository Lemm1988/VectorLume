//! Fenster „Bearbeitungsverlauf“: alle gemerkten Stände mit ihrer Änderung, ein Klick springt zu einem Stand.

use crate::app::VectorLumeApp;
use crate::i18n::{tr, trs};
use crate::theme;
use egui::{Align2, RichText, vec2};
use egui_phosphor::regular as icon;
use vl_core::changes::{Affected, ChangeSummary};

#[derive(Default)]
pub struct HistoryWindow {
    pub open: bool,
    /// Änderung zum aktuellen Stand, gemerkt je (Dokumentrevision, Verlaufslänge) – sonst bei jedem Bild ein Vergleich.
    current: Option<(u64, usize, ChangeSummary)>,
}

/// Beschriftung eines Schritts, z. B. „Verschoben: 3 · Ebene gewechselt: Kreis“.
pub fn label(c: &ChangeSummary) -> String {
    let mut parts = Vec::new();
    let mut part = |a: &Affected, what: fn(&str) -> String| {
        if a.count == 1 && !a.name.is_empty() {
            parts.push(what(&a.name));
        } else if a.count > 0 {
            parts.push(what(&trs(&format!("{} Objekte", a.count))));
        }
    };
    part(&c.added, |v| trs(&format!("Hinzugefügt: {v}")));
    part(&c.removed, |v| trs(&format!("Gelöscht: {v}")));
    part(&c.moved, |v| trs(&format!("Verschoben: {v}")));
    part(&c.transformed, |v| trs(&format!("Skaliert/gedreht: {v}")));
    part(&c.relayered, |v| trs(&format!("Ebene gewechselt: {v}")));
    part(&c.regrouped, |v| trs(&format!("Gruppierung geändert: {v}")));
    part(&c.edited, |v| trs(&format!("Bearbeitet: {v}")));
    if c.reordered {
        parts.push(tr("Reihenfolge geändert").to_string());
    }
    if c.layers {
        parts.push(tr("Ebeneneinstellungen geändert").to_string());
    }
    if parts.is_empty() {
        return tr("Keine sichtbare Änderung").to_string();
    }
    parts.join(" · ")
}

/// Springt zum Stand mit dem Index `target` (0 = ältester gemerkter Stand).
pub fn jump(app: &mut VectorLumeApp, target: usize) {
    let current = app.history.undo_len();
    for _ in target..current {
        app.undo();
    }
    for _ in current..target {
        app.redo();
    }
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.history_window.open {
        app.history_window.current = None;
        return;
    }
    let current = app.history.undo_len();
    let key = (app.revision, current);
    let live = match &app.history_window.current {
        Some((r, n, c)) if (*r, *n) == key => c.clone(),
        _ => {
            let c = app.history.change_to(&app.doc);
            app.history_window.current = Some((key.0, key.1, c.clone()));
            c
        }
    };
    let first = if app.history.truncated() { tr("Ältester gemerkter Stand") } else { tr("Ausgangszustand") };
    let mut rows: Vec<String> = Vec::with_capacity(current + 1);
    rows.push(first.to_string());
    rows.extend(app.history.undo_changes().skip(1).map(label));
    if current > 0 {
        rows.push(label(&live));
    }
    let redo_from = rows.len();
    rows.extend(app.history.redo_changes().map(label));

    let mut open = true;
    let mut clicked = None;
    egui::Window::new(trs(&format!("{} Bearbeitungsverlauf", icon::CLOCK_COUNTER_CLOCKWISE)))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(380.0, 420.0))
        .pivot(Align2::RIGHT_TOP)
        .default_pos(ctx.content_rect().right_top() + vec2(-330.0, 80.0))
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            ui.label(
                RichText::new(tr(
                    "Klick auf einen Eintrag stellt diesen Stand wieder her; spätere Schritte bleiben erhalten, bis \
                     etwas Neues geändert wird. Notizen sind nicht betroffen.",
                ))
                .small()
                .color(pal.text_weak),
            );
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                for (i, text) in rows.iter().enumerate() {
                    let mut rich = RichText::new(format!("{:>3}  {text}", i));
                    if i >= redo_from {
                        rich = rich.color(pal.text_weak).italics();
                    }
                    if ui.selectable_label(i == current, rich).clicked() && i != current {
                        clicked = Some(i);
                    }
                }
            });
        });
    if let Some(i) = clicked {
        jump(app, i);
    }
    app.history_window.open &= open;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_name_single_objects_and_count_many() {
        let mut c = ChangeSummary::default();
        assert_eq!(label(&c), "Keine sichtbare Änderung");
        c.moved = Affected { count: 1, name: "Kreis".into() };
        c.relayered = Affected { count: 3, name: "Text".into() };
        c.layers = true;
        assert_eq!(label(&c), "Verschoben: Kreis · Ebene gewechselt: 3 Objekte · Ebeneneinstellungen geändert");
    }
}
