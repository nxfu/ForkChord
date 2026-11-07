//! The Dynamic Island: a frameless, always-on-top, transparent overlay pinned
//! to the top centre of the primary display, playing the now-playing card the
//! way a phone does. Ported from Airbeats' dynamic-island window
//! (d0x-dev/Airbeats-Desktop, `electron-app/dynamic-island.*`), then re-dressed
//! so it reads as BitChord: the frosted-glass recipe of the app's own player
//! pill (`glass-pill` in the web app's global.css), DM Sans, the key colour,
//! and the shell's existing control path — the island never talks to the page
//! directly, it presses the same `window.__BitChordMediaAction` buttons the
//! tray does.
//!
//! The plumbing is the same shape as everything else in the shell:
//!
//!   main webview → shim (`media_update`) → [media::apply_update] → [sync]
//!   evals the snapshot into the island webview, and the island's presses come
//!   back as commands (`island_action`) that are eval'd into the main window —
//!   identical to the tray's menu items.
//!
//! The window itself starts out exactly the size of the mini pill and is
//! resized when the page expands, like Airbeats resized theirs: a transparent
//! window still eats clicks over its invisible areas, so the OS window must
//! always hug the visible pill. The window-state plugin must never track this
//! window (see the denylist in lib.rs) — its place is computed, not restored.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

use crate::commands::AppState;
use crate::media::MediaSnapshot;

const ISLAND_SHIM: &str = include_str!("../shim/island.js");

/// Whether the main window currently owns the focus. The pill only exists for
/// a glance away from BitChord — while the app is frontmost the page is the
/// now-playing card, and an island over it would repeat what it already says
/// (and sit in the way of the page's own top). The flag starts false: a
/// session that began with the window hidden to the tray has never been
/// focused, so the island may show as soon as something is playing; a normal
/// launch corrects it with the window's first Focused(true) event.
static MAIN_FOCUSED: AtomicBool = AtomicBool::new(false);

/// The main window's focus changed (lib.rs's window-event hook). The flag is
/// the shell's own answer for the island's boot-time query
/// (`island_focus_state` — the page asks once, before its first sync), and
/// every change is pushed into the island page, which re-decides what to show.
/// Regaining focus also clears a right-click dismissal: the island comes back
/// only after the user has been back in the app and left it again.
pub fn notify_main_focus(app: &AppHandle, focused: bool) {
    MAIN_FOCUSED.store(focused, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window("island") {
        let _ = window.eval(&format!("window.__IslandFocus && window.__IslandFocus({focused})"));
    }
}

/// What the shell last saw, for the island's one boot-time ask.
pub fn main_focused() -> bool {
    MAIN_FOCUSED.load(Ordering::SeqCst)
}

/// Logical sizes of the two states, matching `island.css`. The mini pill is a
/// whisker wider than the artwork + title need, so the title gets room to
/// breathe; the expanded card holds art, scrubber and transport.
const MINI_SIZE: (f64, f64) = (216.0, 44.0);
const FULL_SIZE: (f64, f64) = (432.0, 188.0);

/// Clearance from the screen's top edge. macOS keeps the pill below the menu
/// bar; everywhere else it hugs the edge like the player pill hugs the bottom.
const TOP_INSET: f64 = if cfg!(target_os = "macos") { 32.0 } else { 12.0 };

/// The island ships on, like Airbeats'; the settings toggle writes the first
/// flip into the store, and `None` stays the "never chosen" default that
/// means on.
pub fn enabled(app: &AppHandle) -> bool {
    app.try_state::<AppState>()
        .map(|state| state.core.store.read(|d| d.island_enabled) != Some(false))
        .unwrap_or(true)
}

/// Reads the preference and builds the window when the feature is on. Called
/// once from setup; a failure here is a missing overlay, never a dead app.
pub fn init(app: &AppHandle) {
    apply_enabled(app);
}

/// Builds the overlay window. Created hidden and only shown once it sits at
/// the top centre — the page's CSS starts fully transparent either way, so
/// nothing flashes, wherever the WM first dropped the frame.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    if let Some(existing) = app.get_webview_window("island") {
        let _ = existing.show();
        return Ok(());
    }
    #[allow(unused_mut)]
    let mut builder = WebviewWindowBuilder::new(app, "island", WebviewUrl::App("/island.html".into()))
        .title("BitChord Dynamic Island")
        .inner_size(MINI_SIZE.0, MINI_SIZE.1)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .focused(false)
        .maximizable(false)
        .minimizable(false)
        .visible(false)
        .initialization_script(ISLAND_SHIM);
    // The first click lands on the pill instead of only activating the
    // window — the island is a glanceable control, not something to focus.
    #[cfg(target_os = "macos")]
    {
        builder = builder.accept_first_mouse(true);
    }
    let window = builder.build()?;
    recenter(&window, MINI_SIZE.0);
    // The page is hidden until its first sync; clicks pass through it.
    let _ = window.set_ignore_cursor_events(true);
    let _ = window.show();
    Ok(())
}

