//! Native media controls. The web frontend drives `navigator.mediaSession`
//! through the shim; this module mirrors that state into the platform's
//! player: MPRIS on Linux (souvlaki) and SMTC on Windows (souvlaki with the
//! main window's HWND — the thing that puts BitChord on the volume flyout,
//! the lock screen and the hardware media keys at system level). macOS
//! deliberately does not register here — see [attach]. The tray reads the
//! snapshot on every platform for its Play/Pause label.
//!
//! On both macOS and Windows the web engine also runs its own player: a
//! playing element walks straight into the system's media surface (Now
//! Playing via WKWebView, SMTC via WebView2's media session service), and a
//! registration of our own would stand beside it as a second player. macOS
//! answers by staying out of the way; Windows answers by standing the
//! webview's player down — [WEBVIEW_BROWSER_ARGS], which every WebView2 in
//! the app is built with.

use tauri::{AppHandle, Manager};

// The platform player — souvlaki — is only constructed on the two platforms
// that have no player of their own. macOS's WKWebView registers the playing
// element with Now Playing by itself (see [attach]), so none of this compiles
// there.
#[cfg(any(target_os = "linux", target_os = "windows"))]
use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig};

use crate::commands::{platform_name, AppState, MediaUpdate};

/// The WebView2 browser arguments every webview in the app is built with on
/// Windows.
///
/// The first three flags are wry's own Windows defaults, which an
/// `additional_browser_args` override replaces wholesale — they are repeated
/// here so the override loses nothing. The autoplay policy is wry's other
/// default, likewise repeated. The last two are the addition, and they are
/// the web engine's media player standing down: `MediaSessionService` is
/// Chromium's browser-side service that registers a playing element with the
/// OS's system media controls — left on, WebView2 puts its own card beside
/// souvlaki's in the volume flyout, the same two-players bug WKWebView
/// causes on macOS — and `HardwareMediaKeyHandling` is the browser's own
/// media-key handling, which would race the keys the SMTC registration
/// already delivers to souvlaki.
#[cfg(target_os = "windows")]
pub const WEBVIEW_BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,MediaSessionService,HardwareMediaKeyHandling --autoplay-policy=no-user-gesture-required";

#[derive(Debug, Clone, Default)]
pub struct MediaSnapshot {
    pub playback_state: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub artwork_url: Option<String>,
    pub duration_sec: f64,
    pub position_sec: f64,
    pub rate: f64,
}

// The controls live on the main thread (a requirement on macOS; harmless on
// Linux and Windows), so async commands reach them through `run_on_main_thread`.
#[cfg(any(target_os = "linux", target_os = "windows"))]
thread_local! {
    static CONTROLS: std::cell::RefCell<Option<MediaControls>> = const { std::cell::RefCell::new(None) };
}

/// Creates the platform controls and wires their buttons into the page.
///
/// Linux gets MPRIS. Windows gets SMTC through souvlaki's
/// `ISystemMediaTransportControlsInterop::GetForWindow`, pointed at the main
/// window's HWND; souvlaki sets the music properties, the thumbnail and the
/// timeline on every update this module receives, and its `ButtonPressed`
/// handler feeds the same `__BitChordMediaAction` eval path the tray uses —
/// so the volume flyout, the lock screen and the keyboard's media keys all
/// drive the page. (attach runs in `setup`, on the main thread, which is
/// where a WinRT interop object wants to live; the event callbacks arrive
/// through that thread's message pump, which tao runs.)
///
/// macOS registers nothing, on purpose: WKWebView registers the playing
/// element with the system Now Playing on its own — the entry that shows the
/// track, the artist and the working buttons. souvlaki's separate
/// registration used to stand BESIDE that one, and the media panel showed
/// two players for one song: a bare "BitChord" card and the webview's own.
/// With souvlaki out of the way the webview's card is the only one, and the
/// hardware keys come through WebKit's own handling of the same element.
///
/// Windows is the same disease with the opposite cure: WebView2's media
/// session service registers the playing element with SMTC on its own, and
/// a system that once showed *nothing* at all now showed the webview's
/// player AND souvlaki's — two cards, one song. Here souvlaki stays (it is
/// the only thing keeping Windows on the flyout, the lock screen and the
/// hardware keys, and the one that knows the app's own artwork and
/// timeline), and the webview's player is stood down at birth instead:
/// every WebView2 in the app is built with [WEBVIEW_BROWSER_ARGS], whose
/// `MediaSessionService` and `HardwareMediaKeyHandling` flags take the
/// engine's own registration and its own key handling out of the picture.
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn attach(app: &AppHandle) {
    #[cfg(target_os = "windows")]
    let hwnd: Option<*mut std::ffi::c_void> = match app.get_webview_window("main") {
        Some(window) => match window.hwnd() {
            Ok(hwnd) => Some(hwnd.0 as *mut std::ffi::c_void),
            Err(error) => {
                log::warn!("media controls unavailable: no HWND: {error}");
                return;
            }
        },
        None => return,
    };
    #[cfg(target_os = "linux")]
    let hwnd: Option<*mut std::ffi::c_void> = None;
    let config = PlatformConfig {
        dbus_name: "bitchord",
        display_name: "BitChord",
        hwnd,
    };
    let mut controls = match MediaControls::new(config) {
        Ok(controls) => controls,
        Err(error) => {
            log::warn!("media controls unavailable: {error}");
            return;
        }
    };
    let handle = app.clone();
    let attach_result = controls.attach(move |event| handle_event(&handle, event));
    if let Err(error) = attach_result {
        log::warn!("media controls attach failed: {error}");
        return;
    }
    CONTROLS.with(|cell| *cell.borrow_mut() = Some(controls));
}

