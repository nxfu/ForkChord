//! The agent window: the desktop replacement for "the browser tab the user
//! signs in on". A small window on the real service origin holds the user's
//! session; the injected agent shim reports its cookies (or Discord's token)
//! to the core, which does everything after that.
//!
//! Two things the live services do to embedded browsers are handled here:
//!
//! * Google refuses sign-ins from WebView/WKWebView ("This browser or app may
//!   not be secure"), so the window claims a plain Chrome user agent and the
//!   `chrome_spoof` script papers over the JS fingerprints Google checks.
//!   It is re-evaluated on every page load, because the sign-in flow hops
//!   origins (accounts.google.com → accounts.youtube.com → music.youtube.com)
//!   and each hop starts a fresh JS world.
//! * The window is built only from `async` commands or spawned tasks. Building
//!   it on the main thread (a sync command, an event handler) deadlocks the
//!   Windows event loop: white window, frozen app.
//!
//! And one thing JavaScript cannot do is handled by the capture thread: a
//! page's `document.cookie` never includes HttpOnly cookies, and YouTube's
//! session is carried by exactly those (`__Secure-1PSID`, `__Secure-3PSID`,
//! the rotating `PSIDTS` pair, `LOGIN_INFO`). A jar without them gets
//! "signed out" answers from `account/account_menu` however correct the
//! SAPISIDHASH signature is. So a background thread reads the webview's
//! *native* cookie store (`cookies_for_url`, HttpOnly included) for both
//! domains the session spans and reports through the same handler the shim
//! uses; the shim's `document.cookie` reports stay as the fallback for
//! engines without a native cookie API.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::commands::AppState;
use crate::{AGENT_SHIM, CHROME_SPOOF};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AgentKind {
    Yt,
    Spotify,
    Discord,
}

impl AgentKind {
    fn default_url(self) -> &'static str {
        match self {
            AgentKind::Yt => "https://music.youtube.com/",
            // The same continue target the extension's spotify.signIn used.
            AgentKind::Spotify => "https://accounts.spotify.com/login?continue=https%3A%2F%2Fopen.spotify.com%2F",
            AgentKind::Discord => "https://discord.com/app",
        }
    }

    fn title(self) -> &'static str {
        match self {
            AgentKind::Yt => "Sign in to YouTube Music",
            AgentKind::Spotify => "Sign in to Spotify",
            AgentKind::Discord => "Sign in to Discord",
        }
    }
}

/// The origins whose cookie store is harvested for each service, with the
/// domain an engine that does not spell one out should assume. YouTube's
/// session lives on two domains at once — SID / HSID / SSID / SAPISID on
/// `.google.com`, the PSID session cookies on `.youtube.com` — and both
/// halves are needed for YouTube to accept the session.
const YT_CAPTURE_ORIGINS: &[(&str, &str)] = &[
    ("https://music.youtube.com/", ".youtube.com"),
    ("https://accounts.google.com/", ".google.com"),
];

/// Spotify keeps its session on one domain; both login and player hosts are
/// polled so the report works no matter which page the flow ended on.
const SPOTIFY_CAPTURE_ORIGINS: &[(&str, &str)] = &[
    ("https://open.spotify.com/", ".spotify.com"),
    ("https://accounts.spotify.com/", ".spotify.com"),
];

/// A plain Chrome user agent. The embedded engines (WebView2, WKWebView,
/// WebKitGTK) are identified as such by the live services, and Google answers
/// their sign-in pages with "This browser or app may not be secure"; a normal
/// Chrome string is all it takes to be served the real page. Same version the
/// Rust core's HTTP client claims, so requests from both sides read alike.
fn chrome_user_agent() -> String {
    let platform = if cfg!(target_os = "windows") {
        "Windows NT 10.0; Win64; x64"
    } else if cfg!(target_os = "macos") {
        "Macintosh; Intel Mac OS X 10_15_7"
    } else {
        "X11; Linux x86_64"
    };
    format!(
        "Mozilla/5.0 ({platform}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36"
    )
}

fn agent_script() -> &'static str {
    AGENT_SHIM
}

/// The agent window's per-open state: the session values the window started
/// with, and the last YouTube report seen. A report that merely re-states
/// the session the window was opened with must not close it — a user who
/// reopens sign-in to switch accounts would get the window yanked away
/// before they could act. Only a *changed* session closes the window, and
/// only once the core verified it.
pub struct AgentState {
    /// Fingerprint of the last accepted yt cookie report.
    pub yt_seen: Mutex<Option<String>>,
    /// Fingerprint of the yt session at window-open time.
    pub yt_at_open: Mutex<String>,
    /// sp_dc at window-open time.
    pub spotify_at_open: Mutex<String>,
    /// Discord token at window-open time.
    pub discord_at_open: Mutex<String>,
}

