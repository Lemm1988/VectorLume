//! Schlitze anpassen: Steckschlitze auf eine neue Breite bringen, z. B. für Material in anderer Stärke.
//!
//! Ein Schlitz ist eine U-Form aus drei geraden Kanten – zwei gegenläufig parallele Seiten und ein Grund mit der
//! gesuchten Breite –, deren Inneres leer ist: eine Kerbe in den Rand eines Teils oder ein Langloch. Gleich breite
//! Zapfen (das Innere ist Material) bleiben unverändert. Beide Seiten rücken symmetrisch nach außen bzw. innen, der
//! Schlitz bleibt mittig und gleich tief.

use crate::geom::{Affine, Point};
use crate::nodes::{NodeRef, move_node};
use crate::ops::nesting_depths;
use crate::path::{Segment, SubPath, VectorPath};

/// Höchste Abweichung vom rechten Winkel bzw. von der Parallelität (Kosinus).
const ANGLE_TOL: f64 = 0.02;

/// Ecken eines Teilpfades und ob die Kante von Ecke `i` zu `i + 1` gerade ist.
struct Ring {
    pts: Vec<Point>,
    straight: Vec<bool>,
}

fn ring(sp: &SubPath) -> Ring {
    let mut pts = vec![sp.start];
    let mut straight = Vec::new();
    for seg in &sp.segments {
        pts.push(seg.end());
        straight.push(matches!(seg, Segment::Line(_)));
    }
    if pts.len() > 1 && pts[pts.len() - 1].distance(sp.start) < 1e-9 {
        // ausdrücklich geschlossen: der letzte Knoten ist der Start
        pts.pop();
    } else {
        // gedachte Schlusslinie
        straight.push(true);
    }
    Ring { pts, straight }
}

