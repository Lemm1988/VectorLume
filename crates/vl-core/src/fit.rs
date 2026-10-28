//! Kurvenanpassung: dichte Punktfolgen (Konturen aus dem Nachzeichnen, Skelettlinien) → wenige glatte
//! Bézierkurven. Ablauf: Ecken suchen, die Abschnitte dazwischen entrauschen (Taubin-Glättung – sie schrumpft
//! Formen nicht wie ein einfacher Mittelwert), dann kubische Bézierkurven mit Fehlerschranke einpassen
//! (Schneider, „An Algorithm for Automatically Fitting Digitized Curves“, Graphics Gems 1990). An den Nahtstellen
//! zwischen zwei Kurven eines Abschnitts stimmen die Tangenten überein, nur an Ecken knickt der Pfad.

use crate::geom::Point;
use crate::path::{PathBuilder, VectorPath};

/// Einstellungen der Kurvenanpassung.
#[derive(Clone, Copy, Debug)]
pub struct FitOptions {
    /// Größter Abstand der Kurven von den (entrauschten) Punkten, in den Einheiten der Punkte.
    pub tolerance: f64,
    /// 0 = Vieleck aus geraden Linien, darüber Kurven; höher = stärker entrauscht (0…1).
    pub smoothing: f64,
    /// Richtungswechsel über diesem Winkel (Grad) bleiben Ecken.
    pub corner_deg: f64,
    /// Bereich (Einheiten der Punkte), über den Richtungen und Ecken gemessen werden – etwa die Rauschbreite.
    pub reach: f64,
}

/// Punktfolge → Pfad aus Linien und kubischen Bézierkurven.
pub fn fit_points(input: &[Point], closed: bool, o: &FitOptions) -> VectorPath {
    let mut pts: Vec<Point> = Vec::with_capacity(input.len());
    for &p in input.iter().filter(|p| p.x.is_finite() && p.y.is_finite()) {
        if pts.last().is_none_or(|q: &Point| q.distance_sq(p) > 1e-12) {
            pts.push(p);
        }
    }
    if closed && pts.len() > 1 && pts[0].distance_sq(pts[pts.len() - 1]) <= 1e-12 {
        pts.pop();
    }
    let n = pts.len();
    let mut pb = PathBuilder::new();
    if n == 0 {
        return pb.finish();
    }
    let tol = o.tolerance.max(0.01);
    if o.smoothing <= 0.0 || n < 3 {
        let simple = crate::ops::simplify_points(&pts, closed, tol);
        pb.move_to(simple[0]);
        for &p in &simple[1..] {
            pb.line_to(p);
        }
        if closed {
            pb.close();
        }
        return pb.finish();
    }
    let reach = o.reach.max(0.5);
    // Gleichmäßig dicht (Skelettlinien liefern nur die Knickpunkte), sonst verzerren Glättung und
    // Eckensuche lange gegen kurze Abstände
    let pts = densify(&pts, closed, reach * 0.5);
    let n = pts.len();
    let corners = find_corners(&pts, closed, o.corner_deg.to_radians(), reach);
    let pts = taubin(&pts, closed, &corners, (o.smoothing.clamp(0.0, 1.0).powi(2) * 30.0).round() as usize);

    // Abschnitte zwischen Ecken (offene Linienzüge: auch zwischen den Enden). Geschlossen ohne Ecke: zwei
    // Hälften mit gemeinsamer Tangente an den Nahtstellen.
    let mut beziers: Vec<[Point; 4]> = Vec::new();
    let tangent_mid = |i: usize| central_tangent(&pts, closed, i, reach);
    if closed && corners.is_empty() {
        let half = farthest_by_arc(&pts, 0);
        let (t0, th) = (tangent_mid(0), tangent_mid(half));
        let a: Vec<Point> = pts[..=half].to_vec();
        let b: Vec<Point> = pts[half..].iter().copied().chain(std::iter::once(pts[0])).collect();
        fit_cubic(&a, t0, -th, tol, &mut beziers, 0);
        fit_cubic(&b, th, -t0, tol, &mut beziers, 0);
    } else {
        let mut cuts = corners.clone();
        if !closed {
            if cuts.first() != Some(&0) {
                cuts.insert(0, 0);
            }
            if cuts.last() != Some(&(n - 1)) {
                cuts.push(n - 1);
            }
        }
        let sections = if closed { cuts.len() } else { cuts.len() - 1 };
        for k in 0..sections {
            let (a, b) = (cuts[k], cuts[(k + 1) % cuts.len()]);
            let sec: Vec<Point> =
                if b > a { pts[a..=b].to_vec() } else { pts[a..].iter().chain(pts[..=b].iter()).copied().collect() };
            if sec.len() < 2 {
                continue;
            }
            let t1 = end_tangent(&sec, false, reach);
            let t2 = end_tangent(&sec, true, reach);
            fit_cubic(&sec, t1, t2, tol, &mut beziers, 0);
        }
    }
    let Some(first) = beziers.first() else {
        return pb.finish();
    };
    pb.move_to(first[0]);
    for b in &beziers {
        // Fast gerade Kurven werden Linien (weniger Knoten, sauberere Kanten)
        let flat = 0.02 * tol.max(1.0);
        if crate::path::distance_to_segment(b[1], b[0], b[3]) < flat
            && crate::path::distance_to_segment(b[2], b[0], b[3]) < flat
        {
            pb.line_to(b[3]);
        } else {
            pb.cubic_to(b[1], b[2], b[3]);
        }
    }
    if closed {
        pb.close();
    }
    pb.finish()
}

