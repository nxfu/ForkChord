//! Scratch harness for the agent.rs close-guard logic (round v1.9.7-mac).
//! The pure functions are copied verbatim from src-tauri/src/agent.rs; the
//! scenarios reproduce the macOS instant close and prove the fix against it.

use std::collections::HashMap;

// ---- verbatim from agent.rs -------------------------------------------------

fn is_cookie_attribute(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "domain" | "path" | "expires" | "max-age" | "samesite" | "secure" | "httponly" | "partitioned" | "priority"
    )
}

const YT_IDENTITY_COOKIES: &[&str] = &["SAPISID", "__Secure-1PSID", "__Secure-3PSID", "SID"];

fn identity_cookies(dump: &str) -> HashMap<String, String> {
    let mut pairs = HashMap::new();
    for segment in dump.split(['\n', ';']) {
        let Some((name, value)) = segment.trim().split_once('=') else { continue };
        let name = name.trim();
        let value = value.trim();
        if name.is_empty() || value.is_empty() || is_cookie_attribute(name) {
            continue;
        }
        if YT_IDENTITY_COOKIES.contains(&name) {
            pairs.insert(name.to_string(), value.to_string());
        }
    }
    pairs
}

fn identity_unchanged(at_open: &HashMap<String, String>, report: &str) -> bool {
    let pairs = identity_cookies(report);
    pairs.iter().all(|(name, value)| at_open.get(name).is_some_and(|held| held == value))
}

// The old guard, for contrast: a fingerprint comparison against the core jar.
fn fingerprint(dump: &str) -> String {
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

// ---- scenarios ---------------------------------------------------------------

/// The window's webview store at open time: a full YouTube session (the
/// persistent WKWebView store of a user who signed in once before).
const NATIVE_AT_OPEN: &str = "\
SAPISID=ABC123_x; Domain=.google.com; Path=/; Secure; HttpOnly\n\
__Secure-1PSID=SID11g; Domain=.google.com; Path=/; Secure; HttpOnly\n\
__Secure-3PSID=SID33g; Domain=.google.com; Path=/; Secure; HttpOnly\n\
SID=sidplain; Domain=.google.com; Path=/; Secure; HttpOnly\n\
__Secure-1PSIDTS=t-old; Domain=.youtube.com; Path=/; Secure; HttpOnly";

/// The core jar for the same session, as it looks after the app's own API
/// calls rotated the PSIDTS pair — the shape the OLD baseline compared
/// against (a fingerprint that never matches the webview's dump).
const CORE_JAR: &str = "SAPISID=ABC123_x; Domain=.google.com\n__Secure-1PSID=SID11g; Domain=.google.com\n__Secure-3PSID=SID33g; Domain=.google.com\nSID=sidplain; Domain=.google.com\n__Secure-1PSIDTS=t-rotated; Domain=.youtube.com";

fn main() {
    let baseline = identity_cookies(NATIVE_AT_OPEN);

    // 1. The shim's document.cookie report seconds after open (same session,
    //    HttpOnly cookies invisible to JS — a SUBSET of the baseline).
    let shim_report = "SAPISID=ABC123_x; PATH=/; DOMAIN=.google.com";
    assert!(identity_unchanged(&baseline, shim_report), "shim subset report must be unchanged");
    println!("ok 1: shim subset report -> unchanged (window stays up)");

    // 2. The native capture's full dump 1.5 s after open (identical store).
    assert!(identity_unchanged(&baseline, NATIVE_AT_OPEN), "native dump of the same session must be unchanged");
    println!("ok 2: native dump of the same session -> unchanged");

    // 3. The PSIDTS pair rotated in the webview store (Set-Cookie on a page
    //    load): churn, not a sign-in — must NOT close the window.
    let rotated = NATIVE_AT_OPEN.replace("t-old", "t-new");
    assert!(identity_unchanged(&baseline, &rotated), "PSIDTS rotation must be unchanged");
    println!("ok 3: PSIDTS rotation -> unchanged");

    // 4. The OLD guard on the same inputs: the core jar's fingerprint against
    //    the native dump — the comparison that fired the instant close.
    let old_fired = fingerprint(CORE_JAR) != fingerprint(NATIVE_AT_OPEN);
    assert!(old_fired, "the old guard did fire (that was the bug)");
    println!("ok 4: old fingerprint guard fired on the same session (the bug, reproduced)");

    // 5. A different account signs in: SAPISID changes -> changed -> close.
    let switched = NATIVE_AT_OPEN.replace("ABC123_x", "NEWACCOUNT99").replace("SID11g", "NEW1PSID");
    assert!(!identity_unchanged(&baseline, &switched), "an account switch must be changed");
    println!("ok 5: account switch -> changed (closes after verify)");

    // 6. A first-ever sign-in: baseline empty (signed out at open) -> changed.
    let empty = HashMap::new();
    assert!(!identity_unchanged(&empty, NATIVE_AT_OPEN), "an empty baseline must be changed");
    println!("ok 6: first sign-in on an empty baseline -> changed (closes after verify)");

    // 7. Spotify baseline through the same helpers: sp_dc compare.
    let sp_open = "sp_dc=ACC-old; Domain=.spotify.com; Path=/; Secure; HttpOnly";
    let sp_report_same = "sp_dc=ACC-old; Domain=.spotify.com; Path=/";
    let sp_report_new = "sp_dc=ACC-new; Domain=.spotify.com; Path=/";
    assert_eq!(cookie_value(sp_open, "sp_dc"), Some(cookie_value(sp_report_same, "sp_dc").unwrap()));
    assert_ne!(cookie_value(sp_open, "sp_dc").as_deref(), cookie_value(sp_report_new, "sp_dc").as_deref());
    println!("ok 7: spotify baseline compare holds");

    // 8. on_service_page gating: the capture only harvests the service whose
    //    page the window is on.
    let yt_page: Result<url::Url, url::ParseError> = url::Url::parse("https://accounts.google.com/o/oauth2/auth?x");
    let discord_page: Result<url::Url, url::ParseError> = url::Url::parse("https://discord.com/app");
    let yt_hosts = ["youtube.com", "google.com"];
    let spotify_hosts = ["spotify.com"];
    assert!(on_service_page(&yt_page, &yt_hosts));
    assert!(!on_service_page(&yt_page, &spotify_hosts));
    assert!(!on_service_page(&discord_page, &yt_hosts), "a discord page must never harvest yt — this was the instant close");
    assert!(!on_service_page(&discord_page, &spotify_hosts));
    let music_page: Result<url::Url, url::ParseError> = url::Url::parse("https://music.youtube.com/");
    assert!(on_service_page(&music_page, &yt_hosts));
    println!("ok 8: page gating keeps foreign sessions out of the report stream");
}

fn cookie_value(dump: &str, name: &str) -> Option<String> {
    for segment in dump.split(['\n', ';']) {
        let Some((key, value)) = segment.trim().split_once('=') else { continue };
        if key.trim().eq_ignore_ascii_case(name) && !value.trim().is_empty() {
            return Some(value.trim().to_string());
        }
    }
    None
}

fn on_service_page(url: &Result<url::Url, url::ParseError>, suffixes: &[&str]) -> bool {
    let Ok(url) = url.as_ref() else { return false };
    let Some(host) = url.host_str() else { return false };
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    suffixes
        .iter()
        .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
}
