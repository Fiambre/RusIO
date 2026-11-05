mod app;
mod editor;
mod registry;
mod session;
mod tunnels;
mod ui;

use app::RusioApp;
use eframe::egui;
use std::fs::OpenOptions;
use std::io::Write;
use std::time::SystemTime;

fn log_debug(msg: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open("rusio_debug.log") {
        let ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let _ = writeln!(f, "[{}] {}", ts, msg);
    }
}

#[cfg(windows)]
fn switch_to_default_desktop() {
    extern "system" {
        fn OpenDesktopA(
            lpszDesktop: *const u8,
            dwFlags: u32,
            fInherit: i32,
            dwDesiredAccess: u32,
        ) -> *mut std::ffi::c_void;
        fn SetThreadDesktop(hDesktop: *mut std::ffi::c_void) -> i32;
        fn GetLastError() -> u32;
    }
    unsafe {
        let h = OpenDesktopA(b"Default\0".as_ptr(), 0, 0, 0x1000_0000);
        log_debug(&format!("OpenDesktopA returned: {:?}", h));
        if !h.is_null() {
            let res = SetThreadDesktop(h);
            log_debug(&format!("SetThreadDesktop returned: {}, GetLastError: {}", res, GetLastError()));
        }
    }
}

fn main() {
    log_debug("--- RusIO main starting ---");

    #[cfg(windows)]
    switch_to_default_desktop();

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 780.0])
        .with_min_inner_size([900.0, 550.0])
        .with_title("RusIO - SSH, VNC & Terminal Manager")
        .with_decorations(false)
        .with_visible(true)
        .with_active(true);

    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")) {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    log_debug("Calling eframe::run_native...");

    let res = eframe::run_native(
        "RusIO",
        native_options,
        Box::new(|cc| {
            log_debug("App creator closure called!");
            rusio_plugin_api::theme::apply_theme(&cc.egui_ctx);
            Ok(Box::new(RusioApp::new(cc)))
        }),
    );

    log_debug(&format!("eframe::run_native finished with: {:?}", res));
}