/// Fügt auf langen Abschnitten Zwischenpunkte ein (Abstand höchstens `step`); alle Punkte bleiben erhalten.
fn densify(pts: &[Point], closed: bool, step: f64) -> Vec<Point> {
    let n = pts.len();
    let mut out = Vec::with_capacity(n);
    let segments = if closed { n } else { n - 1 };
    for i in 0..segments {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        out.push(a);
        let k = (a.distance(b) / step).ceil() as usize;
        for j in 1..k {
            out.push(a.lerp(b, j as f64 / k as f64));
        }
    }
    if !closed {
        out.push(pts[n - 1]);
    }
    out
}

/// Index des Punkts, der entlang der Kontur am weitesten (Bogenlänge) von `start` entfernt ist.
fn farthest_by_arc(pts: &[Point], start: usize) -> usize {
    let n = pts.len();
    let total: f64 = (0..n).map(|i| pts[i].distance(pts[(i + 1) % n])).sum();
    let mut s = 0.0;
    for k in 1..n {
        let i = (start + k) % n;
        s += pts[(i + n - 1) % n].distance(pts[i]);
        if s >= total / 2.0 {
            return i;
        }
    }
    (start + n / 2) % n
}

/// Punkt, der `dist` Bogenlänge von `i` entfernt liegt (vorwärts oder rückwärts), bei offenen Linienzügen
/// höchstens bis zum Ende. Liefert auch die tatsächlich erreichte Strecke.
fn walk(pts: &[Point], closed: bool, i: usize, dist: f64, forward: bool) -> (usize, f64) {
    let n = pts.len();
    let (mut j, mut s) = (i, 0.0);
    for _ in 0..n - 1 {
        let next = match (forward, closed) {
            (true, _) if j + 1 < n => j + 1,
            (true, true) => 0,
            (false, _) if j > 0 => j - 1,
            (false, true) => n - 1,
            _ => break,
        };
        s += pts[j].distance(pts[next]);
        j = next;
        if s >= dist {
            break;
        }
    }
    (j, s)
}

/// Richtungswechsel je Punkt, gemessen über Sehnen der Länge `reach` davor und danach (0 = gerade). Ecken sind
/// die örtlichen Maxima über `limit`; an offenen Enden gibt es keine Messung.
fn find_corners(pts: &[Point], closed: bool, limit: f64, reach: f64) -> Vec<usize> {
    let n = pts.len();
    if n < 3 || limit >= std::f64::consts::PI {
        return Vec::new();
    }
    let turn: Vec<f64> = (0..n)
        .map(|i| {
            let (a, da) = walk(pts, closed, i, reach, false);
            let (b, db) = walk(pts, closed, i, reach, true);
            if !closed && (da < reach * 0.5 || db < reach * 0.5) {
                return 0.0;
            }
            let (u, v) = (pts[i] - pts[a], pts[b] - pts[i]);
            if u.length() < 1e-9 || v.length() < 1e-9 {
                return 0.0;
            }
            u.cross(v).atan2(u.dot(v)).abs()
        })
        .collect();
    let mut corners = Vec::new();
    for i in 0..n {
        if turn[i] <= limit {
            continue;
        }
        // örtliches Maximum innerhalb der Messweite (bei Gleichstand gewinnt der erste)
        let mut best = true;
        for forward in [false, true] {
            let mut j = i;
            let mut s = 0.0;
            for _ in 0..n - 1 {
                let next = match (forward, closed) {
                    (true, _) if j + 1 < n => j + 1,
                    (true, true) => 0,
                    (false, _) if j > 0 => j - 1,
                    (false, true) => n - 1,
                    _ => break,
                };
                s += pts[j].distance(pts[next]);
                j = next;
                if s > reach || j == i {
                    break;
                }
                if turn[j] > turn[i] || (turn[j] == turn[i] && !forward) {
                    best = false;
                }
            }
        }
        if best {
            corners.push(i);
        }
    }
    corners
}

