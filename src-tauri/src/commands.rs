//! The IPC surface the desktop shim talks to, and the shared app state.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_opener::OpenerExt as _;

use bitchord_core::types::{BridgeRequest, HttpRequest, HttpResponse};

use crate::{agent, chrome, island, media};

pub struct AppState {
    pub core: Arc<bitchord_core::Core>,
    pub media: Mutex<media::MediaSnapshot>,
    /// The agent window's per-open session baseline and report dedup.
    pub agent: agent::AgentState,
    pub data_dir: PathBuf,
}

impl AppState {
    pub fn new(core: Arc<bitchord_core::Core>, data_dir: PathBuf) -> AppState {
        AppState {
            agent: agent::AgentState::snapshot(&core),
            core,
            media: Mutex::new(media::MediaSnapshot::default()),
            data_dir,
        }
    }
}

#[tauri::command]
pub async fn bridge_call(request: serde_json::Value, app: AppHandle) -> Result<serde_json::Value, String> {
    let request: BridgeRequest = serde_json::from_value(request).map_err(|e| e.to_string())?;
    let method = request.method.clone();
    let params = request.params.clone();
    let core = app.state::<AppState>().core.clone();
    let response = core.call(request).await;

    // The shell owns what a browser did with tabs: sign-in links open the
    // agent window here, after the bridge answered the page. The Dynamic
    // Island's toggle is the same shape — the core wrote the preference, the
    // shell applies it (create/destroy the overlay window) before replying.
    if response.get("ok").and_then(|ok| ok.as_bool()).unwrap_or(false) {
        match method.as_str() {
            "spotify.signIn" => agent::open_signin(&app, agent::AgentKind::Spotify, None),
            "discord.connect" => agent::open_signin(&app, agent::AgentKind::Discord, None),
            "yt.setConnected" => {
                let connected = params.get("connected").and_then(|c| c.as_bool()).unwrap_or(false);
                if connected {
                    agent::open_signin(&app, agent::AgentKind::Yt, None);
                }
            }
            "island.setEnabled" => island::apply_enabled(&app),
            _ => {}
        }
    }
    Ok(response)
}

/// The page's own fetch, served the way the browser extension's
/// declarativeNetRequest rules served the site's: the request leaves the core
/// with YouTube's Origin/Referer/X-Origin (headers a page cannot set), and
/// CORS never applies in Rust. youtubei answers CORS with nothing at all, so
/// without this every browse and every media request dies with "Failed to
/// fetch". `fallback` serves any https host with no impersonation — the
/// desktop's answer for cross-origin servers that answer no CORS (Listen
/// Together; the site reached it same-origin), asked for only after the
/// page's own fetch failed as a network error.
#[tauri::command]
pub async fn page_fetch(request: HttpRequest, fallback: Option<bool>, app: AppHandle) -> Result<HttpResponse, String> {
    let core = app.state::<AppState>().core.clone();
    core.page_fetch(&request, fallback.unwrap_or(false)).await.map_err(|(message, _)| message)
}

