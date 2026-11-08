//! Compile check for the two fixed patterns in chrome.rs / commands.rs,
//! against stubs that mirror the real Tauri signatures (borrowed receiver on
//! run_on_main_thread, move-only closure, Manager-trait-shaped lookup).

pub trait Manager {
    fn get_webview_window(&self, label: &str) -> Option<Window>;
}

pub struct Window;
impl Window {
    #[allow(clippy::result_unit_err)]
    pub fn ns_window(&self) -> Result<*mut std::ffi::c_void, ()> {
        Ok(std::ptr::null_mut())
    }
}

#[derive(Clone)]
pub struct AppHandle;
impl Manager for AppHandle {
    fn get_webview_window(&self, _label: &str) -> Option<Window> {
        Some(Window)
    }
}
impl AppHandle {
    #[allow(clippy::result_unit_err)]
    pub fn run_on_main_thread<F: FnOnce() + Send + 'static>(&self, _f: F) -> Result<(), ()> {
        Ok(())
    }
}

// ---- The fixed chrome.rs body, verbatim modulo the stub types. ----

pub fn set_traffic_lights_visible(app: &AppHandle, visible: bool) {
    // The hop's receiver is a second clone: `run_on_main_thread` borrows its
    // receiver while the closure runs, and the closure itself needs the
    // handle by value to reach the window — one clone cannot be both.
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        let Some(window) = app.get_webview_window("main") else {
            return;
        };
        let Ok(_ns_window) = window.ns_window() else {
            return;
        };
        let _ = visible;
    });
}

// ---- The fixed commands.rs import shape. ----

mod chrome {
    pub fn set_traffic_lights_visible(_app: &super::AppHandle, _visible: bool) {}
}

mod commands {
    use super::AppHandle;

    // This is `use crate::{agent, chrome, island, media};` in the real file —
    // the exact line that was missing `chrome` and broke all three CI builds.
    use super::chrome;

    pub fn set_traffic_lights_visible(visible: bool, app: AppHandle) -> Result<(), String> {
        chrome::set_traffic_lights_visible(&app, visible);
        Ok(())
    }
}

fn main() {
    let _ = commands::set_traffic_lights_visible(true, AppHandle);
}