/// Taubin-Glättung (λ/μ): entfernt kleines Zittern, ohne die Form schrumpfen zu lassen. Ecken und die Enden
/// offener Linienzüge bleiben fest.
fn taubin(pts: &[Point], closed: bool, fixed: &[usize], iterations: usize) -> Vec<Point> {
    let n = pts.len();
    let mut p = pts.to_vec();
    if iterations == 0 || n < 3 {
        return p;
    }
    let mut pinned = vec![false; n];
    for &i in fixed {
        pinned[i] = true;
    }
    if !closed {
        pinned[0] = true;
        pinned[n - 1] = true;
    }
    let mut q = p.clone();
    for _ in 0..iterations {
        for f in [0.5, -0.53] {
            for i in 0..n {
                if pinned[i] {
                    q[i] = p[i];
                    continue;
                }
                let (a, b) = (p[(i + n - 1) % n], p[(i + 1) % n]);
                q[i] = p[i] + ((a + b) * 0.5 - p[i]) * f;
            }
            std::mem::swap(&mut p, &mut q);
        }
    }
    p
}

/// Tangente (Einheitsvektor, vorwärts) an einem inneren Punkt über die Sehne ±`reach`.
fn central_tangent(pts: &[Point], closed: bool, i: usize, reach: f64) -> Point {
    let (a, _) = walk(pts, closed, i, reach, false);
    let (b, _) = walk(pts, closed, i, reach, true);
    unit(pts[b] - pts[a]).unwrap_or_else(|| unit(pts[(i + 1) % pts.len()] - pts[i]).unwrap_or(Point::new(1.0, 0.0)))
}

/// Tangente am Anfang (in den Abschnitt hinein) bzw. am Ende (`at_end`, ebenfalls in den Abschnitt hinein).
fn end_tangent(sec: &[Point], at_end: bool, reach: f64) -> Point {
    let n = sec.len();
    let total: f64 = sec.windows(2).map(|w| w[0].distance(w[1])).sum();
    let dist = reach.min(total / 3.0);
    let (i, _) = walk(sec, false, if at_end { n - 1 } else { 0 }, dist, !at_end);
    let from = if at_end { sec[n - 1] } else { sec[0] };
    let fallback = if at_end { sec[n - 2] - sec[n - 1] } else { sec[1] - sec[0] };
    unit(sec[i] - from).or_else(|| unit(fallback)).unwrap_or(Point::new(1.0, 0.0))
}

fn unit(v: Point) -> Option<Point> {
    let l = v.length();
    (l > 1e-12).then(|| v * (1.0 / l))
}

fn bezier(b: &[Point; 4], t: f64) -> Point {
    let s = 1.0 - t;
    b[0] * (s * s * s) + b[1] * (3.0 * s * s * t) + b[2] * (3.0 * s * t * t) + b[3] * (t * t * t)
}

