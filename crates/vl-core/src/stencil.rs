//! Schablonenstege: Inseln in Löchern (z. B. das Innere von O, A, B) über schmale Stege mit dem Rest verbinden,
//! damit sie beim Ausschneiden einer Schablone nicht herausfallen.

use crate::geom::{Affine, Point};
use crate::ops::{self, BoolOp};
use crate::path::{Polyline, point_in_polylines};

/// Fläche (Gerade-Ungerade-Regel) mit eingeschnittenen Stegen der Breite `width` und die Anzahl der Stege. Je Loch
/// zwei Stege in Richtung `angle_deg` (0 = senkrecht) durch die Lochmitte, jeweils über das angrenzende
/// Flächenstück zur einen und zur anderen Seite.
pub fn stencil_bridges(region: &[Polyline], width: f64, angle_deg: f64) -> (Vec<Polyline>, usize) {
    let clean = ops::boolean(region, &[], BoolOp::Union);
    if width <= 0.0 || clean.is_empty() {
        return (clean, 0);
    }
    // Im gedrehten System verlaufen die Stege senkrecht
    let rot = Affine::rotate(-angle_deg.to_radians());
    let back = Affine::rotate(angle_deg.to_radians());
    let turned: Vec<Polyline> =
        clean.iter().map(|p| Polyline::new(p.points.iter().map(|q| rot.apply(*q)).collect(), true)).collect();
    let ext = width;
    let mut bars: Vec<Polyline> = Vec::new();
    for (i, hole) in turned.iter().enumerate() {
        // Loch = von einer ungeraden Anzahl anderer Konturen umschlossen
        let probe = hole.points[0];
        let depth = turned
            .iter()
            .enumerate()
            .filter(|(j, p)| *j != i && point_in_polylines(probe, std::slice::from_ref(*p)))
            .count();
        if depth % 2 == 0 {
            continue;
        }
        let b = hole.bounds();
        let x = (b.min.x + b.max.x) / 2.0;
        let own = crossings(std::slice::from_ref(hole), x);
        let (Some(&top), Some(&bottom)) = (own.first(), own.last()) else { continue };
        let all = crossings(&turned, x);
        let eps = 1e-9;
        let above = all.iter().rev().find(|&&y| y < top - eps).copied();
        let below = all.iter().find(|&&y| y > bottom + eps).copied();
        for (y0, y1) in [above.map(|a| (a, top)), below.map(|c| (bottom, c))].into_iter().flatten() {
            let r = [
                Point::new(x - width / 2.0, y0 - ext),
                Point::new(x + width / 2.0, y0 - ext),
                Point::new(x + width / 2.0, y1 + ext),
                Point::new(x - width / 2.0, y1 + ext),
            ];
            bars.push(Polyline::new(r.iter().map(|p| back.apply(*p)).collect(), true));
        }
    }
    if bars.is_empty() {
        return (clean, 0);
    }
    let n = bars.len();
    // Stege vorher vereinigen: deckungsgleiche (z. B. durch den Mittelstrich eines B) würden sich sonst nach der
    // Gerade-Ungerade-Regel gegenseitig aufheben
    (ops::boolean(&clean, &ops::union_nonzero(&bars), BoolOp::Difference), n)
}

/// Schnittpunkte (y) der senkrechten Geraden bei `x` mit den Konturen, aufsteigend.
fn crossings(polys: &[Polyline], x: f64) -> Vec<f64> {
    let mut ys: Vec<f64> = polys
        .iter()
        .flat_map(|p| p.edges())
        .filter(|(a, b)| (a.x <= x) != (b.x <= x))
        .map(|(a, b)| a.y + (x - a.x) * (b.y - a.y) / (b.x - a.x))
        .collect();
    ys.sort_by(f64::total_cmp);
    ys
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::VectorPath;

    fn flat(paths: &[VectorPath]) -> Vec<Polyline> {
        paths.iter().flat_map(|p| p.flatten(&Affine::IDENTITY, 0.01)).collect()
    }

    fn area(polys: &[Polyline]) -> f64 {
        // Gerade-Ungerade: Löcher sind entgegengesetzt orientiert
        polys.iter().map(|p| p.signed_area()).sum::<f64>().abs()
    }

    #[test]
    fn ring_is_split_and_counter_stays_attached() {
        let o = Point::new(0.0, 0.0);
        let ring = flat(&[VectorPath::ellipse(o, 10.0, 10.0), VectorPath::ellipse(o, 6.0, 6.0)]);
        let (out, n) = stencil_bridges(&ring, 1.0, 0.0);
        assert_eq!(n, 2);
        assert_eq!(out.len(), 2, "zwei C-Hälften");
        let full = std::f64::consts::PI * (100.0 - 36.0);
        assert!((full - area(&out) - 8.0).abs() < 0.3, "{}", area(&out));
        let inside = |p: Point| point_in_polylines(p, &out);
        assert!(!inside(Point::new(0.0, 0.0)), "Innenteil gehört nicht zur Fläche");
        assert!(!inside(Point::new(0.0, 8.0)) && !inside(Point::new(0.0, -8.0)), "Steg oben und unten");
        assert!(inside(Point::new(8.0, 0.0)) && inside(Point::new(-8.0, 0.0)));
        // waagrecht
        let (side, _) = stencil_bridges(&ring, 1.0, 90.0);
        let inside = |p: Point| point_in_polylines(p, &side);
        assert!(!inside(Point::new(8.0, 0.0)) && inside(Point::new(0.0, 8.0)));
    }

    #[test]
    fn stacked_holes_and_plain_shapes() {
        // „B“: Rechteck mit zwei Löchern übereinander – vier Stege, Ergebnis linke und rechte Hälfte
        let b = flat(&[
            VectorPath::rect(0.0, 0.0, 20.0, 30.0, 0.0),
            VectorPath::rect(4.0, 4.0, 12.0, 9.0, 0.0),
            VectorPath::rect(4.0, 17.0, 12.0, 9.0, 0.0),
        ]);
        let (out, n) = stencil_bridges(&b, 2.0, 0.0);
        assert_eq!(n, 4);
        assert_eq!(out.len(), 2);
        assert!((area(&out) - (600.0 - 2.0 * 108.0 - 2.0 * 12.0)).abs() < 1e-6, "{}", area(&out));
        // ohne Löcher bleibt alles, wie es ist
        let sq = flat(&[VectorPath::rect(0.0, 0.0, 10.0, 10.0, 0.0)]);
        let (same, n) = stencil_bridges(&sq, 1.0, 0.0);
        assert_eq!(n, 0);
        assert!((area(&same) - 100.0).abs() < 1e-9);
    }
}