impl AgentState {
    pub fn snapshot(core: &bitchord_core::Core) -> AgentState {
        AgentState {
            yt_seen: Mutex::new(None),
            yt_at_open: Mutex::new(fingerprint(&core.yt.cookie_dump())),
            spotify_at_open: Mutex::new(core.store.read(|d| d.sp_dc.clone()).unwrap_or_default()),
            discord_at_open: Mutex::new(
                core.store
                    .read(|d| d.discord_account.as_ref().map(|a| a.token.clone()).unwrap_or_default()),
            ),
        }
    }
}

/// A comparable summary of a cookie dump: the sorted `name=value` pairs,
/// attributes stripped. Two dumps of the same session produce the same
/// fingerprint whether they came from `document.cookie` or the native cookie
/// store, which is what the dedup and the close guards compare.
pub fn fingerprint(dump: &str) -> String {
    let mut pairs: Vec<String> = dump
        .split(['\n', ';'])
        .filter_map(|segment| {
            let segment = segment.trim();
            let (name, value) = segment.split_once('=')?;
            let name = name.trim();
            if name.is_empty() || is_cookie_attribute(name) {
                return None;
            }
            let value = value.trim();
            if value.is_empty() {
                None
            } else {
                Some(format!("{name}={value}"))
            }
        })
        .collect();
    pairs.sort();
    pairs.dedup();
    pairs.join("\n")
}

/// The Set-Cookie attribute names, mirroring the core's parser: a segment
/// keyed by one of these belongs to the cookie before it.
fn is_cookie_attribute(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "domain" | "path" | "expires" | "max-age" | "samesite" | "secure" | "httponly" | "partitioned" | "priority"
    )
}