pub fn destroy(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("island") {
        let _ = window.close();
    }
}

/// Applies the stored preference: builds the window when the island is on,
/// tears it down when it is off. Everything that flips the preference — the
/// settings page's toggle (through the bridge's `island.setEnabled`) — lands
/// here. A failed window build must not leave a lie in the store: the store
/// was written first (in the core), and this reads it back, so the next open
/// of Settings shows what the app actually did.
pub fn apply_enabled(app: &AppHandle) {
    if enabled(app) {
        if let Err(error) = create(app) {
            log::warn!("dynamic island could not be created: {error}");
        }
    } else {
        destroy(app);
    }
}

/// The one stop every shim update makes (metadata, playback, position): the
/// island interpolates the position itself between syncs, so this can stay as
/// chatty as the snapshot already is. The JSON object literal is eval'd, the
/// same shape as `__BitChordBridgeEvent`'s push into the main window.
pub fn sync(app: &AppHandle, snapshot: &MediaSnapshot) {
    let Some(window) = app.get_webview_window("island") else { return };
    let payload = serde_json::json!({
        "state": snapshot.playback_state,
        "title": snapshot.title,
        "artist": snapshot.artist,
        "album": snapshot.album,
        "artworkUrl": snapshot.artwork_url,
        "duration": snapshot.duration_sec,
        "position": snapshot.position_sec,
        "rate": snapshot.rate,
    });
    let _ = window.eval(&format!("window.__IslandSync && window.__IslandSync({payload})"));
}

/// Grows or shrinks the OS window around the page's animated card, then keeps
/// it centred: the pill is always top centre, at both sizes. Both writes go
/// through one main-thread task — a transparent window resized in one task and
/// moved in the next would paint the intermediate frame, and the pill would
/// visibly jump sideways on every expand and collapse.
pub fn apply_size(app: &AppHandle, expanded: bool) {
    let (width, height) = if expanded { FULL_SIZE } else { MINI_SIZE };
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window("island") {
            apply_geometry(&window, width, height);
        }
    });
}

/// The geometry half of [apply_size], for one logical size, already on the
/// main thread.
///
/// The size and the centring must land in the *same* presented frame, or the
/// morph reads in two steps. On Windows a plain `set_size` keeps the top-left
/// corner anchored (SWP_NOMOVE), so shrinking the 432px window to 216px parked
/// the pill left of centre for a frame before `recenter` walked it home — the
/// "centre-left, then centre" two-step. One atomic `SetWindowPos` that carries
/// position and size together cannot have a frame between them, because there
/// is no *between*: it is a single WM_WINDOWPOSCHANGED.
///
/// macOS has the same disease, one floor up: tao's `set_size` is an async
/// `setContentSize:` (bottom-left origin anchored) and `set_position` an async
/// `setFrameOrigin:`, each queued separately onto the main thread even when
/// asked from it — two presented frames, the same two-step. So the island
/// never calls them; it sends NSWindow one `setFrame:display:` carrying both.
/// Both states hang from the same top-centre point, so the pin is read straight
/// off the current AppKit frame — no monitor math, no scale factor, no
/// coordinate flipping, and correct on every display the island already sits
/// on. `display: false` defers the redraw into the same update cycle the
/// webview's first grown frame rides in.
#[cfg(target_os = "macos")]
fn apply_geometry(window: &tauri::WebviewWindow, width: f64, height: f64) {
    use objc2::msg_send;
    use objc2::runtime::NSObject;
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    let atomic = (|| -> Option<()> {
        let ns_window = window.ns_window().ok()? as *mut NSObject;
        if ns_window.is_null() {
            return None;
        }
        // AppKit frame: origin is the bottom-left corner, y grows upward,
        // all in points (logical pixels).
        let frame: NSRect = unsafe { msg_send![ns_window, frame] };
        if frame.size.width <= 0.0 || frame.size.height <= 0.0 {
            return None; // an unfitted window has no pin worth trusting
        }
        let centre_x = frame.origin.x + frame.size.width / 2.0;
        let top_y = frame.origin.y + frame.size.height;
        let new_frame = NSRect::new(
            NSPoint::new(centre_x - width / 2.0, top_y - height),
            NSSize::new(width, height),
        );
        let _: () = unsafe { msg_send![ns_window, setFrame: new_frame, display: false] };
        Some(())
    })();
    // No raw handle (should not happen): the old two-step path is better
    // than no resize at all.
    if atomic.is_none() {
        let _ = window.set_size(LogicalSize::new(width, height));
        recenter(window, width);
    }
}

