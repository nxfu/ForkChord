//! Standalone harness for the macOS fix in src-tauri/src/agent.rs.
//!
//! `native_dump`'s macOS branch and `domain_matches` are copied verbatim; the
//! tauri window and cookie::Cookie APIs are stubbed with the exact signatures
//! from tauri 2.12.5 (`cookies() -> Result<Vec<Cookie<'static>>>`) and the
//! cookie crate 0.18 (`domain()` strips one leading dot, the way
//! `CookieStr::to_str` + `strip_prefix` does, as wry's converted cookies
//! surface). A simulated WKWebView cookie store — the cookies Google and
//! YouTube actually set during a sign-in, domain cookies included — goes in,
//! and the dump the core's `set_cookies` parses comes out.

// ---- stub of the parts of cookie::Cookie wry's conversion produces ----------

#[derive(Clone, Debug)]
struct Cookie {
    name: String,
    value: String,
    /// As stored: domain cookies keep the leading dot, host-only do not.
    domain_raw: String,
    path: String,
    secure: bool,
    http_only: bool,
}

impl Cookie {
    fn name(&self) -> &str {
        &self.name
    }
    fn value(&self) -> &str {
        &self.value
    }
    /// cookie 0.18 strips one leading dot (CookieStr::to_str + strip_prefix).
    fn domain(&self) -> Option<&str> {
        Some(self.domain_raw.strip_prefix('.').unwrap_or(&self.domain_raw))
    }
    fn path(&self) -> Option<&str> {
        Some(&self.path)
    }
    fn secure(&self) -> Option<bool> {
        Some(self.secure)
    }
    fn http_only(&self) -> Option<bool> {
        Some(self.http_only)
    }
}

// ---- stub of tauri::WebviewWindow's cookie surface ---------------------------

struct FakeWindow {
    store: Vec<Cookie>,
    fail: bool,
}

impl FakeWindow {
    // tauri 2.12.5 signature: pub fn cookies(&self) -> Result<Vec<Cookie<'static>>>
    fn cookies(&self) -> Result<Vec<Cookie>, ()> {
        if self.fail {
            Err(())
        } else {
            Ok(self.store.clone())
        }
    }
    // The pre-fix call, for the contrast row.
    fn cookies_for_url(&self, host: &str) -> Result<Vec<Cookie>, ()> {
        Ok(self
            .store
            .iter()
            .filter(|c| c.domain().unwrap_or_default() == host)
            .cloned()
            .collect())
    }
}

// ---- the fix, verbatim from src-tauri/src/agent.rs --------------------------
// (only the window/Url calls are routed to the stubs)

const YT_CAPTURE_ORIGINS: &[(&str, &str)] = &[
    ("https://music.youtube.com/", ".youtube.com"),
    ("https://accounts.google.com/", ".google.com"),
];

