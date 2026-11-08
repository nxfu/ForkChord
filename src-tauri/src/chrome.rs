//! Window chrome the page asks for, one platform at a time.
//!
//! macOS: the traffic lights. The full-screen player is the app's own
//! full-bleed stage — it draws its own close button at the top-left corner,
//! exactly where the system hangs the traffic lights (TitleBarStyle::Overlay
//! keeps them floating over the glass), so the two collide the moment the
//! player opens. Windows and Linux solved this by having no native buttons in
//! the player at all (WindowControls stands down); macOS gets the same answer
//! by hiding the lights for as long as the player is open and bringing them
//! back the moment it closes. Escape and the player's own X still leave the
//! player, and the drag zone is already unbound in that view (Round 8), so
//! nothing the lights could do is reachable there anyway.
//!
//! Hidden with `-[NSButton setHidden:]` on the three standard window buttons
//! — the same buttons tao's `inset_traffic_lights` reaches — rather than any
//! style-mask change: the window keeps its frame, its shadow and its
//! resizeability, and AppKit does not resurrect hidden buttons on the events
//! that only reset their *positions* (title changes, fullscreen transitions).

// Manager is only exercised inside the macOS body (get_webview_window);
// on Windows and Linux the no-op below would leave it unused.
use tauri::AppHandle;
#[cfg(target_os = "macos")]
use tauri::Manager;

#[cfg(target_os = "macos")]
pub fn set_traffic_lights_visible(app: &AppHandle, visible: bool) {
    // NSWindow is MainThreadOnly: the whole job runs on the main thread. The
    // command arrives off-thread (every async command does), so it hops.
    // The hop's receiver is a second clone: `run_on_main_thread` borrows its
    // receiver while the closure runs, and the closure itself needs the
    // handle by value to reach the window — one clone cannot be both.
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        let Some(window) = app.get_webview_window("main") else {
            return;
        };
        let Ok(ns_window) = window.ns_window() else {
            return;
        };
        unsafe { set_visible(ns_window, visible) };
    });
}

#[cfg(target_os = "macos")]
unsafe fn set_visible(ns_window: *mut std::ffi::c_void, visible: bool) {
    use objc2_app_kit::{NSWindow, NSWindowButton};

    let window = ns_window as *mut NSWindow;
    for button in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        if let Some(button) = (*window).standardWindowButton(button) {
            // NSButton inherits the setter from NSView.
            button.setHidden(!visible);
        }
    }
}

/// No traffic lights anywhere else: Windows and Linux build frameless, and
/// the page's own bar already stands down in the player.
#[cfg(not(target_os = "macos"))]
pub fn set_traffic_lights_visible(_app: &AppHandle, _visible: bool) {}
