//! Grundlegende 2D-Geometrie: Punkte, Rechtecke und affine Transformationen.
//!
//! Alle Maße sind Millimeter. Das Dokument benutzt die Bildschirm-Orientierung:
//! X wächst nach rechts, Y wächst nach unten, (0,0) ist die hintere linke Ecke des Arbeitsbereichs.

use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const ZERO: Point = Point { x: 0.0, y: 0.0 };

    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn length(self) -> f64 {
        self.x.hypot(self.y)
    }

    pub fn distance(self, other: Point) -> f64 {
        (self - other).length()
    }

    pub fn distance_sq(self, other: Point) -> f64 {
        let d = self - other;
        d.x * d.x + d.y * d.y
    }

    pub fn lerp(self, other: Point, t: f64) -> Point {
        self + (other - self) * t
    }

    pub fn dot(self, other: Point) -> f64 {
        self.x * other.x + self.y * other.y
    }

    pub fn cross(self, other: Point) -> f64 {
        self.x * other.y - self.y * other.x
    }

    pub fn rotated(self, angle_rad: f64) -> Point {
        let (s, c) = angle_rad.sin_cos();
        Point::new(self.x * c - self.y * s, self.x * s + self.y * c)
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Add for Point {
    type Output = Point;
    fn add(self, o: Point) -> Point {
        Point::new(self.x + o.x, self.y + o.y)
    }
}

impl AddAssign for Point {
    fn add_assign(&mut self, o: Point) {
        self.x += o.x;
        self.y += o.y;
    }
}

impl Sub for Point {
    type Output = Point;
    fn sub(self, o: Point) -> Point {
        Point::new(self.x - o.x, self.y - o.y)
    }
}

impl SubAssign for Point {
    fn sub_assign(&mut self, o: Point) {
        self.x -= o.x;
        self.y -= o.y;
    }
}

impl Mul<f64> for Point {
    type Output = Point;
    fn mul(self, s: f64) -> Point {
        Point::new(self.x * s, self.y * s)
    }
}

impl Neg for Point {
    type Output = Point;
    fn neg(self) -> Point {
        Point::new(-self.x, -self.y)
    }
}

/// Achsenparalleles Rechteck. Ein leeres Rechteck hat `min > max`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub min: Point,
    pub max: Point,
}

impl Default for Rect {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl Rect {
    pub const EMPTY: Rect = Rect {
        min: Point { x: f64::INFINITY, y: f64::INFINITY },
        max: Point { x: f64::NEG_INFINITY, y: f64::NEG_INFINITY },
    };

    pub fn new(min: Point, max: Point) -> Self {
        Self { min, max }
    }

    pub fn from_xywh(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self::new(Point::new(x, y), Point::new(x + w, y + h))
    }

    pub fn from_points(a: Point, b: Point) -> Self {
        Self::new(Point::new(a.x.min(b.x), a.y.min(b.y)), Point::new(a.x.max(b.x), a.y.max(b.y)))
    }

    pub fn is_empty(&self) -> bool {
        !(self.min.x <= self.max.x && self.min.y <= self.max.y)
    }

    pub fn width(&self) -> f64 {
        if self.is_empty() { 0.0 } else { self.max.x - self.min.x }
    }

    pub fn height(&self) -> f64 {
        if self.is_empty() { 0.0 } else { self.max.y - self.min.y }
    }

    pub fn center(&self) -> Point {
        self.min.lerp(self.max, 0.5)
    }

    pub fn include(&mut self, p: Point) {
        self.min.x = self.min.x.min(p.x);
        self.min.y = self.min.y.min(p.y);
        self.max.x = self.max.x.max(p.x);
        self.max.y = self.max.y.max(p.y);
    }

    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        Rect::new(
            Point::new(self.min.x.min(other.min.x), self.min.y.min(other.min.y)),
            Point::new(self.max.x.max(other.max.x), self.max.y.max(other.max.y)),
        )
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }

    pub fn contains_rect(&self, r: &Rect) -> bool {
        !r.is_empty() && self.contains(r.min) && self.contains(r.max)
    }

    pub fn intersects(&self, r: &Rect) -> bool {
        !(r.min.x > self.max.x || r.max.x < self.min.x || r.min.y > self.max.y || r.max.y < self.min.y)
    }

    pub fn expanded(&self, d: f64) -> Rect {
        Rect::new(self.min - Point::new(d, d), self.max + Point::new(d, d))
    }

    pub fn corners(&self) -> [Point; 4] {
        [self.min, Point::new(self.max.x, self.min.y), self.max, Point::new(self.min.x, self.max.y)]
    }
}