/// Passt Kurven an `d` an; `t1` zeigt am Anfang, `t2` am Ende jeweils in die Kurve hinein.
fn fit_cubic(d: &[Point], t1: Point, t2: Point, tol: f64, out: &mut Vec<[Point; 4]>, depth: usize) {
    let n = d.len();
    let (first, last) = (d[0], d[n - 1]);
    if n == 2 {
        let dist = first.distance(last) / 3.0;
        out.push([first, first + t1 * dist, last + t2 * dist, last]);
        return;
    }
    let tol2 = tol * tol;
    // Passt die Kurve? Sonst der Punkt, an dem geteilt wird: der schlechteste Datenpunkt bzw. – liegen alle
    // Punkte nah genug, die Kurve schlägt aber zwischen ihnen aus – der Punkt an dieser Stelle.
    let check = |b: &[Point; 4], u: &[f64]| -> Result<(), (f64, usize)> {
        let (err, worst) = max_error(d, b, u);
        if err > tol2 {
            return Err((err, worst));
        }
        match strays(d, b, tol * 1.25) {
            None => Ok(()),
            Some(t) => {
                let i = (1..n - 1).min_by(|&i, &j| (u[i] - t).abs().total_cmp(&(u[j] - t).abs())).unwrap_or(n / 2);
                Err((err, i))
            }
        }
    };
    let mut u = chord_params(d);
    let mut b = generate(d, &u, t1, t2);
    let (err, mut split) = match check(&b, &u) {
        Ok(()) => {
            out.push(b);
            return;
        }
        Err(e) => e,
    };
    if err <= tol2 * 16.0 {
        for _ in 0..4 {
            u = reparameterize(d, &u, &b);
            b = generate(d, &u, t1, t2);
            match check(&b, &u) {
                Ok(()) => {
                    out.push(b);
                    return;
                }
                Err((_, s)) => split = s,
            }
        }
    }
    if depth > 40 {
        out.push(b);
        return;
    }
    let tc = unit(d[split - 1] - d[split + 1]).unwrap_or_else(|| unit(d[split - 1] - d[split]).unwrap_or(-t1));
    fit_cubic(&d[..=split], t1, tc, tol, out, depth + 1);
    fit_cubic(&d[split..], -tc, t2, tol, out, depth + 1);
}

fn chord_params(d: &[Point]) -> Vec<f64> {
    let mut u = Vec::with_capacity(d.len());
    let mut s = 0.0;
    u.push(0.0);
    for w in d.windows(2) {
        s += w[0].distance(w[1]);
        u.push(s);
    }
    let total = s.max(1e-12);
    u.iter_mut().for_each(|x| *x /= total);
    u
}

/// Kleinste Quadrate für die Henkellängen bei festen Tangentenrichtungen.
fn generate(d: &[Point], u: &[f64], t1: Point, t2: Point) -> [Point; 4] {
    let n = d.len();
    let (first, last) = (d[0], d[n - 1]);
    let (mut c00, mut c01, mut c11, mut x0, mut x1) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for i in 0..n {
        let t = u[i];
        let s = 1.0 - t;
        let (b0, b1, b2, b3) = (s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t);
        let (a0, a1) = (t1 * b1, t2 * b2);
        c00 += a0.dot(a0);
        c01 += a0.dot(a1);
        c11 += a1.dot(a1);
        let tmp = d[i] - (first * (b0 + b1) + last * (b2 + b3));
        x0 += a0.dot(tmp);
        x1 += a1.dot(tmp);
    }
    let det = c00 * c11 - c01 * c01;
    let (mut al, mut ar) =
        if det.abs() > 1e-12 { ((x0 * c11 - x1 * c01) / det, (c00 * x1 - c01 * x0) / det) } else { (0.0, 0.0) };
    let seg = first.distance(last);
    // Unbrauchbare Lösung (negativ, winzig oder ausufernd): Henkel ⅓ der Sehne (Wu/Barsky)
    let arc: f64 = d.windows(2).map(|w| w[0].distance(w[1])).sum();
    if al < 1e-6 * seg || ar < 1e-6 * seg || al > arc || ar > arc {
        al = seg / 3.0;
        ar = seg / 3.0;
        if seg < 1e-9 {
            al = arc / 3.0;
            ar = arc / 3.0;
        }
    }
    [first, first + t1 * al, last + t2 * ar, last]
}

/// Parameter der Stelle, an der die Kurve zwischen den Punkten weiter als `limit` von ihrem Linienzug abweicht
/// (Schlaufe, Überschwinger) – die Fehlerprüfung an den Datenpunkten allein sieht das nicht.
fn strays(d: &[Point], b: &[Point; 4], limit: f64) -> Option<f64> {
    let mut worst = (limit, None);
    for k in 1..16 {
        let t = k as f64 / 16.0;
        let q = bezier(b, t);
        let dist = d.windows(2).map(|w| crate::path::distance_to_segment(q, w[0], w[1])).fold(f64::MAX, f64::min);
        if dist > worst.0 {
            worst = (dist, Some(t));
        }
    }
    worst.1
}

/// Größter quadrierter Abstand zwischen Punkten und Kurve, mit dem Index des schlechtesten inneren Punkts.
fn max_error(d: &[Point], b: &[Point; 4], u: &[f64]) -> (f64, usize) {
    let n = d.len();
    let mut best = (0.0, n / 2);
    for i in 1..n - 1 {
        let e = bezier(b, u[i]).distance_sq(d[i]);
        if e > best.0 {
            best = (e, i);
        }
    }
    best
}

