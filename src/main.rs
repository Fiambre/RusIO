#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod session;
mod terminal;
mod theme;
mod ui;
mod vnc;

use app::PulentoApp;
use eframe::egui;

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 780.0])
            .with_min_inner_size([900.0, 550.0])
            .with_title("Pulento - SSH, VNC & Terminal Manager")
            .with_active(true),
        ..Default::default()
    };

    eframe::run_native(
        "Pulento",
        native_options,
        Box::new(|cc| {
            theme::apply_theme(&cc.egui_ctx);
            Ok(Box::new(PulentoApp::new(cc)))
        }),
    )
}
