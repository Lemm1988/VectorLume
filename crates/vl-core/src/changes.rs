//! Was sich zwischen zwei Dokumentständen geändert hat – für die Einträge im Bearbeitungsverlauf.

use std::collections::HashMap;

use crate::document::{Document, Object};

/// Anzahl betroffener Objekte; bei genau einem auch dessen Name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Affected {
    pub count: usize,
    pub name: String,
}

impl Affected {
    fn add(&mut self, o: &Object) {
        if self.count == 0 {
            self.name = o.name.clone();
        }
        self.count += 1;
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChangeSummary {
    pub added: Affected,
    pub removed: Affected,
    /// Nur verschoben (Lage geändert, Größe und Drehung gleich).
    pub moved: Affected,
    /// Skaliert, gedreht, gespiegelt oder geschert.
    pub transformed: Affected,
    /// Nur auf eine andere Ebene gelegt.
    pub relayered: Affected,
    /// Nur Gruppenzugehörigkeit geändert (gruppiert, Gruppe aufgehoben).
    pub regrouped: Affected,
    /// Form, Text, Name, Leistung, Laschen … oder mehreres zugleich.
    pub edited: Affected,
    /// Gleiche Objekte, andere Reihenfolge.
    pub reordered: bool,
    /// Ebeneneinstellungen oder Ebenenreihenfolge.
    pub layers: bool,
}

impl ChangeSummary {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Vergleicht zwei Stände über die Objekt-IDs (Notizen zählen nicht, sie gehören nicht zum Verlauf).
pub fn summarize(before: &Document, after: &Document) -> ChangeSummary {
    let mut s = ChangeSummary {
        layers: before.layers != after.layers || before.layer_order != after.layer_order,
        ..Default::default()
    };
    let old: HashMap<_, _> = before.objects.iter().map(|o| (o.id, o)).collect();
    for o in &after.objects {
        match old.get(&o.id) {
            None => s.added.add(o),
            Some(b) if *b == o => {}
            Some(b) => classify(&mut s, b, o),
        }
    }
    if before.objects.len() + s.added.count > after.objects.len() {
        let new: HashMap<_, _> = after.objects.iter().map(|o| (o.id, ())).collect();
        for o in before.objects.iter().filter(|o| !new.contains_key(&o.id)) {
            s.removed.add(o);
        }
    }
    s.reordered = s.added.is_empty()
        && s.removed.is_empty()
        && before.objects.iter().map(|o| o.id).ne(after.objects.iter().map(|o| o.id));
    s
}

fn classify(s: &mut ChangeSummary, b: &Object, a: &Object) {
    // Vollständige Zerlegung: ein neues Feld in `Object` muss hier bedacht werden.
    let Object { id: _, name, layer, group, transform, shape, text, power_scale, primitive, tabs } = a;
    let rest = *name == b.name
        && *shape == b.shape
        && *text == b.text
        && *power_scale == b.power_scale
        && *primitive == b.primitive
        && *tabs == b.tabs;
    let (same_layer, same_group, same_transform) = (*layer == b.layer, *group == b.group, *transform == b.transform);
    if !rest {
        s.edited.add(a);
    } else if same_layer && same_group {
        let [ta, tb, tc, td, ..] = transform.0;
        let [ba, bb, bc, bd, ..] = b.transform.0;
        if [ta, tb, tc, td] == [ba, bb, bc, bd] {
            s.moved.add(a);
        } else {
            s.transformed.add(a);
        }
    } else if same_transform && same_group {
        s.relayered.add(a);
    } else if same_transform && same_layer {
        s.regrouped.add(a);
    } else {
        s.edited.add(a);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Affine, VectorPath};

    fn doc() -> (Document, u64, u64) {
        let mut d = Document::new();
        let a = d.add_path("A", 0, VectorPath::rect(0.0, 0.0, 10.0, 10.0, 0.0));
        let b = d.add_path("B", 0, VectorPath::rect(20.0, 0.0, 10.0, 10.0, 0.0));
        (d, a, b)
    }

    #[test]
    fn add_remove_and_unchanged() {
        let (d0, a, _) = doc();
        assert!(summarize(&d0, &d0).is_empty());
        let mut d1 = d0.clone();
        d1.add_path("C", 1, VectorPath::rect(0.0, 0.0, 1.0, 1.0, 0.0));
        d1.remove(&[a]);
        let s = summarize(&d0, &d1);
        assert_eq!(s.added, Affected { count: 1, name: "C".into() });
        assert_eq!(s.removed, Affected { count: 1, name: "A".into() });
        assert!(!s.reordered && !s.layers && s.moved.is_empty());
    }

    #[test]
    fn classifies_object_changes() {
        let (d0, a, b) = doc();
        let mut d1 = d0.clone();
        d1.transform_objects(&[a, b], &Affine::translate(5.0, 0.0));
        let s = summarize(&d0, &d1);
        assert_eq!(s.moved.count, 2);
        assert!(s.transformed.is_empty() && s.edited.is_empty());

        let mut d2 = d0.clone();
        d2.transform_objects(&[a], &Affine::scale(2.0, 2.0));
        assert_eq!(summarize(&d0, &d2).transformed, Affected { count: 1, name: "A".into() });

        let mut d3 = d0.clone();
        d3.get_mut(b).unwrap().layer = 4;
        assert_eq!(summarize(&d0, &d3).relayered.count, 1);

        let mut d4 = d0.clone();
        d4.group(&[a, b]);
        assert_eq!(summarize(&d0, &d4).regrouped.count, 2);

        let mut d5 = d0.clone();
        d5.get_mut(a).unwrap().power_scale = 50.0;
        d5.get_mut(b).unwrap().layer = 2;
        d5.transform_objects(&[b], &Affine::translate(1.0, 1.0));
        let s = summarize(&d0, &d5);
        assert_eq!(s.edited.count, 2, "Leistung bzw. Ebene und Lage zugleich");
        assert!(s.moved.is_empty() && s.relayered.is_empty());
    }

    #[test]
    fn order_and_layers() {
        let (d0, _, _) = doc();
        let mut d1 = d0.clone();
        d1.objects.reverse();
        let s = summarize(&d0, &d1);
        assert!(s.reordered && s.edited.is_empty());

        let mut d2 = d0.clone();
        d2.layers[3].name = "Gravur".into();
        let s = summarize(&d0, &d2);
        assert!(s.layers && !s.reordered);

        let mut d3 = d0.clone();
        d3.layer_order.swap(0, 1);
        assert!(summarize(&d0, &d3).layers);
    }
}
