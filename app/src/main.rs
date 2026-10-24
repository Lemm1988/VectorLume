//! VectorLume – Laser-Software für GRBL-Diodenlaser.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod canvas;
mod dialogs;
mod history;
mod settings;
mod theme;
mod ui_main;
mod ui_side;

fn icon() -> egui::IconData {
    // Programmsymbol zur Laufzeit erzeugen: Lichtpunkt auf dunklem Grund.
    let size = 64u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 - 31.5, y as f32 - 31.5);
            let r = (fx * fx + fy * fy).sqrt();
            let corner = (fx.abs().max(fy.abs()) > 30.0) as u8;
            let glow = (1.0 - (r / 22.0)).clamp(0.0, 1.0);
            let ring = (1.0 - ((r - 20.0).abs() / 2.0)).clamp(0.0, 1.0);
            let v = glow.powf(1.6).max(ring * 0.9);
            let (br, bg, bb) = (22.0, 26.0, 36.0);
            let (ar, ag, ab) = (92.0, 140.0, 255.0);
            let px = [
                (br + (ar - br) * v + 120.0 * glow.powf(4.0)).min(255.0) as u8,
                (bg + (ag - bg) * v + 100.0 * glow.powf(4.0)).min(255.0) as u8,
                (bb + (ab - bb) * v).min(255.0) as u8,
                if corner == 1 { 0 } else { 255 },
            ];
            rgba.extend_from_slice(&px);
        }
    }
    egui::IconData { rgba, width: size, height: size }
}

fn main() -> eframe::Result {
    // Unter Windows DirectX 12 bevorzugen: Vulkan-Treiber mancher Intel-Grafikchips stellen das Fenster nicht dar.
    #[cfg(windows)]
    if std::env::var_os("WGPU_BACKEND").is_none() {
        // SAFETY: wird vor dem Start weiterer Threads aufgerufen.
        unsafe { std::env::set_var("WGPU_BACKEND", "dx12") };
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("VectorLume")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1024.0, 640.0])
            .with_drag_and_drop(true)
            .with_icon(icon()),
        ..Default::default()
    };
    eframe::run_native("VectorLume", options, Box::new(|cc| Ok(Box::new(app::VectorLumeApp::new(cc)))))
}
