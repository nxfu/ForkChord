//! Stand-in compile + behaviour check for `island.rs::arm_click_through`
//! (the Linux launch-panic fix, tao event_loop.rs:459).
//!
//! The stubs mirror the signatures verified against the pinned crate sources
//! line by line:
//!   * tauri 2.12.5 `AppHandle::run_on_main_thread<F: FnOnce() + Send + 'static>`
//!   * tauri 2.12.5 `WebviewWindow::gtk_window() -> Result<gtk::ApplicationWindow>`
//!     and `set_ignore_cursor_events(bool) -> Result<()>`
//!   * gtk 0.18.2 `WidgetExt::window(&self) -> Option<gdk::Window>` (owned)
//!   * gtk 0.18.2 re-exports glib (`pub use glib;`)
//!   * glib 0.18.5 `idle_add_local<F: FnMut() -> ControlFlow + 'static>` (no
//!     Send bound, main-context thread only), `ControlFlow` enum,
//!     non-`must_use` `SourceId`.
//!
//! The helper body is verbatim from island.rs with `cfg` removed and the
//! crate paths pointed at the stubs. Compiled with `-D warnings`; main()
//! drives the scenarios (window gone / realized / never realized / un-ignore).

use std::sync::{Arc, Mutex};

// ---------- glib 0.18.5 shape ----------
pub mod glib {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ControlFlow {
        Continue,
        Break,
    }

    #[derive(Debug)]
    pub struct SourceId(#[allow(dead_code)] u32);

    /// glib: `FnMut() -> ControlFlow + 'static`, no Send, main-context only.
    /// Stub "main loop": runs the source to completion (one call = one loop
    /// iteration), so the scenarios below are driven for real.
    pub fn idle_add_local<F>(func: F) -> SourceId
    where
        F: FnMut() -> ControlFlow + 'static,
    {
        let mut func = func;
        let mut id = 0;
        loop {
            id += 1;
            if func() == ControlFlow::Break {
                break;
            }
            assert!(id < 1000, "stub main loop ran away");
        }
        SourceId(id)
    }
}

// ---------- gtk 0.18.2 shape ----------
pub mod gtk {
    pub use super::glib;

    #[derive(Debug, Clone)]
    pub struct GdkWindow;

    #[derive(Debug, Clone)]
    pub struct GtkWindow {
        pub gdk: Option<GdkWindow>,
    }

    pub mod prelude {
        pub trait WidgetExt {
            fn window(&self) -> Option<super::GdkWindow>;
        }
        impl WidgetExt for super::GtkWindow {
            fn window(&self) -> Option<super::GdkWindow> {
                // owned Option, like from_glib_none
                self.gdk.as_ref().map(|_| super::GdkWindow)
            }
        }
    }
}

// ---------- tauri 2.12.5 shape ----------
mod tauri {
    use super::gtk::GtkWindow;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone)]
    pub struct Error;

    /// Send+Sync like the real handle: tauri's WebviewWindow is an id-based
    /// handle, so the stub must be too — otherwise the stub, not the fix,
    /// would fail the real `FnOnce + Send` bound of run_on_main_thread.
    #[derive(Debug, Clone)]
    pub struct WebviewWindow {
        pub gtk: Arc<Mutex<GtkWindow>>,
        pub ignore_calls: Arc<Mutex<Vec<bool>>>,
    }

    impl WebviewWindow {
        pub fn gtk_window(&self) -> Result<GtkWindow, Error> {
            Ok(self.gtk.lock().unwrap().clone())
        }
        pub fn set_ignore_cursor_events(&self, ignore: bool) -> Result<(), Error> {
            self.ignore_calls.lock().unwrap().push(ignore);
            Ok(())
        }
    }

    pub trait Manager {
        fn get_webview_window(&self, label: &str) -> Option<WebviewWindow>;
    }

    #[derive(Clone)]
    pub struct AppHandle {
        pub island: Option<WebviewWindow>,
    }

    impl Manager for AppHandle {
        fn get_webview_window(&self, label: &str) -> Option<WebviewWindow> {
            assert_eq!(label, "island");
            self.island.clone()
        }
    }

    impl AppHandle {
        /// The real bound: `F: FnOnce() + Send + 'static`. The stub runs it
        /// inline ("main thread"), so a non-Send capture must not exist.
        pub fn run_on_main_thread<F: FnOnce() + Send + 'static>(&self, f: F) -> Result<(), Error> {
            f();
            Ok(())
        }
    }
}

use tauri::{AppHandle, Manager, WebviewWindow};

// ---------- the fix, verbatim from island.rs (cfg removed) ----------
fn arm_click_through(app: &AppHandle, ignore: bool) {
    use gtk::prelude::WidgetExt;

    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let tries = std::cell::Cell::new(0u32);
        gtk::glib::idle_add_local(move || {
            let Some(window) = handle.get_webview_window("island") else {
                return gtk::glib::ControlFlow::Break; // torn down while waiting
            };
            let realized = window
                .gtk_window()
                .map(|gtk_window| gtk_window.window().is_some())
                .unwrap_or(false);
            if !realized {
                let passes = tries.get() + 1;
                if passes > 250 {
                    log_warn("dynamic island: window never realized; click-through not armed");
                    return gtk::glib::ControlFlow::Break;
                }
                tries.set(passes);
                return gtk::glib::ControlFlow::Continue;
            }
            let _ = window.set_ignore_cursor_events(ignore);
            gtk::glib::ControlFlow::Break
        });
    });
}

fn log_warn(_msg: &str) {}

// ---------- scenarios ----------
use gtk::GtkWindow;

fn stub_window(realized: bool) -> WebviewWindow {
    WebviewWindow {
        gtk: Arc::new(Mutex::new(GtkWindow {
            gdk: if realized { Some(gtk::GdkWindow) } else { None },
        })),
        ignore_calls: Arc::new(Mutex::new(Vec::new())),
    }
}

fn main() {
    // 1. No island window (torn down / never existed): the idle breaks at
    //    once — no panic, nothing sent.
    let app = AppHandle { island: None };
    arm_click_through(&app, true);
    println!("1. window-gone path: breaks cleanly, no request");

    // 2. Realized window (the normal state once the queued show has run):
    //    exactly one request, armed with the right flag.
    let app = AppHandle { island: Some(stub_window(true)) };
    let calls = app.island.as_ref().unwrap().ignore_calls.clone();
    arm_click_through(&app, true);
    assert_eq!(*calls.lock().unwrap(), vec![true], "one ignore=true once realized");
    println!("2. realized path: exactly one request, ignore=true");

    // 3. Never realized (the crash state — tao used to unwrap the None here):
    //    gives up after the 250th pass with a warning; no request, no panic.
    let app = AppHandle { island: Some(stub_window(false)) };
    let calls = app.island.as_ref().unwrap().ignore_calls.clone();
    arm_click_through(&app, true);
    assert!(calls.lock().unwrap().is_empty(), "no request without realization");
    println!("3. never-realized path: 250-pass give-up, no request, no panic");

    // 4. The page's island_overlay(false) flip (pill made visible) forwards
    //    the false through the same arm.
    let app = AppHandle { island: Some(stub_window(true)) };
    let calls = app.island.as_ref().unwrap().ignore_calls.clone();
    arm_click_through(&app, false);
    assert_eq!(*calls.lock().unwrap(), vec![false]);
    println!("4. un-ignore path: ignore=false forwarded");

    println!("ALL SCENARIOS PASS");
}
