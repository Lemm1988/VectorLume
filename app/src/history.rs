//! Rückgängig/Wiederholen über Dokument-Schnappschüsse (Bilder werden dabei nur referenziert). Jeder Stand merkt
//! sich, was sich gegenüber dem Stand davor geändert hat – für das Fenster „Bearbeitungsverlauf“.

use vl_core::Document;
use vl_core::changes::{ChangeSummary, summarize};

const LIMIT: usize = 200;

#[derive(Default)]
pub struct History {
    /// Frühere Stände (ältester zuerst), jeweils mit der Änderung gegenüber dem Stand davor.
    undo: Vec<(Document, ChangeSummary)>,
    /// Rückgängig gemachte Stände (der nächste zuoberst), ebenso mit ihrer Änderung.
    redo: Vec<(Document, ChangeSummary)>,
    /// Ältere Stände wurden wegen der Begrenzung verworfen.
    truncated: bool,
}

impl History {
    /// Vor einer Änderung aufrufen.
    pub fn checkpoint(&mut self, doc: &Document) {
        let last = self.undo.last().map(|(d, _)| d);
        if last == Some(doc) {
            return;
        }
        let change = last.map(|d| summarize(d, doc)).unwrap_or_default();
        self.undo.push((doc.clone(), change));
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
            self.truncated = true;
        }
        self.redo.clear();
    }

    /// Entfernt den letzten Schnappschuss, wenn sich danach nichts geändert hat.
    pub fn discard_if_unchanged(&mut self, doc: &Document) {
        if self.undo.last().map(|(d, _)| d) == Some(doc) {
            self.undo.pop();
        }
    }

    pub fn undo(&mut self, doc: &mut Document) -> bool {
        let Some((prev, _)) = self.undo.pop() else { return false };
        let change = summarize(&prev, doc);
        self.redo.push((std::mem::replace(doc, prev), change));
        true
    }

    pub fn redo(&mut self, doc: &mut Document) -> bool {
        let Some((next, _)) = self.redo.pop() else { return false };
        let change = self.change_to(doc);
        self.undo.push((std::mem::replace(doc, next), change));
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.truncated = false;
    }

    /// Anzahl der Schritte, die sich rückgängig machen lassen (= Index des aktuellen Stands im Verlauf).
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Änderung vom letzten gemerkten Stand zu `doc` (dem aktuellen).
    pub fn change_to(&self, doc: &Document) -> ChangeSummary {
        self.undo.last().map(|(d, _)| summarize(d, doc)).unwrap_or_default()
    }

    /// Änderungen der früheren Stände, ältester zuerst (beim ältesten ohne Bedeutung).
    pub fn undo_changes(&self) -> impl Iterator<Item = &ChangeSummary> {
        self.undo.iter().map(|(_, c)| c)
    }

    /// Änderungen der rückgängig gemachten Stände in der Reihenfolge, in der „Wiederholen“ sie herstellt.
    pub fn redo_changes(&self) -> impl Iterator<Item = &ChangeSummary> {
        self.redo.iter().rev().map(|(_, c)| c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::VectorPath;

    #[test]
    fn undo_redo_cycle() {
        let mut h = History::default();
        let mut doc = Document::new();
        h.checkpoint(&doc);
        doc.add_path("a", 0, VectorPath::rect(0.0, 0.0, 1.0, 1.0, 0.0));
        assert!(h.undo(&mut doc));
        assert!(doc.objects.is_empty());
        assert!(h.redo(&mut doc));
        assert_eq!(doc.objects.len(), 1);
        assert!(!h.redo(&mut doc));
    }

    /// Jeder Stand kennt seine Änderung, auch nach Hin- und Herspringen.
    #[test]
    fn changes_follow_the_states() {
        let mut h = History::default();
        let mut doc = Document::new();
        h.checkpoint(&doc);
        doc.add_path("a", 0, VectorPath::rect(0.0, 0.0, 1.0, 1.0, 0.0));
        h.checkpoint(&doc);
        doc.add_path("b", 0, VectorPath::rect(0.0, 0.0, 1.0, 1.0, 0.0));
        h.checkpoint(&doc);
        doc.objects.clear();
        // Hinzugefügter Name bzw. Zahl der gelöschten Objekte je Schritt (ohne den Ausgangszustand)
        let steps = |h: &History, doc: &Document| {
            let current = h.change_to(doc);
            let all = h.undo_changes().skip(1).chain([&current]).chain(h.redo_changes());
            all.map(|c| format!("{}{}", c.added.name, c.removed.count)).collect::<Vec<_>>()
        };
        let expected = ["a0", "b0", "2"];
        assert_eq!(steps(&h, &doc), expected);
        h.undo(&mut doc);
        h.undo(&mut doc);
        assert_eq!(h.undo_len(), 1);
        assert_eq!(steps(&h, &doc), expected);
        h.redo(&mut doc);
        assert_eq!((h.undo_len(), doc.objects.len()), (2, 2));
        assert_eq!(steps(&h, &doc), expected);
        assert!(!h.truncated());
    }
}