/// The webview cookie store's cookies for [origins], as one full
/// `name=value; Domain=…; Path=…; Secure; HttpOnly` line per cookie — the
/// shape the core's jar parses. Includes the HttpOnly session cookies
/// `document.cookie` never sees. `None` when the store has nothing for any
/// origin (no session yet) or the read failed.
///
/// Platform split, and it matters: wry's `cookies_for_url` is only correct
/// where the engine itself does the matching (WebView2 asks its cookie
/// manager; WebKitGTK asks libsoup). On macOS wry fetches every cookie and
/// then compares `cookie.domain() == url.domain()` as *strings* — but
/// `NSHTTPCookie.domain` carries the `Set-Cookie` attribute verbatim
/// (`.youtube.com`), which the cookie crate surfaces without the leading
/// dot, while the URL's host is `music.youtube.com`. A host never equals its
/// parent domain, so on WKWebView that filter drops every cookie a site set
/// with a `Domain=` attribute — which is YouTube's whole session: SID, the
/// `__Secure-1PSID`/`__Secure-3PSID` pair, the rotating `PSIDTS` pair,
/// `LOGIN_INFO`, SAPISID. The capture came back empty, the only reports left
/// were the shim's `document.cookie` (no HttpOnly cookies — a jar shaped like
/// that gets "signed out" answers however correct the SAPISIDHASH is), and a
/// completed sign-in never verified: the window stayed open and the app stayed
/// signed out, on macOS alone. So there the whole store is read with
/// [tauri::WebviewWindow::cookies] and matched with [domain_matches], the
/// rules a browser applies.
fn native_dump(window: &tauri::WebviewWindow, origins: &[(&str, &str)]) -> Option<String> {
    // One read of the full store serves every origin on the macOS path.
    #[cfg(target_os = "macos")]
    let store = window.cookies().ok();

    let mut lines = Vec::new();
    for (origin, fallback_domain) in origins {
        let Ok(url) = origin.parse::<tauri::Url>() else { continue };
        #[cfg(target_os = "macos")]
        let host = url.host_str().unwrap_or_default().trim().to_ascii_lowercase();

        #[cfg(target_os = "macos")]
        let cookies = match store.as_ref() {
            Some(all) => all
                .iter()
                // The same rule wry applies where its filter works: a secure
                // cookie rides https only, and these origins are https.
                .filter(|cookie| {
                    let secure = cookie.secure().unwrap_or(false);
                    (secure && url.scheme() == "https") || !secure
                })
                .filter(|cookie| {
                    let domain = cookie.domain().unwrap_or(fallback_domain);
                    domain_matches(domain, &host)
                })
                .cloned()
                .collect::<Vec<_>>(),
            None => Vec::new(),
        };
        #[cfg(not(target_os = "macos"))]
        let cookies = match window.cookies_for_url(url.clone()) {
            Ok(cookies) => cookies,
            Err(_) => continue,
        };

        for cookie in &cookies {
            let name = cookie.name().trim();
            let value = cookie.value().trim();
            if name.is_empty() || value.is_empty() {
                continue;
            }
            let domain = cookie.domain().unwrap_or(fallback_domain);
            let path = cookie.path().unwrap_or("/");
            let mut line = format!("{name}={value}; Domain={domain}; Path={path}");
            if cookie.secure().unwrap_or(false) {
                line.push_str("; Secure");
            }
            if cookie.http_only().unwrap_or(false) {
                line.push_str("; HttpOnly");
            }
            lines.push(line);
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// RFC 6265 §5.1.4 domain-match, the way a browser decides which cookies a
/// host receives: an exact match, or the host ending in the cookie's domain
/// at a dot — `music.youtube.com` matches `.youtube.com` and itself, never
/// `notyoutube.com`. This is the matching `cookies_for_url` stands in for on
/// the engines that do it natively (and gets wrong on macOS, see
/// [native_dump]).
#[cfg(target_os = "macos")]
fn domain_matches(cookie_domain: &str, host: &str) -> bool {
    let domain = cookie_domain.trim().trim_start_matches('.').trim_end_matches('.').to_ascii_lowercase();
    let host = host.trim().to_ascii_lowercase();
    if domain.is_empty() || host.is_empty() {
        return false;
    }
    if host == domain {
        return true;
    }
    host.len() > domain.len()
        && host.ends_with(&domain)
        && host.as_bytes()[host.len() - domain.len() - 1] == b'.'
}

/// One background thread polls the agent window's native cookie store for as
/// long as the app runs — cheap when the window is absent, and the only
/// source of the HttpOnly half of YouTube's session. It never runs on the
/// main thread: like every cross-webview dispatch on Windows, that deadlocks
/// the event loop. Reports go through the same [handle_report] the shim's
/// IPC uses, so dedup and the close guards see one stream.
static CAPTURE_THREAD: AtomicBool = AtomicBool::new(false);

pub fn start_capture(app: &AppHandle) {
    if CAPTURE_THREAD.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("agent-cookie-capture".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(1500));
            let Some(window) = app.get_webview_window("agent") else {
                continue;
            };
            if let Some(dump) = native_dump(&window, YT_CAPTURE_ORIGINS) {
                let _ = tauri::async_runtime::block_on(handle_report(&app, "yt", &dump));
            }
            if let Some(dump) = native_dump(&window, SPOTIFY_CAPTURE_ORIGINS) {
                let _ = tauri::async_runtime::block_on(handle_report(&app, "spotify", &dump));
            }
        });
    if spawned.is_err() {
        CAPTURE_THREAD.store(false, Ordering::SeqCst);
    }
}

/// Opens the sign-in window for [kind] at [url] (or the service's own entry
/// page). An existing agent window of the same kind is navigated instead of
/// rebuilt — the shim derives the service from the origin, so the window can
/// move between Google, Spotify and Discord targets freely.
pub fn open_signin(app: &AppHandle, kind: AgentKind, url: Option<String>) {
    let target = url.unwrap_or_else(|| kind.default_url().to_string());
    let parsed: tauri::Url = match target.parse() {
        Ok(url) => url,
        Err(_) => return,
    };

    // This open is the new baseline: only a session different from the one
    // present *now* closes the window later (AgentState's `*_at_open`).
    if let Some(state) = app.try_state::<AppState>() {
        let core = &state.core;
        *state.agent.yt_at_open.lock().unwrap() = fingerprint(&core.yt.cookie_dump());
        *state.agent.spotify_at_open.lock().unwrap() =
            core.store.read(|d| d.sp_dc.clone()).unwrap_or_default();
        *state.agent.discord_at_open.lock().unwrap() = core
            .store
            .read(|d| d.discord_account.as_ref().map(|a| a.token.clone()).unwrap_or_default());
        *state.agent.yt_seen.lock().unwrap() = None;
    }

    // Reuse the window when there is one: building a second window with the
    // same label fails, and closing + rebuilding races the close.
    if let Some(existing) = app.get_webview_window("agent") {
        let navigated = existing.navigate(parsed.clone()).is_ok();
        if navigated {
            let _ = existing.set_title(kind.title());
            let _ = existing.show();
            let _ = existing.unminimize();
            let _ = existing.set_focus();
            start_capture(app);
            return;
        }
        let _ = existing.close();
    }

    let _ = WebviewWindowBuilder::new(app, "agent", WebviewUrl::External(parsed))
        .title(kind.title())
        .inner_size(460.0, 680.0)
        .min_inner_size(360.0, 480.0)
        .user_agent(&chrome_user_agent())
        .initialization_script(agent_script())
        .initialization_script(CHROME_SPOOF)
        // The shim must exist in every document the flow lands on; some
        // engines only honor initialization scripts for the first navigation,
        // so re-eval both on every load (they are idempotent).
        .on_page_load(|window, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Started {
                let _ = window.eval(agent_script());
                let _ = window.eval(CHROME_SPOOF);
            }
        })
        .build();
    start_capture(app);
}