fn native_dump(window: &FakeWindow, origins: &[(&str, &str)]) -> Option<String> {
    let store = window.cookies().ok();

    let mut lines = Vec::new();
    for (origin, fallback_domain) in origins {
        let host = origin
            .split("//")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();

        let cookies: Vec<Cookie> = match store.as_ref() {
            Some(all) => all
                .iter()
                .filter(|cookie| {
                    let secure = cookie.secure().unwrap_or(false);
                    (secure && origin.starts_with("https://")) || !secure
                })
                .filter(|cookie| {
                    let domain = cookie.domain().unwrap_or(fallback_domain);
                    domain_matches(domain, &host)
                })
                .cloned()
                .collect(),
            None => Vec::new(),
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

fn domain_matches(cookie_domain: &str, host: &str) -> bool {
    let domain = cookie_domain
        .trim()
        .trim_start_matches('.')
        .trim_end_matches('.')
        .to_ascii_lowercase();
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

// ---- the simulated session, then the checks ----------------------------------

fn session_cookie(name: &str, value: &str, domain: &str, http_only: bool) -> Cookie {
    Cookie {
        name: name.into(),
        value: value.into(),
        domain_raw: domain.into(),
        path: "/".into(),
        secure: true,
        http_only,
    }
}

fn main() {
    // What the WKWebView store holds after a completed Google sign-in that
    // ended on music.youtube.com (names/domains as YouTube sets them).
    let window = FakeWindow {
        fail: false,
        store: vec![
            session_cookie("SID", "sid-value", ".google.com", true),
            session_cookie("HSID", "hsid-value", ".google.com", true),
            session_cookie("SSID", "ssid-value", ".google.com", true),
            session_cookie("APISID", "apisid-value", ".google.com", false),
            session_cookie("SAPISID", "sapisid-value", ".google.com", false),
            session_cookie("__Secure-1PAPISID", "papisid1", ".google.com", false),
            session_cookie("__Secure-3PAPISID", "papisid3", ".google.com", false),
            session_cookie("LOGIN_INFO", "logininfo", ".youtube.com", false),
            session_cookie("__Secure-1PSID", "psid1", ".youtube.com", true),
            session_cookie("__Secure-3PSID", "psid3", ".youtube.com", true),
            session_cookie("__Secure-1PSIDTS", "psidts1", ".youtube.com", true),
            session_cookie("__Secure-3PSIDTS", "psidts3", ".youtube.com", true),
            session_cookie("VISITOR_INFO1_LIVE", "visitor", ".youtube.com", false),
            // Unrelated cookie that must never leak into the dump.
            session_cookie("sp_dc", "spotify", ".spotify.com", true),
        ],
    };

    let dump = native_dump(&window, YT_CAPTURE_ORIGINS).expect("the session must produce a dump");
    let lines: Vec<&str> = dump.lines().collect();

    let must_contain = [
        "__Secure-1PSID=psid1",
        "__Secure-3PSID=psid3",
        "__Secure-1PSIDTS=psidts1",
        "__Secure-3PSIDTS=psidts3",
        "SAPISID=sapisid-value",
        "SID=sid-value",
        "LOGIN_INFO=logininfo",
    ];
    for needle in must_contain {
        assert!(dump.contains(needle), "dump must contain {needle}");
    }
    assert!(!dump.contains("sp_dc"), "spotify cookie must not leak");
    assert!(
        dump.contains("__Secure-1PSID=psid1; Domain=youtube.com; Path=/; Secure; HttpOnly"),
        "HttpOnly session cookie must carry its attributes: got\n{dump}"
    );
    // Exactly the 13 youtube/google cookies, each once.
    assert_eq!(lines.len(), 13, "13 session cookies expected, got {}", lines.len());
    let mut names: Vec<&str> = lines.iter().map(|l| l.split('=').next().unwrap()).collect();
    names.sort_unstable();
    let before = names.len();
    names.dedup();
    assert_eq!(names.len(), before, "no cookie may appear twice");

    // The pre-fix behaviour, for the record: what cookies_for_url kept.
    let old_yt = window.cookies_for_url("music.youtube.com").unwrap();
    assert!(
        old_yt.is_empty(),
        "pre-fix filter must return nothing for music.youtube.com, got {old_yt:?}"
    );

    // Empty store (signed out) and a failing read both stay None — the
    // capture thread skips, the shim's document.cookie remains the fallback.
    let signed_out = FakeWindow { fail: false, store: vec![] };
    assert!(native_dump(&signed_out, YT_CAPTURE_ORIGINS).is_none());
    let broken = FakeWindow { fail: true, store: vec![] };
    assert!(native_dump(&broken, YT_CAPTURE_ORIGINS).is_none());

    // A Spotify-window report through the same code: only the spotify cookie
    // matches its origin, none of the youtube/google session leaks in.
    let spotify_origins: &[(&str, &str)] = &[("https://open.spotify.com/", ".spotify.com")];
    let dump = native_dump(&window, spotify_origins).expect("sp_dc must be captured");
    assert_eq!(dump, "sp_dc=spotify; Domain=spotify.com; Path=/; Secure; HttpOnly");
    assert!(!dump.contains("PSID"), "no youtube cookie may leak into the spotify dump");

    println!("all harness checks passed ({lines_len} session cookies captured)", lines_len = lines.len());
}