/// macOS: no controls of our own — see [attach] for why the webview's own
/// Now Playing entry is the one the system shows.
#[cfg(target_os = "macos")]
pub fn attach(_app: &AppHandle) {}

/// The OS pressed a button: turn it into the page's own action vocabulary.
/// Only reachable where controls exist (Linux, Windows) — macOS's hardware
/// keys arrive through WebKit's own element handling.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn handle_event(app: &AppHandle, event: MediaControlEvent) {
    // The page binds play and pause separately, each guarded by the player's
    // own state; Toggle resolves against the last reported state.
    let playing = app
        .try_state::<AppState>()
        .map(|state| state.media.lock().map(|m| m.playback_state == "playing").unwrap_or(false))
        .unwrap_or(false);
    let (action, arg): (&str, Option<f64>) = match event {
        MediaControlEvent::Play => ("play", None),
        MediaControlEvent::Pause => ("pause", None),
        MediaControlEvent::Toggle => {
            if playing {
                ("pause", None)
            } else {
                ("play", None)
            }
        }
        MediaControlEvent::Next => ("nexttrack", None),
        MediaControlEvent::Previous => ("previoustrack", None),
        MediaControlEvent::Stop => ("pause", None),
        MediaControlEvent::SetPosition(position) => ("seekto", Some(position.0.as_secs_f64())),
        MediaControlEvent::SeekBy(direction, duration) => (
            match direction {
                souvlaki::SeekDirection::Forward => "seekforward",
                souvlaki::SeekDirection::Backward => "seekbackward",
            },
            Some(duration.as_secs_f64()),
        ),
        MediaControlEvent::Seek(direction) => (
            match direction {
                souvlaki::SeekDirection::Forward => "seekforward",
                souvlaki::SeekDirection::Backward => "seekbackward",
            },
            Some(10.0),
        ),
        _ => return,
    };
    eval_in_main(
        app,
        &format!(
            "window.__BitChordMediaAction({}, {})",
            serde_json::to_string(action).unwrap_or_else(|_| "\"\"".into()),
            arg.map(|seconds| seconds.to_string()).unwrap_or_else(|| "undefined".into())
        ),
    );
    if let MediaControlEvent::Quit = event {
        let _ = app.exit(0);
    }
}

/// Runs a script in the main window, from any thread.
pub fn eval_in_main(app: &AppHandle, script: &str) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.eval(script);
    }
}

/// The shim's throttled updates land here: the snapshot feeds the tray, and
/// the platform controls are refreshed on the main thread.
pub fn apply_update(app: &AppHandle, update: MediaUpdate) {
    let state = app.state::<AppState>();
    let mut snapshot = state.media.lock().unwrap().clone();

    match update.kind.as_str() {
        "metadata" => {
            if let Some(metadata) = &update.metadata {
                // A genuinely new track (not a re-report of the same one)
                // starts its own timeline: drop the old track's position at
                // once, so the island and MPRIS never count down the previous
                // song's remaining time over the new one's title while the
                // page's first position update is still in flight.
                let track_changed = snapshot.title != metadata.title || snapshot.artwork_url != metadata.artwork_url;
                snapshot.title = metadata.title.clone();
                snapshot.artist = metadata.artist.clone();
                snapshot.album = metadata.album.clone();
                snapshot.artwork_url = metadata.artwork_url.clone();
                if track_changed {
                    snapshot.position_sec = 0.0;
                }
            }
        }
        "playback" => {
            snapshot.playback_state = update.state.clone().unwrap_or_default();
        }
        "position" => {
            snapshot.duration_sec = update.duration;
            snapshot.position_sec = update.position;
            snapshot.rate = if update.rate > 0.0 { update.rate } else { 1.0 };
        }
        _ => {}
    }
    *state.media.lock().unwrap() = snapshot.clone();

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        let for_controls = snapshot.clone();
        let kind = update.kind.clone();
        let _ = app.run_on_main_thread(move || {
            CONTROLS.with(|cell| {
                let mut guard = cell.borrow_mut();
                let Some(controls) = guard.as_mut() else { return };
                // souvlaki's metadata borrows: `as_deref` maps Option<String> → Option<&str>.
                let result = match kind.as_str() {
                    "metadata" => controls.set_metadata(MediaMetadata {
                        title: for_controls.title.as_deref(),
                        artist: for_controls.artist.as_deref(),
                        album: for_controls.album.as_deref(),
                        duration: Some(std::time::Duration::from_secs_f64(for_controls.duration_sec.max(0.0))),
                        cover_url: for_controls.artwork_url.as_deref(),
                    }),
                    "playback" | "position" => controls.set_playback(match for_controls.playback_state.as_str() {
                        "playing" => MediaPlayback::Playing {
                            progress: Some(MediaPosition(std::time::Duration::from_secs_f64(for_controls.position_sec.max(0.0)))),
                        },
                        "paused" => MediaPlayback::Paused {
                            progress: Some(MediaPosition(std::time::Duration::from_secs_f64(for_controls.position_sec.max(0.0)))),
                        },
                        _ => MediaPlayback::Stopped,
                    }),
                    _ => Ok(()),
                };
                if let Err(error) = result {
                    log::warn!("media controls update failed: {error}");
                }
            });
        });
    }

    crate::tray::sync_play_label(app, &snapshot);

    // The Dynamic Island redraws from the same snapshot the tray reads —
    // one truth for playback, three mirrors (tray, MPRIS, island).
    crate::island::sync(app, &snapshot);
}

/// Whether the player is currently sounding, for the tray's label.
pub fn is_playing(snapshot: &MediaSnapshot) -> bool {
    snapshot.playback_state == "playing"
}

#[allow(dead_code)]
fn _platform_reference() {
    let _ = platform_name();
}