/// Passt alle Schlitze der Breite `from` (± `tol`) auf `to` an. Gemessen wird im Dokumentsystem (`t` ist die
/// Objekttransformation); der Pfad bleibt im Objektsystem. Gibt die Anzahl der angepassten Schlitze zurück.
pub fn resize_slots(path: &mut VectorPath, t: &Affine, from: f64, to: f64, tol: f64) -> usize {
    let Some(inv) = t.inverse() else { return 0 };
    // Verschachtelungstiefe je geschlossenem Teilpfad: gerade = Außenkontur (Material innen), ungerade = Loch
    let index: Vec<usize> = (0..path.subpaths.len())
        .filter(|&i| path.subpaths[i].closed && !path.subpaths[i].segments.is_empty())
        .collect();
    let polys: Vec<_> =
        index.iter().flat_map(|&i| VectorPath { subpaths: vec![path.subpaths[i].clone()] }.flatten(t, 0.05)).collect();
    if polys.len() != index.len() {
        return 0;
    }
    let depths = nesting_depths(&polys);
    let mut count = 0;
    for (k, &si) in index.iter().enumerate() {
        let r = ring(&path.subpaths[si]);
        let n = r.pts.len();
        if n < 4 || polys[k].points.len() < 3 {
            continue;
        }
        let world: Vec<Point> = r.pts.iter().map(|p| t.apply(*p)).collect();
        // Normale zur Materialseite: links der Fahrtrichtung bei positiver Fläche, bei Löchern umgekehrt
        let mut side = polys[k].signed_area().signum();
        if depths[k] % 2 == 1 {
            side = -side;
        }
        let dir = |i: usize| {
            let (a, b) = (world[i % n], world[(i + 1) % n]);
            (b - a) * (1.0 / a.distance(b).max(1e-12))
        };
        let len = |i: usize| world[i % n].distance(world[(i + 1) % n]);
        let mut moved = vec![false; n];
        let mut moves: Vec<(usize, Point)> = Vec::new();
        for j in 0..n {
            // Kanten j-1 (Seite), j (Grund), j+1 (Seite) und die Randkanten davor und danach
            let (side1, bottom, side2) = ((j + n - 1) % n, j, (j + 1) % n);
            let (before, after) = ((j + n - 2) % n, (j + 2) % n);
            if [before, side1, bottom, side2, after].iter().any(|&e| !r.straight[e]) {
                continue;
            }
            if (len(bottom) - from).abs() > tol || len(side1) <= tol || len(side2) <= tol {
                continue;
            }
            let (s1, u, s2) = (dir(side1), dir(bottom), dir(side2));
            let perpendicular = s1.dot(u).abs() < ANGLE_TOL && s2.dot(u).abs() < ANGLE_TOL;
            let opposite = s1.dot(s2) < -1.0 + ANGLE_TOL;
            // Randkanten an der Öffnung laufen parallel zum Grund
            let rim = dir(before).cross(u).abs() < ANGLE_TOL && dir(after).cross(u).abs() < ANGLE_TOL;
            let material = Point::new(-u.y, u.x) * side;
            if !(perpendicular && opposite && rim) || material.dot(s1) <= 0.0 {
                continue;
            }
            let corners = [side1, bottom, side2, after].map(|e| e % n);
            if corners.iter().any(|&c| moved[c]) {
                continue;
            }
            let shift = u * ((to - from) / 2.0);
            for (c, d) in [(side1, -shift), (bottom, -shift), (side2, shift), (after, shift)] {
                moved[c] = true;
                moves.push((c, world[c] + d));
            }
            count += 1;
        }
        for (c, p) in moves {
            move_node(path, NodeRef { sub: si, idx: c }, inv.apply(p));
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::Polyline;

    fn poly(pts: &[(f64, f64)]) -> VectorPath {
        VectorPath::polygon(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), true)
    }

    fn corners(p: &VectorPath) -> Vec<Vec<Point>> {
        p.flatten(&Affine::IDENTITY, 0.01).into_iter().map(|pl: Polyline| pl.points).collect()
    }

    #[test]
    fn widens_a_notch_and_a_slot_hole() {
        // Platte 50 × 30 mit Kerbe (3 mm breit, 10 tief) von unten und Langloch 3 × 20
        let plate =
            [(0.0, 0.0), (20.0, 0.0), (20.0, 10.0), (23.0, 10.0), (23.0, 0.0), (50.0, 0.0), (50.0, 30.0), (0.0, 30.0)];
        let mut p = poly(&plate);
        p.append(poly(&[(35.0, 5.0), (38.0, 5.0), (38.0, 25.0), (35.0, 25.0)]));
        let t = Affine::IDENTITY;
        assert_eq!(resize_slots(&mut p, &t, 3.0, 4.0, 0.1), 2);
        let c = corners(&p);
        let near = |a: Point, x: f64, y: f64| a.distance(Point::new(x, y)) < 1e-9;
        assert!(near(c[0][1], 19.5, 0.0) && near(c[0][2], 19.5, 10.0));
        assert!(near(c[0][3], 23.5, 10.0) && near(c[0][4], 23.5, 0.0));
        assert!(near(c[0][5], 50.0, 0.0), "Außenmaß bleibt");
        let hole = &c[1];
        let xs: Vec<f64> = hole.iter().map(|p| p.x).collect();
        assert!(xs.iter().all(|&x| (x - 34.5).abs() < 1e-9 || (x - 38.5).abs() < 1e-9), "{xs:?}");
        assert!(hole.iter().all(|p| (p.y - 5.0).abs() < 1e-9 || (p.y - 25.0).abs() < 1e-9), "Länge bleibt");
    }

    #[test]
    fn leaves_tabs_alone_and_works_in_both_directions() {
        // Zapfen 3 mm breit nach unten: Material innen, kein Schlitz
        let tab =
            [(0.0, 0.0), (20.0, 0.0), (20.0, -5.0), (23.0, -5.0), (23.0, 0.0), (50.0, 0.0), (50.0, 30.0), (0.0, 30.0)];
        let mut p = poly(&tab);
        assert_eq!(resize_slots(&mut p, &Affine::IDENTITY, 3.0, 4.0, 0.1), 0);
        // Kerbe in umgekehrter Laufrichtung, Objekt verschoben und gedreht
        let mut notch: Vec<(f64, f64)> = vec![
            (0.0, 0.0),
            (20.0, 0.0),
            (20.0, 10.0),
            (23.0, 10.0),
            (23.0, 0.0),
            (50.0, 0.0),
            (50.0, 30.0),
            (0.0, 30.0),
        ];
        notch.reverse();
        let mut p = poly(&notch);
        let t = Affine::rotate(0.5).then(&Affine::translate(100.0, 40.0));
        assert_eq!(resize_slots(&mut p, &t, 3.0, 2.5, 0.1), 1);
        let pts = &p.flatten(&t, 0.01)[0].points;
        let w = pts.windows(2).map(|w| w[0].distance(w[1])).filter(|&d| (d - 2.5).abs() < 1e-6).count();
        assert_eq!(w, 1, "Grund jetzt 2,5 mm");
        // falsche Breite: nichts
        let mut p = poly(&notch);
        assert_eq!(resize_slots(&mut p, &Affine::IDENTITY, 5.0, 4.0, 0.1), 0);
    }
}
