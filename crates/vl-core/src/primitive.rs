//! Grundformen mit Parametern: Rechteck (Eckenradius), Vieleck (Ecken) und Stern (Zacken, Innenradius) lassen sich
//! nach dem Zeichnen noch ändern. Die Parameter gelten nur, solange der Pfad unverändert aus ihnen hervorgeht –
//! jede andere Bearbeitung (Knoten, Boolesche Operationen …) macht die Form zu einem gewöhnlichen Pfad.

use crate::document::{Object, Shape};
use crate::geom::{Affine, Point};
use crate::path::VectorPath;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Primitive {
    /// Rechteck mit linker oberer Ecke (`x`, `y`), alles lokal in mm.
    Rect { x: f64, y: f64, width: f64, height: f64, radius: f64 },
    /// Regelmäßiges Vieleck, erste Ecke oben.
    Polygon { center: Point, radius: f64, sides: usize },
    /// Stern; `inner` = Innenradius als Anteil des Außenradius.
    Star { center: Point, radius: f64, inner: f64, points: usize },
}

impl Primitive {
    pub fn path(&self) -> VectorPath {
        match *self {
            Primitive::Rect { x, y, width, height, radius } => VectorPath::rect(x, y, width, height, radius),
            Primitive::Polygon { center, radius, sides } => VectorPath::regular_polygon(center, radius, sides),
            Primitive::Star { center, radius, inner, points } => {
                VectorPath::star(center, radius, radius * inner, points)
            }
        }
    }
}

impl Object {
    /// Parameter der Grundform, sofern der Pfad noch genau aus ihnen besteht.
    pub fn live_primitive(&self) -> Option<Primitive> {
        let p = self.primitive?;
        matches!(&self.shape, Shape::Path(path) if *path == p.path()).then_some(p)
    }

    /// Ersetzt die Grundform (Transformation bleibt).
    pub fn set_primitive(&mut self, p: Primitive) {
        self.shape = Shape::Path(p.path());
        self.primitive = Some(p);
    }

    /// Eckenradius eines Rechtecks in Dokument-mm.
    pub fn corner_radius(&self) -> Option<f64> {
        match self.live_primitive()? {
            Primitive::Rect { radius, .. } => Some(radius * self.transform.determinant().abs().sqrt()),
            _ => None,
        }
    }

    /// Setzt den Eckenradius in Dokument-mm. Skalierung und Drehung wandern dabei in die Rechteckmaße bzw. eine
    /// reine Drehung, damit die Ecken auch bei verzerrt skalierten Rechtecken kreisrund werden; nur bei Scherung
    /// wird der Radius lokal umgerechnet.
    pub fn set_corner_radius(&mut self, r: f64) -> bool {
        let Some(Primitive::Rect { x, y, width, height, .. }) = self.live_primitive() else { return false };
        let [a, b, c, d, _, _] = self.transform.0;
        let (u, v) = (Point::new(a, b), Point::new(c, d));
        let (sx, sy) = (u.length(), v.length());
        if sx < 1e-12 || sy < 1e-12 {
            return false;
        }
        let r = r.max(0.0);
        let identity = (a - 1.0).abs() < 1e-12 && b.abs() < 1e-12 && c.abs() < 1e-12 && (d - 1.0).abs() < 1e-12;
        let rect = if identity {
            Primitive::Rect { x, y, width, height, radius: r }
        } else if u.dot(v).abs() > 1e-9 * sx * sy {
            Primitive::Rect { x, y, width, height, radius: r / self.transform.determinant().abs().sqrt() }
        } else {
            let center = self.transform.apply(Point::new(x + width / 2.0, y + height / 2.0));
            let mirror = if self.transform.determinant() < 0.0 { -1.0 } else { 1.0 };
            self.transform = Affine::scale(1.0, mirror)
                .then(&Affine::rotate(b.atan2(a)))
                .then(&Affine::translate(center.x, center.y));
            let (w, h) = (width * sx, height * sy);
            Primitive::Rect { x: -w / 2.0, y: -h / 2.0, width: w, height: h, radius: r }
        };
        self.set_primitive(rect);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    #[test]
    fn corner_radius_survives_scaling_and_rotation() {
        let mut doc = Document::new();
        let rect = Primitive::Rect { x: 10.0, y: 10.0, width: 40.0, height: 20.0, radius: 0.0 };
        let id = doc.add_primitive("Rechteck", 0, rect);
        let o = doc.get_mut(id).unwrap();
        assert_eq!(o.live_primitive(), Some(rect));
        // verzerrt skalieren und drehen
        o.transform = Affine::scale_about(2.0, 0.5, Point::new(10.0, 10.0))
            .then(&Affine::rotate_about(30f64.to_radians(), Point::new(30.0, 20.0)));
        let before = o.bounds();
        assert!(o.set_corner_radius(3.0));
        assert!((o.corner_radius().unwrap() - 3.0).abs() < 1e-9);
        let Some(Primitive::Rect { width, height, .. }) = o.live_primitive() else { panic!() };
        assert!((width - 80.0).abs() < 1e-9 && (height - 10.0).abs() < 1e-9, "Maße übernommen");
        // Ecken ändern den Umriss nur an den Ecken: Mittelpunkt und Drehung bleiben
        assert!(o.bounds().center().distance(before.center()) < 1e-9);
        assert!((o.transform.determinant() - 1.0).abs() < 1e-9);

        // Gespiegelt bleibt gespiegelt
        o.transform = o.transform.then(&Affine::scale(-1.0, 1.0));
        assert!(o.set_corner_radius(2.0));
        assert!(o.transform.determinant() < 0.0 && (o.corner_radius().unwrap() - 2.0).abs() < 1e-9);

        // Jede andere Pfadänderung beendet die Parameter
        o.shape = Shape::Path(VectorPath::rect(0.0, 0.0, 5.0, 5.0, 0.0));
        assert_eq!(o.live_primitive(), None);
        assert!(!o.set_corner_radius(1.0));
    }

    #[test]
    fn star_and_polygon_regenerate() {
        let mut doc = Document::new();
        let id =
            doc.add_primitive("Stern", 0, Primitive::Star { center: Point::ZERO, radius: 10.0, inner: 0.4, points: 5 });
        let o = doc.get_mut(id).unwrap();
        o.set_primitive(Primitive::Star { center: Point::ZERO, radius: 10.0, inner: 0.6, points: 7 });
        assert_eq!(o.live_primitive().map(|p| p.path()), Some(VectorPath::star(Point::ZERO, 10.0, 10.0 * 0.6, 7)));
        let id = doc.add_primitive("Vieleck", 0, Primitive::Polygon { center: Point::ZERO, radius: 10.0, sides: 6 });
        assert!(doc.get(id).unwrap().live_primitive().is_some());
    }
}