/// Newton-Raphson: Parameter so verschieben, dass der Kurvenpunkt dem Datenpunkt am nächsten liegt.
fn reparameterize(d: &[Point], u: &[f64], b: &[Point; 4]) -> Vec<f64> {
    let q1 = [(b[1] - b[0]) * 3.0, (b[2] - b[1]) * 3.0, (b[3] - b[2]) * 3.0];
    let q2 = [(q1[1] - q1[0]) * 2.0, (q1[2] - q1[1]) * 2.0];
    d.iter()
        .zip(u)
        .map(|(&p, &t)| {
            let s = 1.0 - t;
            let q = bezier(b, t);
            let d1 = q1[0] * (s * s) + q1[1] * (2.0 * s * t) + q1[2] * (t * t);
            let d2 = q2[0] * s + q2[1] * t;
            let diff = q - p;
            let den = d1.dot(d1) + diff.dot(d2);
            if den.abs() < 1e-12 { t } else { (t - diff.dot(d1) / den).clamp(0.0, 1.0) }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Affine;
    use crate::path::Segment;

    fn opts() -> FitOptions {
        FitOptions { tolerance: 0.3, smoothing: 0.5, corner_deg: 65.0, reach: 2.0 }
    }

    fn circle(r: f64, n: usize) -> Vec<Point> {
        (0..n)
            .map(|i| {
                let a = i as f64 / n as f64 * std::f64::consts::TAU;
                Point::new(50.0 + r * a.cos(), 50.0 + r * a.sin())
            })
            .collect()
    }

    #[test]
    fn circle_becomes_few_smooth_curves() {
        let p = fit_points(&circle(20.0, 126), true, &opts());
        let sp = &p.subpaths[0];
        assert!(sp.closed);
        assert!(sp.segments.len() <= 8, "{}", sp.segments.len());
        assert!(sp.segments.iter().all(|s| matches!(s, Segment::Cubic(..))));
        for q in &p.flatten(&Affine::IDENTITY, 0.01)[0].points {
            assert!((q.distance(Point::new(50.0, 50.0)) - 20.0).abs() < 0.35, "{q:?}");
        }
    }

    #[test]
    fn square_keeps_its_corners() {
        // Quadrat mit Punkten im Abstand 1
        let mut pts = Vec::new();
        for i in 0..20 {
            pts.push(Point::new(i as f64, 0.0));
        }
        for i in 0..20 {
            pts.push(Point::new(20.0, i as f64));
        }
        for i in 0..20 {
            pts.push(Point::new(20.0 - i as f64, 20.0));
        }
        for i in 0..20 {
            pts.push(Point::new(0.0, 20.0 - i as f64));
        }
        let p = fit_points(&pts, true, &opts());
        let poly = &p.flatten(&Affine::IDENTITY, 0.01)[0];
        for c in [Point::new(0.0, 0.0), Point::new(20.0, 0.0), Point::new(20.0, 20.0), Point::new(0.0, 20.0)] {
            let d = poly.points.iter().map(|q| q.distance(c)).fold(f64::MAX, f64::min);
            assert!(d < 0.05, "Ecke {c:?} abgerundet ({d})");
        }
        // drei Linien, die vierte schließt der Pfad
        assert_eq!(p.subpaths[0].segments.len(), 3, "{:?}", p.subpaths[0].segments);
    }

    #[test]
    fn jagged_line_is_straightened_and_ends_stay() {
        // Treppe mit halben Stufen um eine Gerade: wird eine Linie, Enden bleiben exakt
        let pts: Vec<Point> = (0..=40).map(|i| Point::new(i as f64, if i % 2 == 0 { 0.0 } else { 0.4 })).collect();
        let p = fit_points(&pts, false, &FitOptions { smoothing: 0.6, ..opts() });
        let sp = &p.subpaths[0];
        assert_eq!(sp.start, pts[0]);
        assert_eq!(sp.end(), pts[40]);
        assert!(sp.segments.len() <= 2, "{:?}", sp.segments);
    }

    #[test]
    fn no_smoothing_gives_polygon() {
        let p = fit_points(&circle(10.0, 60), true, &FitOptions { smoothing: 0.0, ..opts() });
        assert!(p.subpaths[0].segments.iter().all(|s| matches!(s, Segment::Line(..))));
    }
}
