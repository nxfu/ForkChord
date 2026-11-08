//! Compiles the changed shell modules (media.rs, chrome.rs) against tauri- and
//! souvlaki-shaped stubs whose signatures were taken verbatim from the locked
//! crate sources (tauri 2.12.5, souvlaki 0.8.3). On this host no system
//! libdbus/gtk exists, so the real crates cannot build here — what this
//! proves is that OUR code, verbatim, compiles against the API as the crates
//! declare it.

mod tauri;
mod souvlaki;

pub fn platform_name() -> &'static str {
    "linux"
}

// ---- the tray/island sinks, stubbed -----------------------------------------

pub mod tray {
    use crate::media::MediaSnapshot;
    use crate::tauri::AppHandle;
    pub fn sync_play_label(_app: &AppHandle, _snapshot: &MediaSnapshot) {}
}
pub mod island {
    use crate::media::MediaSnapshot;
    use crate::tauri::AppHandle;
    pub fn sync(_app: &AppHandle, _snapshot: &MediaSnapshot) {}
}

// ---- media.rs, verbatim but for the module paths ----------------------------

pub struct AppState {
    pub media: std::sync::Mutex<media::MediaSnapshot>,
}

mod commands {
    pub use super::AppState;
    pub use super::platform_name;
    pub use super::{MediaMetadataUpdate, MediaUpdate};
}

pub mod media {
    use crate::souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig};
    use crate::tauri::{AppHandle, Manager};

    use super::commands::{platform_name, AppState, MediaUpdate};

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

    /// macOS: no controls of our own — the webview's own Now Playing entry is
    /// the one the system shows.
    #[cfg(target_os = "macos")]
    pub fn attach(_app: &AppHandle) {}

    /// The OS pressed a button: turn it into the page's own action vocabulary.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn handle_event(app: &AppHandle, event: MediaControlEvent) {
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
                    crate::souvlaki::SeekDirection::Forward => "seekforward",
                    crate::souvlaki::SeekDirection::Backward => "seekbackward",
                },
                Some(duration.as_secs_f64()),
            ),
            MediaControlEvent::Seek(direction) => (
                match direction {
                    crate::souvlaki::SeekDirection::Forward => "seekforward",
                    crate::souvlaki::SeekDirection::Backward => "seekbackward",
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

        super::tray::sync_play_label(app, &snapshot);
        super::island::sync(app, &snapshot);
    }

    /// Whether the player is currently sounding, for the tray's label.
    pub fn is_playing(snapshot: &MediaSnapshot) -> bool {
        snapshot.playback_state == "playing"
    }

    #[allow(dead_code)]
    fn _platform_reference() {
        let _ = platform_name();
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaUpdate {
    pub kind: String,
    #[serde(default)]
    pub metadata: Option<MediaMetadataUpdate>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub position: f64,
    #[serde(default)]
    pub rate: f64,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MediaMetadataUpdate {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub artwork_url: Option<String>,
}

use serde::Deserialize;

// ---- chrome.rs, verbatim -----------------------------------------------------

#[cfg(not(target_os = "macos"))]
pub mod chrome {
    use crate::tauri::AppHandle;
    /// No traffic lights anywhere else: Windows and Linux build frameless, and
    /// the page's own bar already stands down in the player.
    pub fn set_traffic_lights_visible(_app: &AppHandle, _visible: bool) {}
}

fn main() {
    println!("media.rs compiles: Linux branch against the souvlaki-signature stub");
    println!("is_playing: {}", media::is_playing(&Default::default()));
}