/// What a report carries: a cookie dump (native lines or `document.cookie`),
/// or Discord's token.
pub async fn handle_report(app: &AppHandle, kind: &str, payload: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    match kind {
        "yt" => {
            if payload.contains("SAPISID") {
                let mark = fingerprint(payload);
                {
                    let mut seen = state.agent.yt_seen.lock().unwrap();
                    if seen.as_deref() == Some(mark.as_str()) {
                        // The same session, reported again by the next poll —
                        // nothing new for the core to absorb or verify.
                        return Ok(());
                    }
                    *seen = Some(mark.clone());
                }
                state.core.store.with(|d| d.yt_cookie_dump = Some(payload.to_string()));
                state.core.store.save();
                state.core.yt.set_cookies(payload);
                state.core.yt.invalidate_accounts().await;
                let fresh = state.core.yt.auth_state(true).await;
                // The jar has absorbed every Set-Cookie from the read; persist
                // it so the session survives a restart without a fresh report.
                let dump = state.core.yt.cookie_dump();
                if !dump.is_empty() {
                    state.core.store.with(|d| d.yt_cookie_dump = Some(dump));
                    state.core.store.save();
                }
                let _ = state.core.sink().send(bitchord_core::types::BridgeEvent::YtAuthState(fresh.clone()));
                // The window's job is done once the core accepted a session
                // the window did not already hold: the user signed in or
                // switched accounts. A "signed out" answer (mid-flow, or the
                // report raced YouTube's own session setup) leaves the window
                // up for the next poll.
                let unchanged = *state.agent.yt_at_open.lock().unwrap() == mark;
                if fresh.signed_in && !unchanged {
                    if let Some(window) = app.get_webview_window("agent") {
                        let _ = window.close();
                    }
                }
            }
        }
        "spotify" => {
            if let Some(sp_dc) = cookie_value(payload, "sp_dc") {
                // Same guard as yt: a report of the session the window was
                // opened with is not a sign-in, and must not close it.
                let changed = *state.agent.spotify_at_open.lock().unwrap() != sp_dc
                    && state.core.store.read(|d| d.sp_dc.clone()).as_deref() != Some(sp_dc.as_str());
                state.core.store.with(|d| d.sp_dc = Some(sp_dc));
                state.core.store.save();
                if changed {
                    state.core.spotify.invalidate_session();
                    let fresh = state.core.spotify.auth_state().await;
                    let _ = state.core.sink().send(bitchord_core::types::BridgeEvent::SpotifyAuthState(fresh));
                    if let Some(window) = app.get_webview_window("agent") {
                        let _ = window.close();
                    }
                }
            }
        }
        "discord" => {
            let token = payload.trim().to_string();
            if !token.is_empty() && *state.agent.discord_at_open.lock().unwrap() != token {
                match state.core.discord.user_info(&token).await {
                    Ok(user) => {
                        state.core.store.with(|d| {
                            d.discord_account = Some(bitchord_core::store::DiscordAccount { token, user });
                            d.discord_enabled = true;
                        });
                        state.core.store.save();
                        let _ = state.core.sink().send(bitchord_core::types::BridgeEvent::DiscordState(state.core.discord.state().await));
                        if let Some(window) = app.get_webview_window("agent") {
                            let _ = window.close();
                        }
                    }
                    Err(problem) => {
                        log::warn!("discord connect failed: {problem}");
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// The value of [name] in a cookie dump, if present — both dump shapes work,
/// since attribute segments never carry a cookie's value.
fn cookie_value(dump: &str, name: &str) -> Option<String> {
    for segment in dump.split(['\n', ';']) {
        let Some((key, value)) = segment.trim().split_once('=') else { continue };
        if key.trim().eq_ignore_ascii_case(name) && !value.trim().is_empty() {
            return Some(value.trim().to_string());
        }
    }
    None
}
