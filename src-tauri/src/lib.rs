//! BitChord for the desktop: a Tauri 2 shell around the exact same web
//! frontend, with a Rust core (`bitchord-core`) doing everything the browser
//! extension did — plus the native parts a browser can't: the system tray,
//! MPRIS / Now Playing media controls, sign-in agent windows, deep links,
//! single instance and launch-at-login.

mod agent;
mod chrome;
mod commands;
mod island;
mod media;
mod tray;

use std::sync::Arc;

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

pub use commands::AppState;

const DESKTOP_SHIM: &str = include_str!("../shim/desktop.js");
const AGENT_SHIM: &str = include_str!("../shim/agent.js");
const CHROME_SPOOF: &str = include_str!("../shim/chrome_spoof.js");

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // A second launch focuses the running window, like any music player.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        // The agent windows and the dynamic island are excluded:
        // state-tracking them is what once showed a hidden window on first
        // launch — the plugin's restore starts with `should_show = true` for
        // any window it has no saved state for. The island doubly so: its
        // place is computed (top centre), never restored — a saved geometry
        // would drag the overlay wherever it last stood. Visibility itself is
        // not tracked either: the plugin saves on exit, and a session that
        // ended from the tray (main window hidden) must not come back as a
        // saved `visible: false` — the next launch would restore a window
        // that never shows, leaving only the tray icon.
        //
        // DECORATIONS is excluded for the same class of reason: the plugin
        // restores on `on_window_ready`, *after* the window was built
        // frameless below, and the state file an older, decorated build
        // wrote says `decorations: true` — so an upgrade would redraw the
        // native title bar over the frameless window (double caption
        // buttons, a webview mis-sized for the caption it never planned
        // for). The main window is frameless by design; that is not a
        // per-session preference to remember.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_denylist(&["agent", "island"])
                .with_state_flags(tauri_plugin_window_state::StateFlags::all()
                    & !tauri_plugin_window_state::StateFlags::VISIBLE
                    & !tauri_plugin_window_state::StateFlags::DECORATIONS)
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .invoke_handler(tauri::generate_handler![
            commands::bridge_call,
            commands::page_fetch,
            commands::desktop_info,
            commands::open_external,
            commands::open_signin,
            commands::agent_report,
            commands::media_update,
            commands::set_launch_at_login,
            commands::island_action,
            commands::island_focus_state,
            commands::island_resize,
            commands::island_overlay,
            commands::window_minimize,
            commands::window_toggle_maximize,
            commands::window_hide,
            commands::window_start_drag,
            commands::set_traffic_lights_visible,
        ])
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("bitchord"));
            let (core, events) = bitchord_core::Core::new(&data_dir);
            let core = Arc::new(core);
            app.manage(AppState::new(core, data_dir));

            // The main window, with the desktop shim injected before the app's
            // own scripts: the web frontend never learns it isn't in a browser.
            // Windows and Linux ship frameless: the page's own transparent top
            // bar (shell/WindowControls.tsx) draws the drag zone and the three
            // window buttons over whatever the app is showing, so the bar
            // merges with the page the way the sidebar and the player pill
            // already do — and a tinted artist or album page colours the top
            // of the window like it colours the rest. macOS keeps the native
            // traffic lights over the glass (TitleBarStyle::Overlay).
            #[allow(unused_mut)]
            let mut builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("/".into()))
                .title("BitChord")
                .inner_size(1180.0, 760.0)
                .min_inner_size(940.0, 600.0)
                .initialization_script(DESKTOP_SHIM);
            // The red mark, set on the window itself: the taskbar icon comes
            // from here at runtime, not from what the exe's resources or
            // Windows' icon cache happen to hold.
            if let Ok(icon) = tauri::image::Image::from_bytes(include_bytes!("../icons/128x128.png")) {
                builder = builder.icon(icon).expect("the window icon is valid");
            }
            // The one argument set every WebView2 here is built with: wry's
            // defaults plus the two flags that stand the engine's own media
            // player down (media.rs — without them the volume flyout shows
            // the webview's card beside souvlaki's, two players, one song).
            // The island and the agent windows repeat it verbatim: WebView2
            // only shares one browser process between webviews whose
            // environments agree, and a second process for a differing arg
            // set is memory spent on nothing.
            #[cfg(target_os = "windows")]
            {
                builder = builder.additional_browser_args(crate::media::WEBVIEW_BROWSER_ARGS);
            }
            #[cfg(target_os = "macos")]
            {
                // Traffic lights over the glass sidebar, Apple Music style.
                builder = builder.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true);
            }
            #[cfg(not(target_os = "macos"))]
            {
                // The native frame (its title, its icon and its caption
                // buttons) is gone; the page supplies the bar.
                builder = builder.decorations(false);
            }
            // The shell's own file-drop handler stands down: while it is
            // armed, WebView2 hands every drag to it and the page's HTML5
            // drag events (dragstart/dragover/drop) never fire — which left
            // the Up Next list's drag-to-reorder dead on Windows. Nothing in
            // the app drops files onto the window, so the handler only ever
            // cost the queue its rows.
            builder = builder.disable_drag_drop_handler();
            // macOS never reads the binding (its decorations call below is
            // cfg'd out — the Overlay title bar IS the design), so the allow
            // keeps that build's warning quiet.
            #[cfg_attr(target_os = "macos", allow(unused_variables))]
            let main = builder.build()?;
            // Belt and braces for the frame above: the plugin's restore hook
            // ran inside build(), so any state file it read has already had
            // its say — this re-asserts the frameless design after it. A
            // decorated main window here would mean two caption rows (the
            // page draws its own) and a webview sized for a frameless
            // window, so it is worth pinning in code, not just in flags.
            #[cfg(not(target_os = "macos"))]
            main.set_decorations(false)?;

            tray::build(app.handle())?;

            // The Dynamic Island, when the store says it is on (the default).
            // Built after the tray so its toggle reads a settled store.
            island::init(app.handle());

            // Pump the core's BridgeEvent stream (auth states, Discord status)
            // into the page, the way the extension broadcast them.
            let pump_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut events = events;
                while let Some(event) = events.recv().await {
                    if let Ok(text) = serde_json::to_string(&event) {
                        media::eval_in_main(
                            &pump_handle,
                            &format!(
                                "window.__BitChordBridgeEvent({})",
                                serde_json::to_string(&text).unwrap_or_else(|_| "\"\"".into())
                            ),
                        );
                    }
                }
            });

            // The desktop player reports state through the shim; mirror it
            // into the platform players the webview can't own: MPRIS on
            // Linux, SMTC on Windows (the volume flyout, the lock screen and
            // the hardware keys — souvlaki's registration is the only one on
            // Windows, the webview's own having been stood down at birth by
            // the browser args above; see media.rs). macOS registers
            // nothing: WKWebView puts the playing element on Now Playing by
            // itself, and a registration of our own used to stand beside it
            // as a second player (media.rs::attach).
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            media::attach(app.handle());

            // bitchord://party/<CODE> invites open the Listen Together page.
            use tauri_plugin_deep_link::DeepLinkExt;
            let deep_handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in &event.urls() {
                    if let Some(code) = invite_code(&url.to_string()) {
                        if let Some(window) = deep_handle.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                            let _ = window.eval(&format!("window.location.replace('/invite/{code}')"));
                        }
                    }
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            // The island mirrors the main window's focus: it shows while the
            // user is elsewhere and folds away while BitChord is frontmost
            // (island.rs::notify_main_focus). Hiding the window to the tray
            // fires the same event, so a closed-to-tray session keeps the
            // card up while the music plays.
            if let tauri::WindowEvent::Focused(focused) = event {
                if window.label() == "main" {
                    island::notify_main_focus(window.app_handle(), *focused);
                }
            }
            // Closing the window keeps the music: BitChord goes to the tray,
            // like every desktop player. Quit lives in the tray menu. This is
            // unconditional — the flag it once read could only ever have been
            // written false by an old build's tray-quit, and a stored false
            // made every later close end the whole app with no way back.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running BitChord");
}

/// `bitchord://party/<CODE>`, `bitchord://party?code=<CODE>` and the web
/// invite route (`/invite/<CODE>`) all resolve to the six-character code.
fn invite_code(value: &str) -> Option<String> {
    let regex = regex::Regex::new(r"(?:party/|invite/|\bcode=)([A-Za-z0-9]{6})(?:[?#/]|$)").unwrap();
    let caps = regex.captures(value)?;
    let code = caps.get(1)?.as_str().to_uppercase();
    if code.len() == 6 && code.chars().all(|c| c.is_ascii_alphanumeric()) {
        Some(code)
    } else {
        None
    }
}