/// Affine 2D-Transformation `[a, b, c, d, e, f]`:
/// `x' = a*x + c*y + e`, `y' = b*x + d*y + f`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Affine(pub [f64; 6]);

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine {
    pub const IDENTITY: Affine = Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    pub fn translate(dx: f64, dy: f64) -> Self {
        Affine([1.0, 0.0, 0.0, 1.0, dx, dy])
    }

    pub fn scale(sx: f64, sy: f64) -> Self {
        Affine([sx, 0.0, 0.0, sy, 0.0, 0.0])
    }

    pub fn rotate(angle_rad: f64) -> Self {
        let (s, c) = angle_rad.sin_cos();
        Affine([c, s, -s, c, 0.0, 0.0])
    }

    pub fn scale_about(sx: f64, sy: f64, center: Point) -> Self {
        Affine::translate(-center.x, -center.y)
            .then(&Affine::scale(sx, sy))
            .then(&Affine::translate(center.x, center.y))
    }

    pub fn rotate_about(angle_rad: f64, center: Point) -> Self {
        Affine::translate(-center.x, -center.y)
            .then(&Affine::rotate(angle_rad))
            .then(&Affine::translate(center.x, center.y))
    }

    /// Erst `self`, danach `next` anwenden.
    pub fn then(&self, next: &Affine) -> Affine {
        let [a1, b1, c1, d1, e1, f1] = self.0;
        let [a2, b2, c2, d2, e2, f2] = next.0;
        Affine([
            a2 * a1 + c2 * b1,
            b2 * a1 + d2 * b1,
            a2 * c1 + c2 * d1,
            b2 * c1 + d2 * d1,
            a2 * e1 + c2 * f1 + e2,
            b2 * e1 + d2 * f1 + f2,
        ])
    }

    pub fn apply(&self, p: Point) -> Point {
        let [a, b, c, d, e, f] = self.0;
        Point::new(a * p.x + c * p.y + e, b * p.x + d * p.y + f)
    }

    pub fn apply_vector(&self, v: Point) -> Point {
        let [a, b, c, d, _, _] = self.0;
        Point::new(a * v.x + c * v.y, b * v.x + d * v.y)
    }

    pub fn determinant(&self) -> f64 {
        let [a, b, c, d, _, _] = self.0;
        a * d - b * c
    }

    pub fn inverse(&self) -> Option<Affine> {
        let det = self.determinant();
        if det.abs() < 1e-12 || !det.is_finite() {
            return None;
        }
        let [a, b, c, d, e, f] = self.0;
        Some(Affine([d / det, -b / det, -c / det, a / det, (c * f - d * e) / det, (b * e - a * f) / det]))
    }

    /// Mittlerer Skalierungsfaktor (für Toleranzen beim Abflachen von Kurven).
    pub fn mean_scale(&self) -> f64 {
        self.determinant().abs().sqrt().max(1e-9)
    }

    /// Drehwinkel der X-Achse in Grad (nur zur Anzeige).
    pub fn rotation_deg(&self) -> f64 {
        let [a, b, ..] = self.0;
        b.atan2(a).to_degrees()
    }

    pub fn transform_rect(&self, r: &Rect) -> Rect {
        let mut out = Rect::EMPTY;
        if r.is_empty() {
            return out;
        }
        for c in r.corners() {
            out.include(self.apply(c));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Point, b: Point) -> bool {
        a.distance(b) < 1e-9
    }

    #[test]
    fn affine_then_and_inverse() {
        let t = Affine::scale(2.0, 3.0).then(&Affine::rotate(0.7)).then(&Affine::translate(5.0, -2.0));
        let p = Point::new(1.5, -4.0);
        let q = t.apply(p);
        let manual = Point::new(1.5 * 2.0, -4.0 * 3.0).rotated(0.7) + Point::new(5.0, -2.0);
        assert!(close(q, manual));
        let back = t.inverse().unwrap().apply(q);
        assert!(close(back, p));
    }

    #[test]
    fn scale_about_keeps_center() {
        let c = Point::new(10.0, 20.0);
        assert!(close(Affine::scale_about(3.0, 0.5, c).apply(c), c));
        assert!(close(Affine::rotate_about(1.0, c).apply(c), c));
    }

    #[test]
    fn rect_union_and_empty() {
        let a = Rect::from_xywh(0.0, 0.0, 10.0, 5.0);
        assert!(Rect::EMPTY.is_empty());
        assert_eq!(Rect::EMPTY.union(&a), a);
        let b = Rect::from_xywh(-5.0, 2.0, 1.0, 10.0);
        let u = a.union(&b);
        assert_eq!(u.min, Point::new(-5.0, 0.0));
        assert_eq!(u.max, Point::new(10.0, 12.0));
    }
}