/// Linux: gdk_window_move_resize carries position and size in one request —
/// one X11 ConfigureWindow, nothing to paint between — where gtk's move and
/// resize are two server round-trips a WM can present apart, the same
/// two-step. Coordinates stay inside one API (gdk's own root space, logical
/// pixels), so the current centre reads straight from the window. On Wayland
/// the move is advisory as ever; sizing still applies, as it did before.
#[cfg(target_os = "linux")]
fn apply_geometry(window: &tauri::WebviewWindow, width: f64, height: f64) {
    use gtk::prelude::WidgetExt;

    let atomic = window
        .gtk_window()
        .ok()
        .and_then(|gtk_window| gtk_window.window())
        .map(|gdk_window| {
            let (x, y) = gdk_window.position();
            let centre_x = f64::from(x) + f64::from(gdk_window.width()) / 2.0;
            let new_x = (centre_x - width / 2.0).round() as i32;
            gdk_window.move_resize(new_x, y, width.round() as i32, height.round() as i32);
        });
    if atomic.is_none() {
        let _ = window.set_size(LogicalSize::new(width, height));
        recenter(window, width);
    }
}

/// Windows: see the doc comment on [apply_geometry] — one atomic
/// SetWindowPos that carries position and size together.
#[cfg(target_os = "windows")]
fn apply_geometry(window: &tauri::WebviewWindow, width: f64, height: f64) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER};

    let monitor = window.current_monitor().ok().flatten();
    let Some(monitor) = monitor else {
        let _ = window.set_size(LogicalSize::new(width, height));
        recenter(window, width);
        return;
    };
    let scale = monitor.scale_factor();
    let x = (monitor.position().x as f64 + ((monitor.size().width as f64 - width * scale) / 2.0).round()) as i32;
    let y = (monitor.position().y as f64 + (TOP_INSET * scale).round()) as i32;
    let w = (width * scale).round() as i32;
    let h = (height * scale).round() as i32;
    match window.hwnd() {
        Ok(hwnd) => unsafe {
            // Position and size in one WM_WINDOWPOSCHANGED — no intermediate
            // frame exists to paint. Z order stays (always-on-top), no focus
            // is stolen: the island is a glance, never an activation.
            let _ = SetWindowPos(hwnd, Some(HWND::default()), x, y, w, h, SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE);
        },
        // No raw handle (should not happen): the old two-step path is better
        // than no resize at all.
        Err(_) => {
            let _ = window.set_size(LogicalSize::new(width, height));
            recenter(window, width);
        }
    }
}

/// While hidden the pill must not block what is under it — a transparent
/// window otherwise swallows clicks on its whole rectangle.
pub fn set_click_through(app: &AppHandle, ignore: bool) {
    if let Some(window) = app.get_webview_window("island") {
        let _ = window.set_ignore_cursor_events(ignore);
    }
}

/// The island's presses, resolved the same way the tray's menu items are:
/// transport goes to the page's media-session handlers, `open` raises the
/// app. `arg` carries the seek target in seconds.
pub fn handle_action(app: &AppHandle, action: &str, arg: Option<f64>) {
    match action {
        "open" => crate::tray::show_main(app),
        "toggle" => {
            let playing = app
                .try_state::<AppState>()
                .map(|state| state.media.lock().map(|m| m.playback_state == "playing").unwrap_or(false))
                .unwrap_or(false);
            let resolved = if playing { "pause" } else { "play" };
            crate::media::eval_in_main(app, &format!("window.__BitChordMediaAction('{resolved}')"));
        }
        "seekto" => {
            let seconds = arg.unwrap_or(0.0);
            crate::media::eval_in_main(app, &format!("window.__BitChordMediaAction('seekto', {seconds})"));
        }
        "play" | "pause" | "nexttrack" | "previoustrack" => {
            crate::media::eval_in_main(app, &format!("window.__BitChordMediaAction('{action}')"));
        }
        _ => {}
    }
}

/// Centres the window horizontally on the monitor it lives on, [TOP_INSET]
/// from the top. Physical pixels throughout: the monitor reports them and the
/// position wants them, with the scale factor doing the one conversion.
fn recenter(window: &tauri::WebviewWindow, width: f64) {
    let monitor = window.current_monitor().ok().flatten();
    let Some(monitor) = monitor else { return };
    let scale = monitor.scale_factor();
    let monitor_x = monitor.position().x as f64;
    let monitor_w = monitor.size().width as f64;
    let x = monitor_x + ((monitor_w - width * scale) / 2.0).round();
    let y = monitor.position().y as f64 + (TOP_INSET * scale).round();
    let _ = window.set_position(PhysicalPosition::new(x as i32, y as i32));
}