#[tauri::command]
pub fn desktop_info() -> DesktopInfo {
    DesktopInfo {
        platform: platform_name().to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

#[derive(Serialize)]
pub struct DesktopInfo {
    pub platform: String,
    pub version: String,
}

pub fn platform_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// Where an outside link goes: the services we sign into open inside the app
/// (their cookies are the point), everything else opens in the system browser.
const SIGNIN_HOSTS: &[&str] = &[
    "accounts.google.com",
    "accounts.spotify.com",
    "open.spotify.com",
    "discord.com",
    "music.youtube.com",
    "www.youtube.com",
];

/// Async on purpose: on Windows, creating a webview window from a *synchronous*
/// command deadlocks the WebView2 event loop — the window opens white, the whole
/// app freezes and only Task Manager ends it (the Webview2 issue the Tauri docs
/// warn about under `WebviewWindowBuilder` · "Known issues"). Async commands run
/// off the main thread, which is the documented-safe pattern for `build()`.
#[tauri::command]
pub async fn open_external(url: String, app: AppHandle) -> Result<(), String> {
    let parsed = url::Url::parse(&url).map_err(|e| e.to_string())?;
    if let Some(host) = parsed.host_str() {
        if SIGNIN_HOSTS.iter().any(|known| host == *known || host.ends_with(&format!(".{known}"))) {
            let kind = match host {
                h if h.contains("spotify") => agent::AgentKind::Spotify,
                h if h.contains("discord") => agent::AgentKind::Discord,
                _ => agent::AgentKind::Yt,
            };
            agent::open_signin(&app, kind, Some(url));
            return Ok(());
        }
    }
    // Everything else leaves the app: the Android release page, a lyric
    // provider's site, the project's GitHub.
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// Async for the same reason as [open_external]: it builds a webview window.
#[tauri::command]
pub async fn open_signin(kind: String, app: AppHandle) -> Result<(), String> {
    let kind = match kind.as_str() {
        "spotify" => agent::AgentKind::Spotify,
        "discord" => agent::AgentKind::Discord,
        _ => agent::AgentKind::Yt,
    };
    agent::open_signin(&app, kind, None);
    Ok(())
}

#[tauri::command]
pub async fn agent_report(kind: String, payload: String, app: AppHandle) -> Result<(), String> {
    agent::handle_report(&app, &kind, &payload).await
}

#[derive(Deserialize)]
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

#[tauri::command]
pub fn media_update(update: MediaUpdate, app: AppHandle) -> Result<(), String> {
    media::apply_update(&app, update);
    Ok(())
}

/// The Dynamic Island's presses and window asks. Async like the other window
/// builders' users: resizing a window on Windows can re-enter the WebView2
/// event loop, and the off-main-thread command is where the shell already
/// keeps everything that touches window geometry.
#[tauri::command]
pub async fn island_action(action: String, arg: Option<f64>, app: AppHandle) -> Result<(), String> {
    island::handle_action(&app, &action, arg);
    Ok(())
}

/// The island asks once at load what the shell last saw, so a card born into
/// an already-unfocused app (settings toggled while the app is in the
/// background) knows it may show before the next Focused event arrives.
#[tauri::command]
pub fn island_focus_state() -> bool {
    island::main_focused()
}

/// The frameless main window's own top bar. Three buttons and a drag zone
/// replace the native decorations the shell no longer draws (lib.rs), so the
/// page's background shows through the top of the app on every page. Each
/// command is async for the same reason every other window-touching command
/// here is: the off-main-thread path is the documented-safe one on Windows.
#[tauri::command]
pub async fn window_minimize(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.minimize();
    }
    Ok(())
}

/// One button, both states: a maximized window restores, the rest maximizes.
/// Branching here keeps `is_maximized` out of the ACL entirely.
#[tauri::command]
pub async fn window_toggle_maximize(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_maximized().unwrap_or(false) {
            let _ = window.unmaximize();
        } else {
            let _ = window.maximize();
        }
    }
    Ok(())
}

/// The top bar's close is the native close: BitChord goes to the tray and the
/// music keeps playing (the CloseRequested handler in lib.rs hides, never
/// exits), so the button hides the window outright.
#[tauri::command]
pub async fn window_hide(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    Ok(())
}

/// Dragging the transparent top bar. The page presses this from a plain
/// mousedown on empty top-of-window space — the same gesture the native
/// frame's caption handled, decided in the page where the content lives.
#[tauri::command]
pub async fn window_start_drag(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.start_dragging();
    }
    Ok(())
}

/// The macOS traffic lights, from the page: the full-screen player owns the
/// whole window while it is up, and its own close button sits exactly where
/// the lights hang (TitleBarStyle::Overlay keeps them over the glass), so the
/// player's open state hides them and its close brings them back. chrome.rs
/// does the hiding; on Windows and Linux this is a no-op (the frameless
/// build has no lights, and WindowControls already stands down in the
/// player). Async for the same reason every window-touching command here is:
/// the off-main-thread path is the safe one, and the job hops to the main
/// thread inside.
#[tauri::command]
pub async fn set_traffic_lights_visible(visible: bool, app: AppHandle) -> Result<(), String> {
    chrome::set_traffic_lights_visible(&app, visible);
    Ok(())
}

#[tauri::command]
pub async fn island_resize(expanded: bool, app: AppHandle) -> Result<(), String> {
    island::apply_size(&app, expanded);
    Ok(())
}

#[tauri::command]
pub async fn island_overlay(hidden: bool, app: AppHandle) -> Result<(), String> {
    island::set_click_through(&app, hidden);
    Ok(())
}

#[tauri::command]
pub fn set_launch_at_login(on: bool, app: AppHandle) -> Result<(), String> {
    let autostart = app.autolaunch();
    let result = if on { autostart.enable() } else { autostart.disable() };
    result.map_err(|e| e.to_string())
}

/// The signed `yt.fetch` request may arrive with a base64 body; the core
/// re-decodes it. This helper exists for the docs' sake: the wire contract is
/// the extension's, byte for byte.
#[allow(dead_code)]
fn _wire_shape_note(_r: &HttpRequest) {}
