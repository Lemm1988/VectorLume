//! VectorLume – Laser-Software für GRBL-Diodenlaser.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autosave;
mod box_gen;
mod camera;
mod canvas;
mod center_finder;
mod crash;
mod dialogs;
mod gear_tool;
mod grbl_params;
mod hinge_tool;
mod history;
mod history_window;
mod i18n;
mod i18n_en;
mod icon_art;
mod kerf_tool;
mod library;
mod manual;
mod materials;
mod model_tool;
mod node_tool;
mod print_cut;
mod puzzle_tool;
mod rotary_ui;
mod settings;
mod shape_ops;
mod snapping;
mod tab_tool;
mod text_tool;
mod theme;
mod trace_ui;
mod ui_main;
mod ui_side;
#[cfg(test)]
mod ui_tests;
mod units;
mod warp_tool;

fn icon() -> egui::IconData {
    let size = 64;
    egui::IconData { rgba: icon_art::icon_rgba(size), width: size, height: size }
}

fn main() -> eframe::Result {
    // Unter Windows DirectX 12 bevorzugen: Vulkan-Treiber mancher Intel-Grafikchips stellen das Fenster nicht dar.
    #[cfg(windows)]
    if std::env::var_os("WGPU_BACKEND").is_none() {
        // SAFETY: wird vor dem Start weiterer Threads aufgerufen.
        unsafe { std::env::set_var("WGPU_BACKEND", "dx12") };
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    crash::install();
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
