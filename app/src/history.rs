//! Rückgängig/Wiederholen über Dokument-Schnappschüsse (Bilder werden dabei nur referenziert).

use vl_core::Document;

const LIMIT: usize = 200;

#[derive(Default)]
pub struct History {
    undo: Vec<Document>,
    redo: Vec<Document>,
}

impl History {
    /// Vor einer Änderung aufrufen.
    pub fn checkpoint(&mut self, doc: &Document) {
        if self.undo.last() == Some(doc) {
            return;
        }
        self.undo.push(doc.clone());
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Entfernt den letzten Schnappschuss, wenn sich danach nichts geändert hat.
    pub fn discard_if_unchanged(&mut self, doc: &Document) {
        if self.undo.last() == Some(doc) {
            self.undo.pop();
        }
    }

    pub fn undo(&mut self, doc: &mut Document) -> bool {
        let Some(prev) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(doc, prev));
        true
    }

    pub fn redo(&mut self, doc: &mut Document) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(doc, next));
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
}
