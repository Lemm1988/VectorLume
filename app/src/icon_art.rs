// Programmsymbol als RGBA-Pixel. Wird von main.rs (Fenstersymbol) und build.rs (.ico in der .exe) eingebunden.

/// Abgerundetes dunkles Quadrat mit leuchtendem Laserpunkt und Ring, beliebige Kantenlänge.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let s = size as f32;
    let c = (s - 1.0) / 2.0;
    let radius = s * 0.22;
    let half = s / 2.0;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 - c, y as f32 - c);
            // Abstand zum Rand des abgerundeten Quadrats (negativ = innen), 1 px weiche Kante.
            let qx = (fx.abs() - (half - radius)).max(0.0);
            let qy = (fy.abs() - (half - radius)).max(0.0);
            let edge = (qx * qx + qy * qy).sqrt() - radius;
            let alpha = (0.5 - edge).clamp(0.0, 1.0);

            let r = (fx * fx + fy * fy).sqrt() / s;
            let glow = (1.0 - r / 0.36).clamp(0.0, 1.0);
            let ring = (1.0 - ((r - 0.31).abs() / 0.035)).clamp(0.0, 1.0);
            let core = (1.0 - r / 0.09).clamp(0.0, 1.0);
            let v = glow.powf(1.8).max(ring * 0.95);
            let (br, bg, bb) = (20.0, 24.0, 34.0);
            let (ar, ag, ab) = (92.0, 140.0, 255.0);
            let px = [
                (br + (ar - br) * v + 163.0 * core).min(255.0),
                (bg + (ag - bg) * v + 115.0 * core).min(255.0),
                (bb + (ab - bb) * v).min(255.0),
                255.0 * alpha,
            ];
            rgba.extend(px.iter().map(|&v| v.round() as u8));
        }
    }
    rgba
}
