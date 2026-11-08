# BitChord Desktop — Build & Runtime Fixes

This is the fixed source tree of `bitchord-desktop` (v1.9.1). The original
source did not compile — the Rust shell (`src-tauri/src`) had missing module
imports, a missing crate re-export, a plugin API misuse, and two invalid
fields in `tauri.conf.json`. Every fix is additive or corrective; **no file
was removed and no feature was dropped** (verified by a full recursive diff
against the original archive: 5 files changed, 0 removed).

## What was fixed

### 1. `src-tauri/src/commands.rs` — missing module imports (18 compile errors)
`commands.rs` referenced `agent::…` and `media::…` without importing the
crate modules, so every use failed to resolve. Added:

```rust
use crate::{agent, media};
```

### 2. `src-tauri/src/media.rs` — missing `Manager` trait import (3 compile errors)
`AppHandle::state`, `try_state` and `get_webview_window` come from Tauri's
`Manager` trait, which was not in scope. Changed the import to:

```rust
use tauri::{AppHandle, Manager};
```

### 3. `src-tauri/src/lib.rs` — window-state plugin built incorrectly (1 compile error)
`tauri_plugin_window_state::Builder` implements no `Plugin` trait itself; the
plugin instance is produced by its `build()` method:

```rust
.plugin(tauri_plugin_window_state::Builder::default().build())
```

### 4. `src-tauri/core/src/lib.rs` — `Core` not exported from the core crate (3 compile errors)
The shell uses `bitchord_core::Core`, but the type lives in the private-ish
`bridge` module and was never re-exported. Added at the crate root:

```rust
pub use bridge::Core;
```

### 5. `src-tauri/tauri.conf.json` — invalid config fields (build-time config validation error)
`bundle.linux.appCategories` and `bundle.linux.desktopFileTemplate` do not
exist in the Tauri v2 configuration schema (`tauri-utils` `LinuxConfig` only
has `appimage`, `deb`, `rpm`), so `tauri-build` rejected the whole config and
no build could start. Replaced the block with `"linux": {}`. Nothing is lost:
desktop categories were already derived from `bundle.category` ("Music").

## How the Windows installer was produced

Cross-compiled from Linux with the officially documented Tauri toolchain
(https://v2.tauri.app/distribute/windows-installer/):

```
rustup target add x86_64-pc-windows-msvc
cargo install --locked cargo-xwin
pnpm install
pnpm --filter @bitchord/app build
tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis
```

Notes for reproducing on a normal machine:
- The `tauri` CLI must run from the repository root (it looks for
  `src-tauri/tauri.conf.json` in subfolders), e.g.
  `cd <repo-root> && ./app/node_modules/.bin/tauri build …` or use
  `pnpm --filter @bitchord/app exec tauri build` with the CLI's project
  path pointed at the repo root.
- The installer is **unsigned** (no certificate), so Windows SmartScreen may
  show a warning on first run — choose "More info" → "Run anyway".
- The installer bundles/boostraps WebView2 (`downloadBootstrapper`), so the
  app itself needs no bundled browser runtime.

## Artifacts

- `BitChord_1.9.0_x64-setup.exe` — the NSIS setup installer (per-user or
  per-machine, English, WebView2 bootstrapper).
- `BitChord-portable.exe` — the same release binary without an installer,
  for portable/green use.

---

# Round 2 — the sign-in fixes (v1.9.1)

The build now works, but three independent bugs made sign-in impossible and
one made a robots.txt window appear at startup. All four were reproduced from
first principles against the Tauri 2.12.5 sources before fixing.

## 1. The white sign-in window that never loads and cannot be closed

**Symptom:** clicking the sidebar "Sign In" pill opened a blank window, the
whole app froze, and only Task Manager ended it.

**Cause:** `open_external` and `open_signin` were *synchronous* Tauri commands
that called `WebviewWindowBuilder::build()`. Sync commands execute on the main
thread — which is the event loop — and building a webview window there
deadlocks the Windows WebView2 initialization. This is a documented known
issue (the `WebviewWindowBuilder` docs: "On Windows, this function deadlocks
when used in a synchronous command or event handlers"; tauri#13092).

**Fix:** both commands are now `async fn`. Async commands run off the main
thread, which is the documented-safe pattern for window creation.

## 2. Sign-in could never complete, even with a loaded window

**Cause:** the agent window runs on remote origins (accounts.google.com,
music.youtube.com, …). In Tauri 2, **every IPC call from a remote URL is
rejected by the ACL unless the app defines its own permission manifest and a
capability with `remote` URLs grants those commands** (the check fires when
`!is_local`, per `webview/mod.rs`). The app defined no permissions, so the
agent's `agent_report` invoke — the one carrying the session cookies — was
silently denied. Application commands only work by default from *local*
windows, which is why everything else seemed fine.

**Fix:** `build.rs` now registers the eight desktop commands with
`tauri_build::AppManifest::new().commands([…])`, which autogenerates
`allow-<command>` permissions, and both capabilities (`main`, `agent`) grant
them. The `agent` capability's `remote.urls` also gained the origins the
Google sign-in flow hops through (accounts.youtube.com, consent.google.com,
myaccount.google.com, www.google.com) so the report works from wherever the
redirect lands.

## 3. The robots.txt "console" window at startup

**Symptom:** ~8 s after launch, a window opened showing
`# robots.txt file for YouTube Music …`.

**Cause:** two stacked mistakes, both verified in source:

* The shell opened a *hidden* agent window (`agent-quiet`) on
  `https://music.youtube.com/robots.txt` every 15 minutes to refresh YouTube's
  rotating cookies.
* The window-state plugin's `restore_state()` initializes
  `should_show = true` and **shows any window it has no saved state for** —
  every fresh install's first run — even a window created with
  `visible(false)`. So the "hidden" robots.txt window appeared on screen.

**Fix:** the hidden-window refresh is gone entirely — it was redundant: the
core's cookie jar is the `cookie_provider` for every YouTube request, so
YouTube's own `Set-Cookie` responses keep it current; after sign-in the jar is
re-serialized into the persisted dump (`YtService::cookie_dump`). The
window-state plugin also denylists the agent window, so it is never tracked,
restored or saved.

## 4. Google would refuse the sign-in anyway ("This browser or app may not be secure")

**Cause:** Google blocks sign-in from embedded engines (WebView2, WKWebView,
WebKitGTK) — a long-standing policy (WebView2Feedback#1578/#1584). A white
sign-in page would have been the next report.

**Fix:** the agent window now claims a plain Chrome 138 user agent
(`agent.rs::chrome_user_agent`), and a new `shim/chrome_spoof.js` papers over
the JS fingerprints Google checks (`window.chrome`,
`navigator.userAgentData`, `navigator.plugins`, `navigator.webdriver=false`).
Both scripts run as initialization scripts *and* are re-evaluated on every
page load (`on_page_load` → `Started`), because the OAuth flow hops origins
and each document is a fresh JS world.

## Smaller changes in this round

* `shim/agent.js` derives the service from `location.hostname` at report time
  instead of a baked-in kind, so one agent window can move between Google,
  Spotify and Discord targets (the shell reuses it via `navigate()` instead of
  closing and rebuilding, which raced).
* The YouTube/Spotify/Discord HTTP clients claim Chrome 138, matching the
  agent window.
* `ci.yml`: repaired the corrupted `branches:` value (`ain, master]` →
  `[main, master]`); the workflow would not have parsed on GitHub.
* Version bumped to 1.9.1 so the fixed build is recognizable in
  Add/Remove Programs and About.
* The `tokio` dependency left the shell crate with the timer gone.

---

# Round 3 — the session wasn't accepted (v1.9.2)

The sign-in window now opens and Google accepts the sign-in, but the app still
answered "account_menu: YouTube answered as if signed out (the session wasn't
accepted)". Reproduced from the cookie-handling code path end to end; two
runtime bugs and one build blocker this time.

## 1. YouTube rejected the session: `document.cookie` cannot see HttpOnly cookies

**Symptom:** sign-in completes inside the agent window, but every
`account/account_menu` call comes back with `logged_in: 0`.

**Cause:** the agent shim reported the page's `document.cookie`. That only
ever exposes JS-readable cookies — `SAPISID` among them, which is why the
signature could be built and `browser_signed_in` was true. But YouTube
*identifies* the session by cookies JavaScript cannot read:
`__Secure-1PSID`, `__Secure-3PSID`, the rotating `__Secure-1PSIDTS` /
`__Secure-3PSIDTS` pair, `SID`, `HSID`, `SSID`, `LOGIN_INFO` — all HttpOnly.
A jar without them is the signed-out jar no matter how correct the
SAPISIDHASH header is (this is also why the browser extension worked: the
browser attached its *full* cookie jar to every request, HttpOnly included;
the port has to reproduce that jar itself). The old `set_cookies` also
force-assigned every cookie to `.youtube.com`, losing the `.google.com` half
of the session (`SID`/`HSID`/`SSID`/`SAPISID` live there).

**Fix:** the agent window's cookies are now captured *natively*, from the
webview's own cookie store, where HttpOnly cookies live:

* `src-tauri/src/agent.rs` runs a single background capture thread
  (`cookies_for_url`, Tauri 2's API — "returns all cookies … including
  HTTP-only and secure cookies") polling every 1.5 s while the agent window
  exists. It reads **both** domains the session spans —
  `https://music.youtube.com/` and `https://accounts.google.com/` — and
  reports through the same `handle_report` path the shim uses. It never
  touches the main thread (same deadlock rules as window building).
* `bitchord-core`'s `YtService::set_cookies` now parses one full
  `name=value; Domain=…; Path=…; Secure; HttpOnly` line per cookie, keeps
  each cookie's real domain (whitelisted to the `youtube.com` /
  `google.com` suffixes the session spans, RFC 6265 domain-match honoured —
  Google-domain cookies are added through the Google URL, the jar rejects
  the rest), and `YtService::cookie_dump` persists the same multi-line shape
  so both halves survive a restart. The old one-line `document.cookie`
  reports still parse (every pair becomes a `.youtube.com` cookie), so the
  shim's report remains a working fallback for engines without a native
  cookie API.
* `shim/agent.js` is unchanged; its reports are simply no longer the only
  source.

## 2. The window closed before the session existed, and re-verified the same report every 2 s

**Cause:** `handle_report` closed the agent window on the *first* report
containing `SAPISID` — which can appear on accounts.google.com while the
redirect chain is still running, i.e. before YouTube has set the
`.youtube.com` session cookies. Every identical report also re-ran the full
account read (10 `account_menu` calls + a homepage fetch) — the error text
the UI showed was produced repeatedly by that loop.

**Fix:** `handle_report` now dedups reports by fingerprint (sorted
`name=value` pairs, attributes stripped — identical across both dump
shapes), and the window closes only when the core *verified* a session
(`auth_state().signed_in`) that differs from the one the window was opened
with (`AgentState`'s per-open baseline, refreshed by every `open_signin`).
Mid-flow reports leave the window up; a user reopening sign-in to switch
accounts doesn't get the window yanked away. The same guards apply to the
Spotify `sp_dc` and Discord token paths (which also stopped repeating the
Discord `users/@me` lookup per poll).

## 3. Build blocker found by the cross-check: the capabilities named permissions that don't exist

**Symptom:** `tauri build` panics in `build.rs` with `invalid plugin or
permission identifier 'allow-bridge_call'` (this is one of the issues the
local build had needed hand-fixing before).

**Cause:** the eight desktop commands were registered via
`tauri_build::AppManifest::new().commands([…])`, which autogenerates
hyphenated permissions (`allow-bridge-call`) under
`permissions/autogenerated/` — but the `main` and `agent` capabilities
referenced *underscored* names (`allow-bridge_call`). Underscores are not
valid in ACL identifiers at all (lowercase ASCII and hyphens only), so the
capability parse fails and tauri-build refuses the whole manifest. The
mismatched references were doubly broken: invalid by schema *and* pointing
at a permission that didn't exist.

**Fix:** the documented application-permissions route replaces the
autogeneration, so the names live in one visible place:

* `src-tauri/permissions/desktop-commands.toml` defines eight hyphenated
  permissions (`allow-bridge-call`, …) whose `commands.allow` lists carry
  the real command names;
* `build.rs` is back to plain `tauri_build::build()` and the stale
  `permissions/autogenerated/` directory is deleted from the tree — the
  permissions no longer depend on build-time autogeneration;
* the `main` and `agent` capabilities reference the hyphenated identifiers.
  Nothing about the ACL's reach changed — the agent capability still grants
  the commands to the remote service origins, which is what makes
  `agent_report` work from the sign-in page.

## Verification

* `cargo test` (core): 10/10 — three new tests cover the dump round-trip
  (both domains + HttpOnly flags survive), foreign-domain refusal, the
  legacy `document.cookie` shape, and dump → jar → dump stability.
* `cargo xwin check --target x86_64-pc-windows-msvc` (the shell, cross-
  checked against the real Windows target): clean.
* `pnpm -r typecheck && pnpm -r test`: 159/159 — the frontend is untouched.

Version 1.9.2.

---

# Round 4 — the v1.9.2 build verification and the installer

The v1.9.2 source tree (rounds 1–3 applied) was re-verified end to end and
the Windows installer was produced from it. **No source file needed another
fix** — every check below passed against the tree as uploaded:

* `pnpm --filter @bitchord/app build` (tsc + vite): clean, 2.2 s.
* `pnpm -r typecheck`: clean.
* `cargo test` (core): 10/10.
* `cargo xwin check --target x86_64-pc-windows-msvc` (shell): clean —
  only the documented dead-code warnings for the Linux-only media
  functions, which Windows never calls (WebView2 has its own media
  session).
* `cargo xwin build --release --target x86_64-pc-windows-msvc
  --features tauri/custom-protocol`: produced `bitchord-desktop.exe`
  (PE32+ x86-64).
* `tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc
  --bundles nsis`: produced the installer.

One packaging-time note, not a code change: the tauri CLI's tray-icon
probe needs an appindicator library on the *build host* even when the
target is Windows. Pointing `PKG_CONFIG_PATH` at the user-space
ayatana-appindicator files (and `TAURI_LINUX_AYATANA_APPINDICATOR=1`)
satisfies it; the Windows binary doesn't link them.

## Artifacts (this round)

* `BitChord_1.9.2_x64-setup.exe` — the NSIS setup installer (28.6 MB,
  per-user or per-machine, English, WebView2 bootstrapper), verified as a
  Nullsoft self-extracting PE carrying the BitChord 1.9.2 version info.
* `BitChord-portable.exe` — the same release binary without an installer.
* `bitchord-desktop-v1.9.2-fixed-src.tar.gz` — this source tree, file set
  identical to the uploaded archive.

The installer is unsigned: Windows SmartScreen may show a warning on first
run — choose "More info" → "Run anyway".

---

# Round 5 — every page failed with "Failed to fetch" (v1.9.3)

Sign-in now works, but every page except Settings died with
"Couldn't load this page — Failed to fetch", and "Try Again" did not help.

## Cause: the page's own fetches had nobody to carry them

The web app fetches YouTube **from the page itself** — YouTube.js posts to
`music.youtube.com/youtubei/…` and `www.youtube.com/youtubei/…`, reads
`www.youtube.com/sw.js_data` to build a session, pulls the player script from
`www.youtube.com/s/player/…`, and streams audio/video from `googlevideo.com`.
On the site this worked only because the **browser extension's
declarativeNetRequest rules** (`web/extension/lib/rules.ts::youtubeRules`)
did two things a page cannot do:

1. **Dressed the requests in YouTube's headers** — `Origin`, `Referer` and
   `X-Origin` (Impersonate as the YouTube Music / YouTube web clients;
   googlevideo as the YouTube web client, and VISIONOS direct URLs with no
   Origin/Referer at all — a YouTube web Origin on them is refused 403
   mid-track);
2. **Added `Access-Control-Allow-Origin` to the responses** — youtubei
   answers CORS with *nothing at all* (every capture in
   `web/fixtures/index.json` records `acao: none`), so a bare page fetch is
   killed by the browser's CORS check before YouTube even answers.

The desktop webview has no extension and no DNR. Both halves of that
contract were missing, so every page's data request died as
`TypeError: Failed to fetch` — Home, Search, Explore, Library, playlists,
artists, everything except Settings (which is local). It also turned out the
second half masks the first: even without CORS, youtubei refuses an
anonymous cross-origin request outright — measured live:

* the same POST **without** the impersonation headers → **HTTP 403**;
* with `Origin`/`Referer`/`X-Origin` of the Music origin → **HTTP 200** with
  the home-feed JSON.

## Fix: a page-network layer, the DNR rules reimplemented in Rust

The page's matched fetches now go to the Rust core, which sends them under
the exact rule set — same headers, and CORS never applies inside Rust:

* `src-tauri/core/src/http.rs` — `page_request_headers` mirrors
  `youtubeRules` one for one (InnerTube as each host's own web client;
  `sw.js_data`/`iframe_api`/`s/player/` untouched; googlevideo with
  Origin+Referer only; `c=VISIONOS` URLs with neither; `/api/jnn/`
  deliberately *not* served, like in the rules, because BotGuard's Create
  allows any origin and refuses a rewritten one), and `page_fetch` executes
  them on a cookie-free client (the page's YouTube requests are anonymous,
  `credentials: 'omit'`), Chrome 138, 60 s deadline (120 s for media).
* `src-tauri/shim/desktop.js` — new **section 4, "The page's network"**:
  `window.fetch` is wrapped at document start. A URL matching the rules is
  forwarded to the new `page_fetch` command and the answer is rebuilt into a
  real `Response` (content-type preserved, `content-encoding`/
  `content-length` dropped — reqwest already decoded the body; aborts
  surface as a proper `AbortError` DOMException; string bodies stay text,
  ArrayBuffer/TypedArray/Blob bodies ride as base64 — Shaka's protobuf and
  UMP traffic included). Everything else fetches natively first, exactly as
  on the site.
* **CORS fallback.** The Listen Together server answers *no* CORS headers on
  any response (measured: no `Access-Control-Allow-Origin` on any status;
  the site reached it same-origin through its own domain). A cross-origin
  https fetch that failed natively as a `TypeError` is retried once through
  the core with `fallback: true` — no impersonation, the page's own headers,
  https only. Same for lyric providers if a host drops its ACAO overnight:
  the direct route now degrades to the core before the `lyrics.fetch`
  extension route is even asked.
* `src-tauri/src/commands.rs` + `lib.rs` — the `page_fetch` command;
  `permissions/desktop-commands.toml` gains `allow-page-fetch`, and the
  `main` capability grants it.

The frontend is untouched — byte for byte the web build.

## Verification

* `cargo test` (core): 11/11 — the new test pins the rule table: both
  youtubei hosts impersonated as themselves, the bootstrap/player-script
  URLs passing untouched, googlevideo without X-Origin, VISIONOS with no
  Origin/Referer, jnn and foreign hosts refused, plain http refused.
* `cargo xwin check --target x86_64-pc-windows-msvc` (the shell): clean —
  only the same four documented dead-code warnings for the Linux-only media
  functions.
* `pnpm -r typecheck && pnpm -r test`: 159/159; `pnpm --filter @bitchord/app
  build`: clean.
* The shim's fetch layer was exercised against a mocked IPC in eight
  scenarios (rule interception, Response rebuild with transport headers
  dropped, UMP base64 round-trip and a streamable body, jnn passing through,
  same-origin untouched, the CORS fallback with `fallback: true` reaching
  the core, mid-flight aborts, `Request`-object bodies, ArrayBuffer bodies).
* Live end-to-end: the exact request the core now sends — anonymous
  WEB_REMIX `browse` with the rule headers — returns 200 with the home feed
  (and 403 without them); `www.youtube.com/sw.js_data` returns 200.

Version 1.9.3.

--------------------------------------------------------------------------------

## v1.9.4 — performance pass, sidebar trim, new icons

Feedback: "everything works perfectly, the only problem is the optimization."
Home idle was fine (1–2% CPU); playing burned 32–41% CPU with RAM climbing
(400 MB+ in three minutes), full-screen-with-lyrics ran 42–56% (peaks 78%) and
the lyrics side panel pushed the homepage to ~70% CPU / 600 MB — while toggling
the lyrics panel off dropped the CPU to ~17%. The difference between those
states is per-frame work, and that is what this pass removes.

### What was costing the frames

* The lyric clock wrote `el.currentTime` into the am-lyrics element **every
  animation frame**, and am-lyrics does real work on every write (per-word
  style updates, wipe-animation seeks, the predictive scroll): ~50% of the
  whole app while lyrics were open.
* The seek bars (the LCD's line and the full-screen scrubber — two instances
  in full screen) also ran 60 writes a second, each repainting the bar and the
  times; the original loop also had a latent double-schedule: a `seeked` event
  during playback started a second, parallel loop, and every seek added
  another.
* The full-screen mesh gradient rendered at full pixel density, 30 times a
  second, with nine Gaussian blobs per fragment — most of the player's GPU
  cost at full-screen sizes.
* All of it kept running when the window was hidden (where rAF still fires on
  some engines), and the churn — arrays and strings per frame across all three
  loops — is what kept the heap climbing toward its ~650 MB plateau.

### Changes (`app/src`)

* `shell/LyricsPanel.tsx` — the clock still advances at full rate, but lands
  on the element at most **30 writes a second**, and instantly on any jump
  (word boundary, seek, hold). While the window is hidden the clock holds and
  writes nothing; it picks back up on show.
* `shell/useScrubber.ts` — steady playback is drawn **10 times a second**;
  dragging, the Automix pulse and the blend covers keep the frame rate, since
  those move fast enough to show. Identical time text is skipped (Blink would
  otherwise invalidate the span each frame). The loop is now single-scheduler
  (one pending frame at a time — the seek double-loop is gone) and stops
  entirely while the window is hidden.
* `shell/MeshBackdrop.tsx` — the WebGL mesh renders at **half resolution**
  (soft blobs; the browser's upscale is invisible, the fragment work drops 4×)
  and draws nothing while the window is hidden.
* `shell/Sidebar.tsx` — the "BitChord Extension" row and the "Get the Android
  app" link are gone (both desktop-only noise); the bottom row is the account
  button and the settings gear.

### Icons

`src-tauri/icons/` now carries the icon set the extension toolbar uses
(`web/extension/public/icon/{16,32,48,96,128}.png` — the same mark on the red
gradient): the 32 px and 128 px files are those exact pixels, the 256 px and
512 px (`icon.png`) come from the same design at `ic_launcher-playstore.png`
(512, corner colours identical), and `icon.ico` carries 16/24/32/48 (the
extension set) plus 64/128/256 (from the 512 master); `icon.icns` embeds the
standard 32–1024 variants. File names are unchanged, so `tauri.conf.json` and
the tray (which uses `default_window_icon`) needed no edits.

### am-lyrics itself

Untouched. The throttled clock keeps the word-by-word wipe exactly as it was —
thirty updates a second is past what a moving wipe can show — so there was no
need to port another lyrics frontend.

### Verification

* `pnpm -r typecheck`: clean. `pnpm -r test`: 159/159. `pnpm --filter
  @bitchord/app build`: clean.
* No Rust changes; the core and shell are byte-identical to 1.9.3.

Version 1.9.4.

--------------------------------------------------------------------------------

## v1.9.5 — close goes to the tray, the icon forced red, memory trims

Feedback on 1.9.4: CPU down to ~30 % and RAM plateauing near 500 MB ("obviously
better but not good enough"), the close button **ended the app** instead of
minimizing to the tray, and the icon it had been updated to read as **blue**.

### 1. The close button quit the app

The tray-and-close code was in every build (`on_window_event` hides the main
window when `close_to_tray` is on, and the store default is `true`), so the
report pointed at the one writer that could turn it off for good: the tray's
own **Quit** item called `set_close_to_tray(false)` *and persisted it*. One
click on "Quit BitChord" — months of testing make that a certainty — wrote
`closeToTray: false` into `state.json`, and from then on every close button
quit the app, with no UI anywhere to turn it back on. A hidden trap, and the
report is exactly what it springs.

Fixed by removing the trap rather than re-defaulting it:

* The main window's `CloseRequested` now *unconditionally* hides to the tray;
  quitting lives in the tray menu alone. A music player's close button keeping
  the music is the behaviour the report asks for, and there is no flag left
  that an old build could have poisoned.
* The tray's Quit no longer writes anything: `app.exit(0)` and done.
* The `set_close_to_tray` command, its permission (`allow-close-to-tray`), the
  `close_to_tray` store field and the `AppState` flag are gone. A leftover
  `closeToTray` key in an existing `state.json` is ignored (the store never
  used `deny_unknown_fields`).

**Second bug this uncovered before it bit:** `tauri-plugin-window-state`
saves window state on exit **including visibility**, and restores with
`should_show = state.visible`. A session ended from the tray — main window
hidden — would save `visible: false`, and the *next launch* would restore a
main window that never shows: the app would appear to not open, leaving only
the tray icon. The plugin now runs with `StateFlags::all() & !VISIBLE`, so
position, size and maximization survive but visibility is never tracked; the
window is always created visible.

### 2. The icon looked blue

Nothing in the icon set ever was blue — the shipped files were the red mark
(measured), and so were the upstream ones. A blue icon on Windows is the
system's generic/Tauri default, which is what the taskbar falls back to when
the icon chain (exe resource → window class → icon cache) hands it nothing
usable — a stale icon cache or a rebuild that didn't re-embed resources both
do it. Two changes make the red mark deterministic instead of assumed:

* The window icon is now **set at runtime**, on the builder:
  `WebviewWindowBuilder::icon(Image::from_bytes(include_bytes!("../icons/128x128.png")))`.
  The taskbar takes the window's icon (`WM_SETICON`) first; from this build it
  comes from the app binary itself, not from what the exe's resources or the
  icon cache happen to hold.
* The tray icon is set the same way from `icons/32x32.png` (falling back to
  `default_window_icon`), instead of only the default.
* The whole set was rebuilt from the exact upstream files
  (`web/extension/public/icon/{16,32,48,96,128}.png`): `32x32.png` and
  `128x128.png` are those bytes verbatim, `128x128@2x.png` (256) and the large
  ICO/ICNS frames come from the 512 master (verified the same artwork, mean
  per-pixel delta 0.73/255), `icon.ico` carries BMP frames 16/24/32/48 —
  16, 32 and 48 from the upstream pixels — plus PNG frames 64/128/256, and
  `icon.icns` embeds 16–512.

If Windows still shows an old or generic icon on the shortcut or in Explorer
after installing: that is the icon cache, not the app — `ie4uinit.exe -show`,
or reinstall once more after a reboot. The window and tray icons come from
the runtime now and are immune.

### 3. Memory: what else was held

`Shaka` was configured to keep **120 s of audio ahead and 300 s behind** the
playhead — the whole played song retained, several MB per track, re-armed on
every song. It now keeps 60 s ahead and 30 s behind (five times the 0.01 s
rebuffering goal; seeking further back re-fetches, as every player does).
The rest of the 1.9.4 pass stands: the renderer's plateau outside these
buffers is Chromium itself (V8 + MSE + image caches), which is the same order
of memory a browser tab of the site uses.

### Verification

* `pnpm -r typecheck`: clean. `pnpm -r test`: 159/159. `pnpm --filter
  @bitchord/app build`: clean.
* `cargo test` (core): 11/11. `cargo check` (shell): clean — the four
  documented dead-code warnings for the Linux-only media functions remain.
* Icons verified by pixel sampling: every small frame is the upstream bytes,
  the ICO parses with its 7 frames, the ICNS with 6 sizes, all red-gradient.

Version 1.9.5.

---

# Round — v1.9.6-dynamic-island build & verification (source stays 1.9.5)

The dynamic-island round (see `PORT-NOTES.md`) arrived as
`bitchord-desktop-v1.9.6-dynamic-island-src (1).tar.gz` and **built clean on
the first pass** — every check in the build sequence passed with **no change
to any source file**. The island feature (window, commands, permissions,
capabilities, `island.html`) is fully self-consistent. Note the version fields
(`src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `Cargo.lock`) still read
**1.9.5**; per the no-speculative-changes rule the artifacts keep 1.9.5 even
though the tarball name says 1.9.6.

## What was verified this round

* `pnpm -r typecheck`: clean (app + shared). `pnpm -r test`: 159/159.
* `pnpm --filter @bitchord/app build` (tsc + Vite production): clean; `dist/`
  carries `index.html`, `island.html` and the onnx/worker assets.
* `cargo test -p bitchord-core`: 11/11.
* `cargo xwin check` + full release build for `x86_64-pc-windows-msvc`: clean
  (3 dead-code warnings, all in Linux-only code paths, pre-existing).
* NSIS bundling: `BitChord_1.9.5_x64-setup.exe` (NSIS 3.11, Unicode, LZMA),
  verified with 7-Zip: payload = `bitchord-desktop.exe` (PE32+ x64 GUI,
  FileVersion 1.9.5) + the NSIS plugin set; header/data sizes match the file
  size exactly. Unsigned, WebView2 via `downloadBootstrapper`.
* `cargo check --target aarch64-apple-darwin`: clean — the whole macOS ARM64
  tree (shell, core, objc2/souvlaki/tao/wry paths, island cfg blocks)
  compiles. This is a type/semantic check only: linking, `.app` bundling,
  codesigning and DMG creation need a macOS host, and were **not** performed
  or claimed.

## Source changes (preservation audit: 0 files removed)

* `src-tauri/gen/schemas/windows-schema.json` — regenerated by `tauri-build`.
  The copy shipped in the tarball predates the island permissions; the
  refreshed schema now describes `allow-island-action`, `allow-island-resize`
  and `allow-island-overlay`. Build metadata only, no behaviour change.
* `src-tauri/gen/schemas/macOS-schema.json` — added by `tauri-build` when the
  Apple Silicon target was checked (the macOS counterpart of the same
  auto-generated schema). Additive build metadata.
* This file — this section.

Totals against the uploaded archive: **283 original files → 284, 0 removed,
2 modified** (`windows-schema.json` regeneration + this documentation),
**1 added** (`macOS-schema.json`).

## Build environment notes (reproducibility, not source fixes)

These were environment-level obstacles found while building on a Linux host;
none of them touched the app's source, and all are already reflected in the
delivered binaries:

1. **`stdalign.h` missing from the xwin SDK cache** — `cargo-xwin`'s splatted
   MSVC CRT include tree ships `stdbool.h`/`stdint.h`/`stdatomic.h` but omits
   `stdalign.h`, so `ring` 0.17's C sources (`OPENSSL_STATIC_ASSERT` users)
   failed to compile for the Windows target. The standard C11 header was added
   to the local xwin cache (`~/.cache/cargo-xwin/xwin/crt/include/`). Nothing
   in the repo is affected; a full MSVC installation has this header.
2. **tauri-cli 2.12.4 appindicator panic** — on a Linux host the CLI probes
   GTK appindicator libraries unconditionally when the `tray-icon` feature is
   on, even for an NSIS/Windows bundle, and panics
   (`Can't detect any appindicator library`) when they are absent. A `pkg-config`
   shim answered only those two probes during bundling; the Windows binary
   never links them.
3. **NSIS on Linux** — the bundler does not download the NSIS toolkit on
   non-Windows hosts; NSIS 3.11 (`apt`-equivalent debs, user-space) plus a
   thin `makensis` wrapper that points `NSISDIR`/`NSISCONFDIR` at the
   user-space tree supplied it.
4. **macOS C build scripts on Linux** — `objc2-exception-helper` and `ring`
   compile C/Objective-C for the target during `cargo check`. zig cc
   (0.17.1) served as the `aarch64-apple-darwin` C compiler via small wrapper
   scripts (dropping `-arch`/`--target`/`-mmacosx-version-min` and letting zig
   set `--target=aarch64-macos.11.0`). Check-only: no macOS object code was
   linked or shipped.

## Artifacts (this round)

* `BitChord_1.9.5_x64-setup.exe` — Windows x64 NSIS setup installer
  (unsigned; installs per-user or per-machine; bootstraps WebView2).
* `bitchord-desktop-v1.9.5-fixed-src.tar.gz` — the source tree as audited
  above.
* No macOS DMG: the build host is Linux; a native Apple Silicon `.app`/DMG
  requires macOS (Xcode CLT, Apple SDK, `hdiutil`, codesign) and was
  explicitly **not** cross-faked. See the round report for what *was*
  verified for `aarch64-apple-darwin`.

# Round 6 — the Dynamic Island UX fixes (v1.9.5, dynamic-island tree)

Three user-visible defects in the island and one settings-placement change,
reported against the running app:

1. **The progress time desynced when switching songs** — the mini pill (and
   MPRIS / Now Playing) kept counting down the previous track's remaining
   time over the new track's title (e.g. the app at 0:28 / −3:18 while the
   pill read −2:28). Root cause: the desktop shim throttles
   `setPositionState` to one update per 900 ms, and the page only pushes a
   position state on `durationchange` / `ratechange` / `seeked` — never on
   `timeupdate` — so the one update a new track ever sends could be the one
   that got swallowed. The snapshot then rode the old track's timeline until
   the user happened to seek. Fixed in three layers:
   * `src-tauri/shim/desktop.js` — the throttle now always lets through an
     update whose `duration` differs from the last one sent (a track switch)
     or whose position drifted more than 1.25 s from the projected line (a
     seek that would otherwise be swallowed); a metadata change re-arms the
     throttle so the new track's first position update is never the dropped
     one.
   * `app/src/state/player.ts` — `updatePositionState()` now also runs on
     `timeupdate`, so the shell receives a steady (~1 Hz after the throttle)
     trickle of truth and any residual drift self-corrects within a second.
   * `src-tauri/src/media.rs` — a metadata update that actually changes the
     track (title or artwork differ) zeroes `position_sec`, so the switch
     reads as the new song starting from 0:00 instead of carrying the old
     position across the metadata gap.
2. **The expanded/mini morph looked buggy and choppy** — three contributors,
   all fixed:
   * `app/src/island/main.ts` — the expand used to flip the CSS class the
     same tick it asked the shell for a bigger window, so the growing card
     clipped at the pill's 216 px for the first frames (a visible stutter),
     and the late class flip raced the animation. Now the shell resolves
     `island_resize` first, two painted frames pass, and only then does the
     morph start; a token guards the flip against a collapse that arrives
     mid-expand. Bonus fix: collapsing from *full* straight to *hidden*
     (pause while expanded) never scheduled the window shrink, so after a
     resume the pill sat in a 432×188 window that ate clicks around it —
     the shrink now follows any departure from `full`.
   * `src-tauri/src/island.rs` — `apply_size` runs the size write and the
     re-centre inside one `run_on_main_thread` task; issued as two separate
     main-thread tasks, the window painted an intermediate frame at the new
     size and old position, and the pill visibly jumped sideways on every
     expand and collapse.
   * `app/src/island/island.css` — the two faces crossfade as a morph now:
     the outgoing face fades fast (0.16 s, settles back to 0.94 scale), the
     arriving one waits for the box (0.08 s) and eases up with it; delays
     apply only on arrival, so leaving is instant. Reduced-motion kills the
     delays too.
3. **The island toggle was only in the tray** — removed the tray checkbox
   entirely and moved the switch to the settings page:
   * `src-tauri/core/src/types.rs` — the `island` capability joins `ping`'s
     list (a browser extension never reports it, so the web build is
     unaffected).
   * `src-tauri/core/src/bridge.rs` — two new bridge methods,
     `island.state` and `island.setEnabled`, reading/writing the same
     `island_enabled` store key (still `None` = on).
   * `src-tauri/src/commands.rs` — an ok'd `island.setEnabled` now applies
     the preference in the shell (`island::apply_enabled`: create or destroy
     the overlay window) before the response returns, the same shape as
     `spotify.signIn` opening its agent window.
   * `src-tauri/src/island.rs` — `apply_enabled` replaces the old
     `set_enabled`/`toggle` pair; `init` uses it too.
   * `src-tauri/src/tray.rs` — the checkable Dynamic Island item,
     `TrayIslandItem` and its event arm are gone; the menu is Open /
     transport / Launch at login / Quit again.
   * `shared/src/bridge/protocol.ts` — `'island'` capability plus the two
     typed methods.
   * `app/src/extension/useIsland.ts` (new) — the query/mutation pair over
     the bridge, optimistic on the toggle.
   * `app/src/pages/SettingsPage.tsx` — a **Desktop** section (rendered only
     where the shell reports the capability) with the Dynamic Island switch.
4. **The "BitChord Extension" row left Settings** — it existed for testing
   the extension bridge; the `/extension` page itself is kept and routed as
   before, only the settings row is gone.

## Verification this round

* `pnpm -r typecheck`: clean. `pnpm -r test`: 159/159.
* `pnpm --filter @bitchord/app build` (tsc + Vite production): clean.
* `cargo check -p bitchord-core`: clean. `cargo test -p bitchord-core`:
  11/11. (The Tauri shell itself was not compiled here: the check host has
  no root, and WebKitGTK/GDK development libraries could not be installed;
  the shell's changes were verified against the vendored `tauri` 2.12.5
  source for the APIs used — `AppHandle::run_on_main_thread`, `set_size`,
  `LogicalSize` — and by review.)
* `node --check src-tauri/shim/desktop.js`: clean.

# Round 7 — the island's manners, the transparent top bar, and the light lyric clock (v1.9.5)

## 1. The Dynamic Island now belongs to the glance away, not to the app

The card used to float whenever music played, including over BitChord's own
screen — where it repeated what the page already says and sat in the way of
the window's top. The shell now tracks the main window's focus and the island
re-decides what to show on every input:

* `src-tauri/src/island.rs` — a `MAIN_FOCUSED` atomic plus
  `notify_main_focus`, which stores the flag and evals
  `window.__IslandFocus(<bool>)` into the island page, and `main_focused`
  for the page's one boot-time ask. The flag starts *false*, so a session
  that began hidden to the tray may show the card at once; a normal launch
  corrects it with the window's first `Focused(true)`.
* `src-tauri/src/lib.rs` — the `on_window_event` hook forwards the main
  window's `Focused` events. Hiding the window to the tray fires the same
  event, which is what keeps the card up over the desktop after a
  close-to-tray.
* `src-tauri/src/commands.rs` — `island_focus_state` answers the page's
  single query (`allow-island-focus-state` in `permissions/` +
  `capabilities/island.json`), covering an island created while BitChord is
  already in the background.
* `app/src/island/main.ts` — one `reconcile()` policy decides the mode from
  three inputs, in order: paused (nothing), BitChord focused (nothing), a
  right-click dismissal (nothing). Music playing + app in the background +
  no dismissal is the only state where the pill exists. A dismissal is
  cleared *only* by the app regaining focus, so the island returns exactly
  when the user has been back in BitChord and left it again — never on a
  song change.

## 2. The spawn, the two-step morph, and the auto-expand

* **Spawn-in** (`island.css` + `main.ts`): coming out of `hidden` the card
  now pops — `island-spawn`, 0.55 s spring: 0 → 55% a grow from 0.55 scale
  with the opacity arriving, the last quarter a settle dip to 0.97 and back.
  Every frame stays inside the card's own box, so the OS window that hugs
  the pill never clips it; the class removes itself on `animationend`.
* **The centre-left-then-centre morph**: on Windows, `set_size` anchors the
  top-left corner (SWP_NOMOVE), so the collapse's shrink parked the 216px
  pill left of centre for a frame before `recenter` walked it home — the
  reported "goes to the centre left and then the centre". `apply_geometry`
  (island.rs) now issues *one* atomic `SetWindowPos` carrying position and
  size together — a single WM_WINDOWPOSCHANGED has no between for a frame
  to land in. New `[target.'cfg(windows)'.dependencies] windows = "0.62"`
  (same 0.62.2 the tao/tauri tree already pins; features only).
* **No auto-expand on a song change**: the `expand()` on `trackChanged` is
  gone from `__IslandSync`; a new track refreshes what the card shows,
  never whether it shows. The user's own click on the pill still expands
  it, with the same five-second auto-collapse as before.

## 3. The transparent top bar (Windows/Linux; macOS untouched)

The native frame is gone from the main window — `decorations(false)` for
`#[cfg(not(target_os = "macos"))]` in lib.rs — so no title, no icon and no
caption buttons interrupt the app's own background, and a tinted artist,
album or playlist page colours the top of the window like it colours the
rest. What the frame did, the page now does:

* `app/src/shell/WindowControls.tsx` (new) — a transparent top strip: the
  three window buttons top right in the app's own tokens (hover quinary;
  close hovers the system red), a window-level mousedown within the bar's
  40px that drags the window (`window_start_drag`) *unless* it landed on
  content (buttons, links, fields keep their clicks), and a double-click
  there that toggles maximize. The strip itself paints nothing and lets
  every pointer event through (`pointer-events: none`), so page content
  scrolled under it stays clickable.
* `app/src/shell/useWindowChrome.ts` (new) — one ask to `desktop_info`
  decides whether this window ships the bar: desktop Windows/Linux only.
  macOS keeps its traffic lights over the glass; a plain browser has
  neither window nor shell, so nothing renders there.
* `src-tauri/src/commands.rs` — `window_minimize`, `window_toggle_maximize`
  (the maximize branch lives in Rust, keeping `is_maximized` out of the
  ACL), `window_hide` (the tray close the native button always did) and
  `window_start_drag`, all async like every other window-touching command.
  Registered in lib.rs, granted in `permissions/desktop-commands.toml` +
  `capabilities/main.json`.
* `app/src/shell/Sidebar.tsx` — the "BitChord" wordmark and logo left the
  window's top; the spacer keeps the traffic lights (macOS) and the drag
  zone their room.
* `app/src/shell/AppShell.tsx` — the bar mounts once, outside the scrolling
  page (skipped on phone browsers, which have `MobileTopBar`).
* `app/src/shell/FullScreenPlayer.tsx` — the player's own top-right corner
  chrome (browser full screen, video quality) steps aside by 132px where
  the window buttons live, so the two never overlap.

## 4. The lyric clock stopped polling

The lyrics view wrote the media time onto am-lyrics thirty times a second,
and every write swept every word, syllable and character of the active
lines (class checks, wipe seeks, the predictive scroll) — measured from the
user's side as nearly double the system resources while a panel was open,
on top of the paint the animation itself costs. But am-lyrics animates
itself: every wipe, rise and blur is a CSS/WAAPI animation that advances on
its own clock once started, so a write is only needed when something new
begins.

* `app/src/shell/LyricsPanel.tsx` — `collectEvents` reads the same TTML
  am-lyrics loads into a sorted set of instants (line starts/ends, every
  word and syllable edge, background vocals), and the clock writes only
  there: exactly at each boundary (forced — a boundary is a class flip),
  at the leads am-lyrics itself looks ahead from (2000ms, the next line's
  unblur; 500/400ms, wipe prep, predictive scroll, gap collapse), and a
  500ms heartbeat bounded to a 1s sleep cap between far-apart events.
  Seeks and pauses write at once, as before. Renders dropped from ~30
  sweeps/s to a handful, and a boundary write now lands to the frame —
  closer than the polled cadence's 33ms ever was. Same element, same TTML,
  same CSS: the look is am-lyrics', unchanged; nothing runs while the
  window is hidden or the song is paused. A TTML that refuses to parse
  falls back to the old fixed cadence rather than risking the sync.

## Verification this round

* `pnpm -r typecheck`: clean. `pnpm -r test`: 159/159.
  `pnpm --filter @bitchord/app build` (tsc + Vite production): clean.
* `cargo xwin check --target x86_64-pc-windows-msvc` (cargo-xwin, as in
  Round 4): clean — the whole shell compiles for Windows, including the
  atomic `SetWindowPos`, the `hwnd()` path and the new commands. The Linux
  target still cannot be compiled on this host (no root, no WebKitGTK dev
  packages); the non-Windows `apply_geometry` is the pre-existing
  `set_size` + `recenter` pair, unchanged.
* Browser-driven behaviour matrix over the built `island.html` (headless,
  with the sync/focus evals driven by hand): initial load stays hidden;
  playing + unfocused spawns (`mini spawn`, opacity and scale animated,
  class cleared on `animationend`); right-click dismisses; a song change
  while dismissed stays hidden; focus regained clears the dismissal without
  showing the card; focus lost again spawns; pause hides; resume spawns;
  click expands to the 432×188 card. The main shell was loaded headless
  with the shell stubbed: the strip renders (Minimize / Maximize / Close,
  z-index above the full-screen player, below the menus), a mousedown on a
  button presses no drag, a mousedown on empty top space presses
  `window_start_drag`, one below the bar does nothing, and a double-click
  presses `window_toggle_maximize`; in a plain browser the strip renders
  nothing and the sidebar's top carries no wordmark.

---

# Round 8 — the bar that came back, the buttons in full screen, the wordmark's home, and the scrollbar (v1.9.5)

The screenshot told the whole story: the native title bar sat above the
frameless build's own button strip (two rows of window controls), the
sidebar's BitChord wordmark was gone, and a classic Windows scrollbar ran
down the window's right edge. Four fixes, one root cause.

## 1. The native title bar was never really gone — the state file kept bringing it back

**Symptom:** the window still had its native caption row ("BitChord", icon,
minimize/maximize/close) above the page's own button strip — two rows of
window controls.

**Cause:** `lib.rs` builds the main window with `decorations(false)`, but the
window-state plugin restores state on `on_window_ready` — which fires
*inside* `builder.build()`, after the frame was already removed. Round 7
excluded only the `VISIBLE` flag; `DECORATIONS` was still in
`StateFlags::all()`, so the plugin read the state file the old, decorated
build had written (`"decorations": true`) and called `set_decorations(true)`
over the frameless window. Upgrades from any pre-7 build drew the native bar
back on first launch.

The re-decoration also explains the scrollbar: the webview was sized for a
frameless window, and the caption appearing over it disturbed the layout —
content that fit no longer did, and WebView2's classic bar (arrows, track,
~17px) appeared at the right edge.

**Fix, two layers:**

* `src-tauri/src/lib.rs` — the plugin's flags now exclude `DECORATIONS` as
  they excluded `VISIBLE`:

  ```rust
  .with_state_flags(StateFlags::all()
      & !StateFlags::VISIBLE
      & !StateFlags::DECORATIONS)
  ```

  The frameless design is not a per-session preference to remember; the
  plugin neither restores nor saves it.
* Belt and braces: after `builder.build()` the setup re-asserts
  `main.set_decorations(false)` (non-macOS). The plugin's restore has
  already had its say by then, so even a stale state file loses.

## 2. The wordmark lives in the sidebar, where it always did

Round 7 over-reached: it removed the logo from the sidebar's top (leaving an
empty spacer), when the request was about the window bar's title. The
original block is back verbatim in `app/src/shell/Sidebar.tsx` — the
`LogoMark` glyph and the "BitChord" wordmark, linking home, `h-18` tall as
before, which still keeps the macOS traffic lights their air. The window's
top edge stays brandless; the app's navigation carries the brand.

## 3. No window buttons in full screen — at all

The full-screen player is the app's own full-bleed stage with its own corner
chrome and its own way out; a second row of window buttons floating over it
has nothing to do there.

* `app/src/shell/WindowControls.tsx` — the bar reads the UI store's
  `fullScreen` flag and stands down entirely: no buttons, and the
  window-level drag/double-click listeners are unbound too (a mousedown in
  the player's top area must never start a window drag). It returns the
  moment the player closes.
* `app/src/shell/FullScreenPlayer.tsx` — Round 7's workaround is gone: the
  player's own top-right chrome (enter/exit full screen, video quality) no
  longer steps aside by 132px for buttons that are no longer there. It sits
  at `right-5`, exactly where it always did.

## 4. The scrollbar is the app's own now

Beyond the re-decoration that introduced it, the classic Windows scrollbar
would have come back with any page taller than the window. It is retired:

* `app/src/styles/global.css` — `#scrollable-page` (the app's main scroller)
  and `html` get a thin, rounded thumb in the app's own materials
  (`--systemTertiary`, hover `--systemSecondary`) over a transparent track,
  8px, no arrow buttons, present only when there is something to scroll.
  Both the standard properties (`scrollbar-width: thin`,
  `scrollbar-color`) and the WebKit aliases are set, so older WebView2s get
  the same treatment. The sidebar and the shelves hide their scrollbars
  outright; this is the middle ground for the one view where mouse users
  still need the affordance.

## Verification this round

* `pnpm -r typecheck`: clean. `pnpm -r test`: 159/159.
  `pnpm --filter @bitchord/app build` (tsc + Vite production): clean.
* `cargo xwin check --target x86_64-pc-windows-msvc` (cargo-xwin + LLVM
  clang-cl/llvm-rc): clean — the flag exclusion, the re-assertion and the
  whole shell compile for Windows.
* Browser-driven behaviour matrix over the dev shell (headless, Tauri stub
  injected before the module graph): the sidebar renders the wordmark and
  the logo mark; Minimize/Maximize/Close render exactly once; a press on
  Minimize invokes `window_minimize` and is not a drag; a mousedown on
  empty top space presses `window_start_drag`; a double-click there presses
  `window_toggle_maximize`; `#scrollable-page` computes `scrollbar-width:
  thin`; with the full-screen player up (a track seeded into the player
  store) the three buttons are gone, the only Close is the player's own at
  the top-left, the enter-full-screen button's right edge is 20px from the
  window's (right-5, no offset), and empty top space presses nothing;
  closing the player brings the buttons back. 14/14.

# Round 9 — macOS: the sign-in that never verified, and the window that could not be moved (v1.9.5)

Two reports from a Mac, one tree that had only ever been exercised on
Windows: the Google sign-in window opens, the login goes through, YouTube
Music opens — and then nothing. The window never closes, the app stays
signed out; a second attempt ends in an "account error" while the settings
page insists the browser session exists. And independently: the window
cannot be dragged at all.

## 1. The native cookie capture was empty on macOS — every session cookie was filtered out

**Symptom:** sign-in completes inside the agent window (Google accepts the
login, music.youtube.com loads), but the window stays open and the app never
sees the session. Re-opening sign-in shows Google's account error, while the
app considers the browser signed in (the partial session it did receive).

**Cause:** `agent.rs::native_dump` reads the agent webview's *native* cookie
store with `window.cookies_for_url(url)` for the two origins the YouTube
session spans — and wry's `cookies_for_url` is only correct where the engine
does its own matching. WebView2 hands the URI to its cookie manager
(`GetCookies`), libsoup does the same for WebKitGTK. On macOS, wry 0.57.1
fetches **every** cookie from `WKHTTPCookieStore` and then filters with a
plain string compare:

```rust
cookie.domain() == url.domain()   // wry, src/wkwebview/mod.rs
```

`NSHTTPCookie.domain` carries the `Set-Cookie` attribute verbatim
(`.youtube.com`), which the cookie crate surfaces without the leading dot —
while the URL's host is `music.youtube.com`. A host is never equal to its
parent domain, so **every cookie a site set with a `Domain=` attribute was
dropped** — which is YouTube's whole session: `SID`/`HSID`/`SSID`,
`__Secure-1PSID`/`__Secure-3PSID`, the rotating `PSIDTS` pair, `LOGIN_INFO`,
`SAPISID`. The capture returned nothing; the only reports left were the
shim's `document.cookie` fallbacks, which cannot see HttpOnly cookies — the
exact shape Round 3 documented as "signed out answers however correct the
SAPISIDHASH is". So the verification always failed (`fresh.signed_in ==
false`), the close guard never fired, and the half-session that did land in
the store (the non-HttpOnly cookies) is what made the app claim the browser
was signed in.

**Fix:** on macOS the whole store is read once with
`tauri::WebviewWindow::cookies()` (no engine filtering at all) and matched
per origin with the rules a browser applies — RFC 6265 §5.1.4 domain-match,
implemented as `agent.rs::domain_matches`:

```rust
fn domain_matches(cookie_domain: &str, host: &str) -> bool {
    let domain = cookie_domain.trim().trim_start_matches('.')
        .trim_end_matches('.').to_ascii_lowercase();
    let host = host.trim().to_ascii_lowercase();
    if domain.is_empty() || host.is_empty() { return false; }
    if host == domain { return true; }
    host.len() > domain.len()
        && host.ends_with(&domain)
        && host.as_bytes()[host.len() - domain.len() - 1] == b'.'
}
```

`music.youtube.com` receives the `.youtube.com` cookies and its own
host-only cookies; `accounts.google.com` receives the `.google.com` half;
`notyoutube.com` matches nothing; a secure cookie rides https only. Windows
and Linux keep `cookies_for_url` — their engines match correctly and their
behaviour is unchanged (the platform split is `#[cfg]`, and `domain_matches`
is compiled on macOS alone). The dump's shape is untouched, so the core's
`set_cookies` parser and every downstream guard run exactly as before.

## 2. The window could not be moved on macOS — the drag zone was Windows/Linux-only

**Symptom:** the app window cannot be dragged anywhere on screen.

**Cause:** the page's drag zone lives in `WindowControls`' window-level
mousedown listener, but the component only existed where `useWindowChrome`
said the shell had removed the native frame — and that answer was
deliberately `false` on macOS (`platform !== 'macos'`), on the assumption
the native title bar still handles dragging there. It does not:
`TitleBarStyle::Overlay` keeps the traffic lights but hides the title, and
the webview runs the full height of the window, so there is no native strip
left to grab. macOS ended up with neither a native drag surface nor the
page's one.

**Fix, split by the two halves of the job:**

* `app/src/shell/useWindowChrome.ts` — now reports the platform alongside
  `present`, and `present` is true on every desktop platform: it answers
  "does the page own the title-bar gestures", which on this frameless build
  includes macOS.
* `app/src/shell/WindowControls.tsx` — the drag/double-click listeners
  install wherever `present` is true (macOS included); the three caption
  buttons render only where the frame was actually removed
  (`platform !== 'macos'`). macOS keeps its traffic lights and gains a
  movable window; Windows and Linux are byte-identical in behaviour.

## Verification this round

* `pnpm -r typecheck`: clean. `pnpm -r test`: 159/159.
* The macOS capture was exercised in a standalone harness (rustc, `-D
  warnings`, edition 2021): `native_dump`'s macOS branch verbatim against a
  simulated WKWebView store of the cookies a real sign-in leaves — all 13
  YouTube/Google session cookies captured, HttpOnly ones with their
  attributes, each once, no cross-origin leakage (the Spotify capture sees
  only `sp_dc`), signed-out and failed reads stay `None`; and the pre-fix
  `cookies_for_url` filter reproduced against the same store: empty, as
  reported. Harness in `scripts/macos_native_dump_harness.rs` of the fix
  bundle.

## Round — v1.9.5 follow-up: the island's morph, a pause that dismisses, truncated names, right-click, and a sidebar back button

### 1. The expand/collapse morph still read in two steps on macOS ("centre-left, then centre")

**Symptom:** expanding the pill first shuffled it sideways before it grew;
collapsing turned it mini at a spot left of centre and then walked it home —
the same report the Windows build had, even after the atomic `SetWindowPos`
there.

**Cause:** the Windows fix made the *Windows* geometry atomic, but macOS and
Linux kept the two-call path in `island.rs::apply_geometry`, whose comment
argued the calls run "one frame apart at worst". On macOS that is not what
happens: tao's `set_size` is an *asynchronous* `setContentSize:` and
`set_position` an asynchronous `setFrameOrigin:` — each dispatched separately
onto the main thread even when asked from it — and `setContentSize:` pins the
window's bottom-left origin, so the shrinking 432→216 window parked the mini
pill left of centre for a presented frame before `recenter` moved it. Two
painted frames, the exact reported two-step.

**Fix (`src-tauri/src/island.rs`, `src-tauri/Cargo.toml`):** every platform
now moves and resizes in one atomic primitive, with the card's top-centre
pinned (both states hang from the same top-centre point, so the pin is read
straight off the current frame — no monitor math, no scale factor):

* **macOS** — one NSWindow `setFrame:display:` message (objc2, already in
  Cargo.lock via tao/wry; pinned 0.6.5 / 0.3.2, no second copy), with
  `display: false` so the frame change coalesces into the same update cycle
  the webview's first grown frame rides in. Falls back to the old two-call
  path only if the raw handle is unavailable.
* **Linux** — `gdk_window_move_resize`, which carries position and size in
  one request (one X11 ConfigureWindow), instead of gtk's two server
  round-trips; same fallback. (gtk 0.18.2, what tauri already pins.)
* **Windows** — unchanged (the existing atomic `SetWindowPos`).

The page-side choreography (`island/main.ts::applyMode`: grow the window
first, morph the card second, shrink the window after the card settles) is
untouched — with the geometry atomic there is no intermediate frame left to
paint, so the card morphs as one motion in both directions.

### 2. Pausing dismissed the island

**Symptom:** pressing pause (from the island's own button or anywhere else)
made the island fold away entirely; it only came back when playback resumed.

**Cause:** `island/main.ts::reconcile` treated "not playing" the same as
"nothing to show": `snap.state !== 'playing'` went straight to `hidden`.

**Fix:** the policy now separates the two. A pause is *not* nothing — the
pill holds its ground with its play button, like the phone's does, and only
a track-less player (no title and no artwork), BitChord being focused, or a
right-click dismissal hides it. Song changes and seek drags still never
decide visibility. Position and remaining time hold where the pause left
them (`livePosition` already did).

### 3. Long titles and artists were cut off with "…"

**Symptom:** the mini pill and the expanded card truncated the song title
and artist with an ellipsis; there was no way to read the rest of the name.

**Fix (island.html, island.css, island/main.ts):** every text line is now a
clipping box over an inner `.tape` that carries the words. `main.ts` measures
the overflow whenever the words, the box, or the fonts change and, when there
is any, arms a slow ping-pong crawl (`tape-crawl` in `island.css`): out at a
steady 24 px/s, a rest long enough to read the ending, back, a rest at the
start. A name that fits never crawls; nothing is ever cut. The crawl distance
is re-measured on every trigger that can move the ground under it — the card's
morph (ResizeObserver during the width transition, plus one measure at the
morph's settle), a font swap (`loadingdone`, since `fonts.ready` can resolve
before the @font-face downloads even start, plus a short ladder as backstop),
each metadata sync, and a change of text — and an armed crawl whose distance
re-measures the same is left alone. Face measurements are guarded by the
card's current width, so the expanded card's lines are never measured against
the pill's 216px (or the reverse) while the other face is laid out. Reduced
motion keeps the overflow clipped and still.

### 4. Right-click: the stock webview menu everywhere; songs got nothing

**Symptom:** right-clicking anywhere opened the webview's own menu (Reload,
Inspect…) — no function, ugly over the glass — while right-clicking a song
offered nothing either.

**Fix:**

* `app/src/shell/AppShell.tsx` — one capture-phase `contextmenu` listener on
  the document preventDefaults the stock menu across every corner of the
  page (inputs and portals included, which bubble-phase handlers would miss).
* `app/src/components/Menu.tsx` — the menu's anchor may now be a *point* as
  well as an element, with a `below-start` placement: the panel opens at the
  cursor, clamped inside the window at both edges, and flips above the
  pointer (left edge kept at the pointer) when it doesn't fit below. Element
  anchors keep their exact old placements.
* `app/src/components/music/TrackMenu.tsx` — a `contextPoint` prop opens the
  same Play Next / Add to Queue / Favorite / Add to Playlist menu at the
  right-click's cursor instead of the ⋯ button.
* `app/src/components/music/TrackList.tsx`, `app/src/shell/UpNext.tsx` —
  album/playlist rows and Up Next rows open their ⋯ menu at the cursor on
  right-click (unavailable rows keep nothing).
* The dynamic island is untouched: it is its own webview, and its
  right-click-to-dismiss lives in `island/main.ts`, which this page's
  listeners can never reach.

### 5. No way back — the sidebar had no back button

**Fix (`app/src/shell/Sidebar.tsx`):** a chevron button rides the wordmark
row, styled like the settings button it mirrors. It calls `navigate(-1)`,
stays disabled while the app's history is still on its first page (react-
router stamps the depth into every entry's `history.state`), re-checks on
every navigation, and closes the mobile drawer when it fires.

## Verification this round

* `pnpm --filter @bitchord/app typecheck`: clean. `pnpm --filter @bitchord/app
  test`: 159/159. `vite build`: clean.
* The macOS geometry core was compiled verbatim in a standalone scratch crate
  against objc2 0.6.5 / objc2-foundation 0.3.2 for `aarch64-apple-darwin`:
  clean, no warnings (a full `cargo check --target aarch64-apple-darwin` of
  the shell itself needs a macOS SDK for `objc2-exception-helper`, which only
  a Mac build has). The Linux path's API usage was verified line-by-line
  against the gtk-rs 0.18.2 sources in the lock (`WidgetExt::window`,
  `gdk::Window::{position, width, move_resize}` — all inherent or prelude
  methods with the exact signatures used).
* The island page was exercised in a browser against the dev server: mini
  crawl distance converged to the measured overflow (±0px) with fonts loaded;
  paused kept the pill; a track-less snapshot hid it; right-click dismissed
  it and the dismissal held through paused syncs until a focus cycle re-armed
  it; expand measured the full card's lines at the settled width. The menu
  point-anchor placement was exercised the same way: at the cursor, clamped
  at the right edge (x = innerWidth − 185 − 8), flipped above near the bottom
  with the left edge kept at the pointer.

# Round — v1.9.7: the Linux launch panic in tao's event loop (event_loop.rs:459)

**Symptom:** on Linux the app died at launch with

    thread 'main' panicked at .../tao-0.37.2/src/platform_impl/linux/event_loop.rs:459:18:
    called `Option::unwrap()` on a `None` value

**Where that line is:** tao's handler for `WindowRequest::CursorIgnoreEvents`
— the request `set_ignore_cursor_events` sends. The `true` branch is

```rust
window
  .window()   // the gtk window's GdkWindow — Option
  .unwrap()   // line 459: unconditional
  .input_shape_combine_region(&empty_region, 0, 0);
```

`window()` is `WidgetExt::window`, the **GdkWindow**, and it is `None` until
the gtk window is *realized* — which, for a toplevel, happens when it is
first shown. There is no guard.

**How the app hit it:** the island is created hidden and click-through —
`island.rs::create` built the window with `.visible(false)` and called
`set_ignore_cursor_events(true)` **before** `show()` (the old line 117 ahead
of line 118). Windows and macOS accept the request on an unshown window
(`WS_EX_TRANSPARENT` / `setIgnoresMouseEvents:` both do), so the order never
hurt there. On Linux the queued `CursorIgnoreEvents` request was processed
while the window had never been shown: `window()` answered `None`, the
unwrap fired inside the main thread's event loop, and the process died. The
island is on by default (`None` = on in the store), so this was a launch
crash for every Linux user. The v1.9.6 round's Linux change — the atomic
`gdk_window_move_resize` geometry — is a different code path entirely
(Option-mapped, no unwrap) and neither caused nor fixed this.

**Fix (`src-tauri/src/island.rs`):** on Linux, the click-through request is
never sent for an unrealized window.

* New `arm_click_through(app, ignore)` — hops to the main thread
  (`run_on_main_thread`) and attaches a glib idle that re-checks the window
  on every pass: island gone → stop; `gtk_window().window()` still `None` →
  retry (bounded at 250 passes, then `log::warn!` and give-up — a
  click-eating pill is better than a dead app, and in practice realization
  lands on the first or second pass because every pass lets the main loop
  process the show queued just before); realized → send the real
  `set_ignore_cursor_events` and stop.
* `create()` — Linux now shows the window first and then arms click-through.
  The other platforms keep the old order, which works there and leaves zero
  unclickable gap.
* `set_click_through()` — the page's `island_overlay` flips ride the same
  arm on Linux, which also covers a flip landing before the first show has
  been processed; other platforms unchanged.

The island page paints nothing until its first sync (its CSS starts fully
transparent), so the one-or-two-idle gap between "on screen" and
"click-through armed" is unobservable.

**Why not patch tao:** guarding the unwrap upstream is a one-line `[patch]`
fork of tao 0.37.2 — a vendored copy, a rewritten lock, and a maintenance
tax on every future `cargo update`. The app-side arm is a `cfg(linux)`
function that leaves the lock untouched and makes the panic unreachable, and
`set_ignore_cursor_events` stays callable as before for every window that
*has* been shown.

**Verification this round**

* No full Linux compile is possible on this host (no root; pkg-config finds
  neither `gtk+-3.0` nor `webkit2gtk-4.1`), so the fix was verified the way
  the earlier platform-specific rounds were:
  * **Signature line-by-line against the locked crate sources** — tauri
    2.12.5 `run_on_main_thread<F: FnOnce() + Send + 'static>` (app.rs:496),
    `WebviewWindow::gtk_window() -> Result<gtk::ApplicationWindow>`
    (webview_window.rs:1980, Linux-gated); gtk 0.18.2 re-exports glib
    (`gtk/src/lib.rs:21 pub use glib;` — the tauri-pinned glib 0.18.5, no
    new dependency and no second copy in the lock) and
    `WidgetExt::window(&self) -> Option<gdk::Window>` (widget.rs:1100,
    owned — `.is_some()` never lets the borrow escape); glib 0.18.5
    `idle_add_local<F: FnMut() -> ControlFlow + 'static>` (source.rs:337 —
    **no Send bound**, so the `Cell` pass counter is legal, but it must run
    on the main-context thread, hence the `run_on_main_thread` hop), the
    `ControlFlow` enum (control_flow.rs:14), and `SourceId` is not
    `#[must_use]` (source.rs:21).
  * **Behaviour + structure in a standalone scratch compile** (`rustc -D
    warnings --edition 2021`, stubs mirroring those exact signatures; kept
    in `scripts/island_arm_scratch/` of the fix bundle): the verbatim helper
    body compiles clean, the `FnOnce + Send` outer closure carries only the
    `Send` `AppHandle` (the non-Send `Cell` is created *inside* it), and the
    four scenarios pass — window gone: breaks with no request; realized:
    exactly one `ignore=true` request; never realized: the 250-pass give-up
    logs and sends nothing, where the old code panicked on the first pass;
    `ignore=false` flips forward unchanged.
  * **No sibling unwrap on the island's request path:** tao's `Position`
    (`window.move_`), `Size` (`window.resize`), `Visible`
    (`show_all`/`hide`), `Resizable`, `Closable`, `Minimized`, `Maximized`
    and `CursorIgnoreEvents(false)` handlers all take an unrealized window
    without unwrapping — recenter, show and the geometry writes cannot trip
    the same failure mode.
* Front end untouched by this round (no TS/CSS change).


---

# Round — v1.9.7-mac: the link previews, the two players, the traffic lights, the vanishing sign-in windows, the Windows transport, the selections, and the backdrop (six reports, one macOS thread through them)

Six reports from the v1.9.7 beta, three of them macOS-only, one Windows-only,
two everywhere — plus the red screen. Each is reproduced from first principles
against the locked crate sources (tauri 2.12.5, wry 0.57.1, tao 0.37.2,
souvlaki 0.8.3, objc2-app-kit 0.3.2) before being fixed, in the rounds' usual
way.

## 1. Text selection is off everywhere but where text is typed

A drag across a cover or a lyric painted a selection, and a double-click
painted one over a control's label. The engine's default — select on drag and
on double-click — belongs to documents, not to the player:

* `app/src/styles/global.css` — `body` gets `user-select: none` (with the
  `-webkit-` alias for WKWebView), and the fields that exist to be typed into
  get `user-select: text` back: `input`, `textarea`, `select`, and anything
  editable via `contenteditable`. Nothing in the app reads
  `window.getSelection()` (checked), so nothing that worked before stops
  working; the `::selection` rule stays, in case a field paints one.

## 2. macOS: the "BitChord / tauri://localhost/home" tooltip is gone

The hover tooltip over every link is WKWebView's link preview, and
JavaScript cannot cancel it: WebKit computes the tooltip from the hit-tested
element, not from any event the page can prevent. What the page *can* do is
not have the `href`:

* `src-tauri/shim/desktop.js` — a fifth section, macOS-only (the platform
  lands with the shim's own `desktop_info` call, and the observer catches
  everything rendered before it): a `MutationObserver` over
  `attributeFilter: ['href']` and subtree childList moves every anchor's
  `href` into `data-href` and keeps the element tabbable with
  `tabindex="0"`. React Router's `Link` is a click handler that never reads
  the attribute back (`shouldProcessLinkClick` checks button, target and
  modifiers — verified against the installed react-router 8.4.0
  `lib/dom/dom.js`), so navigation is untouched; keyboard focus still
  reaches the link, and `PrefetchLink`'s focus-prefetch still fires. The
  `_blank` interception gains the `data-href` fallback, so external links
  still route to `open_external`.
* Windows and Linux are untouched: WebView2 and WebKitGTK hang no tooltip on
  hrefs.

## 3. macOS: one player, not two

The system media panel showed two cards for one song: a bare "BitChord" row
with a replay button, and the track's own row ("FEAR" / NF, pause and next).
The first card was souvlaki's registration (media.rs), the second the
WKWebView's own — WebKit registers the playing element with Now Playing by
itself, and `MPNowPlayingInfoCenter`'s default entry plus the webview's
session are two players in one process.

* `src-tauri/src/media.rs` — `attach` on macOS registers nothing now (the
  webview's card is the one that shows the track, the artist and the working
  buttons). The souvlaki import, the controls thread-local, `handle_event`
  and the update block are `cfg`'d to the platforms that construct controls
  (Linux, Windows). The tray and the island still read the snapshot from
  `media_update`, which flows exactly as before; the snapshot logic is
  untouched.

## 4. macOS: the traffic lights no longer sit on the player's close button

The full-screen player's close button (`top-4 left-5`) is exactly where
`TitleBarStyle::Overlay` hangs the traffic lights, so the white X sat half
under the red light. Windows and Linux already had this answer (Round 8: the
page's own bar stands down in the player, and there is nothing native left to
stand down); macOS now gets the same one by hiding the lights:

* `src-tauri/src/chrome.rs` (new) — `set_traffic_lights_visible(app, visible)`
  hops to the main thread (NSWindow is MainThreadOnly) and flips
  `-[NSButton setHidden:]` on the three standard window buttons, the same
  buttons tao's `inset_traffic_lights` reaches (`NSWindow.standardWindowButton`
  → `Option<Retained<NSButton>>`, per objc2-app-kit 0.3.2's generated
  NSWindow.rs:1645 and NSView.rs:255). A hidden flag, not a style-mask
  change: the window keeps its frame, shadow and resizeability, and AppKit's
  position-only resets (title changes, fullscreen transitions) do not
  resurrect hidden buttons.
* `src-tauri/src/commands.rs` — the async `set_traffic_lights_visible`
  command; registered in `lib.rs`'s handler, granted through
  `permissions/desktop-commands.toml` (`allow-set-traffic-lights-visible`,
  the hyphenated-identifier route the other commands use) and
  `capabilities/main.json`. On Windows and Linux it is a no-op.
* `app/src/shell/WindowControls.tsx` — an effect keyed on
  `{present, platform, fullScreen}` presses the command: lights hidden for as
  long as the player is up, restored the moment it closes (Escape and the
  player's own X both leave the player, so nothing is trapped).
* `src-tauri/Cargo.toml` — `objc2-app-kit = "0.3.2"` on macOS: the same
  version the lock already pins (wry pulls it, with NSWindow/NSView/NSButton
  features enabled), so this adds an edge to the graph, not a second copy.
  Cargo reconciles `Cargo.lock`'s dependency lists on the next build.

## 5. macOS: the sign-in windows stopped vanishing after ~1.5 s

Discord *and* Google sign-in windows closed on their own within about a
second and a half of opening — before the page had finished loading — making
Discord integration and YouTube sign-in impossible. Reproduced end to end:

* The capture thread (agent.rs) polls the agent window's native cookie store
  every 1.5 s, for YouTube and Spotify origins **regardless of which page
  the window is on**. On macOS that store is WKWebView's default data store:
  persistent on disk, shared by every agent window. A user who signed into
  YouTube once (round 9 made that work) carries the whole session in it
  forever.
* So opening the Discord sign-in window produced a YouTube report 1.5 s
  later — a session nobody navigated to. The close guard only checks that
  the core verified the session and that it differs from the window's
  baseline, and the baseline was the **core jar's fingerprint**, taken from
  `core.yt.cookie_dump()`. Jar and webview store never fingerprint equal:
  the jar re-serialises what it absorbed, and YouTube rotates
  `__Secure-1PSIDTS` on the core's own API calls, so at least one pair
  differed at every open. `fresh.signed_in && !unchanged` → close. The same
  sequence fired for the Google window (the page loads signed in, the shim's
  `document.cookie` reports, the jar never matches). Both windows died
  before painting.
* The scratch harness (`scripts/agent_scratch/`, kept in the fix bundle)
  reproduces it: `fingerprint(core jar) != fingerprint(native dump)` on the
  same session — the old guard fires — while every scenario the new guard
  must hold, holds (below).

Two fixes, both in `src-tauri/src/agent.rs`:

* **Harvest the service the window is on.** The capture reads
  `window.url()` (tauri 2.12.5 `WebviewWindow::url`, webview_window.rs:2506)
  and polls a service's origins only while the page's host belongs to that
  service — `youtube.com`/`google.com` for YouTube (the flow hops both),
  `spotify.com` for Spotify. A Discord sign-in window can never produce a
  YouTube report again, whatever the persistent store holds.
* **Baseline the window's own store, not the core jar.** `open_signin` calls
  `note_session_at_open` (both the reuse and the new-build paths) once the
  window exists: the native dump for the kind's origins is taken and its
  YouTube *identity* cookies (`SAPISID`, `__Secure-1PSID`, `__Secure-3PSID`,
  `SID`) become the baseline as a name→value map. The close check is
  `identity_unchanged`: every identity cookie the report carries must match
  the baseline's value. The report may carry FEWER of them than the baseline
  (the shim's `document.cookie` cannot see HttpOnly), which is the same
  session through a smaller window, not a new one. A `__Secure-1PSIDTS`
  rotation is not in the identity set — churn, not a sign-in — so it never
  closes the window. An account switch, or a first sign-in on an empty
  baseline, changes at least one value and closes after the core verified.
  Spotify's baseline is now its own store's `sp_dc` (same principle);
  Discord's token stays core-sourced (localStorage is only reachable from
  the page, which is what reports it).

`AgentState::snapshot` now produces empty baselines — the real ones are
per-open; nothing can know what a window that does not exist yet holds.

## 6. Windows: the playback controls are on the system level

The volume flyout, the lock screen and the hardware media keys never showed
BitChord. The previous rounds' note — "Windows needs nothing here,
WebView2's own media session already owns SMTC" — was the assumption that
failed: the shim forwarded `navigator.mediaSession` calls into the webview's
native session on Windows, but that session does not surface as the system's
System Media Transport Controls, and where a runtime does surface it, it
would be a second player beside the shell's own.

* `src-tauri/src/media.rs` — `attach` on Windows constructs souvlaki's
  controls with the main window's HWND
  (`WebviewWindow::hwnd()`, webview_window.rs:1966; `HWND.0` an isize — the
  same access island.rs's `SetWindowPos` already shipped). souvlaki 0.8.3's
  Windows platform is `SystemMediaTransportControlsInterop::GetForWindow`
  with `SetIsEnabled`/`SetIsPlayEnabled`/… and a `ButtonPressed` handler —
  read from the crate source (platform/windows/mod.rs), and its own
  `examples/window.rs` runs it directly on a winit event-loop thread, the
  same STA message pump tao runs. attach happens in `setup`, on the main
  thread, where the interop object lives; button presses arrive through that
  thread's pump and feed the existing `handle_event` →
  `window.__BitChordMediaAction` path the tray and MPRIS already use.
* `apply_update`'s controls block is `cfg(any(linux, windows))`: every
  metadata update now sets the SMTC music properties and the thumbnail
  (`CreateFromUri` for https covers), every playback/position update the
  status and the timeline — play/pause/next/previous/seek from the volume
  flyout and the lock screen drive the page.
* `src-tauri/src/lib.rs` — `media::attach` now runs on Windows too (it was
  `cfg(any(linux, macos))`, so the Windows build never even called it).
* `src-tauri/shim/desktop.js` — the Windows passthrough to
  `navigator.mediaSession` is removed (it never reached the system level,
  it skipped every call that landed before `desktop_info` resolved, and it
  never forwarded `setPositionState` — a half-wired session that could only
  ever duplicate the shell's). souvlaki is the single SMTC owner; the shim's
  header documents this.

## 7. The full-screen player's backdrop: the Kotlin system, ported

The player's background was one flat colour on red covers — the old web
backdrop asked a quantiser what the artwork is "about" and painted four
blobs of the answer, which for a nine-tenths-dark sleeve is one loud colour
over the whole screen. The Kotlin app replaced exactly this approach, for
exactly this reason (ArtworkMeshBackdrop.kt's own docstring names the red
screen), with a two-layer system; it is ported here file for file:

* `app/src/lib/artworkBackdrop.ts` — the mesh: the artwork decoded at
  MESH_PX (120, a CDN-cached variant), averaged into a 6×6 grid of cell
  means **flipped bottom-up** (row 0 is the sleeve's own bottom edge), each
  cell lifted the Kotlin lift — saturation ×1.12, lightness floored at
  0.045, via ColorUtils.RGBToHSL/HSLToColor ported as written "so the
  phone's numbers do not move" — the rows below the seam cyclic-shifted
  sideways by a seeded java.util.Random (BigInt LCG, `String.hashCode` for
  the seed; mirrored half the time), and the grid resampled to 32×32 with
  the smoothstep curve that flattens at every control point. A mean per
  cell, not a quantised swatch: nine-tenths black stays nine-tenths black.
  Plus the second layer: the same decode box-blurred 3 passes at radius
  min(w,h)/12 (the sliding-window Kotlin pass), cached by URL (64 meshes, 8
  blurs), with retry (4 attempts, 1.5 s apart) matching the Kotlin reader.
* `app/src/shell/ArtworkBackdrop.tsx` — both layers stay mounted for the
  whole song: the mesh under its 0.06→0.30 scrim (still, never animated —
  the Kotlin mesh retired the blob mesh that paid a permanent 60 Hz blur for
  a wallpaper), and the full-cover blur over its 0.34→0.48→0.64 legibility
  scrim, cropped across the surface (`object-fit: cover` is Kotlin's
  `ContentScale.Crop`). The mesh crossfades between tracks (900 ms, the
  Kotlin MESH_FADE_MS, FastOutSlowIn) and holds what is on screen until the
  new track's decode lands — wrong colour for one decode is the trade;
  absent colour was the flicker. The blur fades in over 320 ms when a decode
  arrives; a cached cover swaps without a fade; while it is up, the mesh
  layer crossfades out (Kotlin's `alpha = 1f - fullArtworkBackdropAlpha`,
  360 ms). The floor before any artwork is read is #121212, the Kotlin
  FallbackBackdrop. The mesh's 32px blur is overscaled by 6% so the blur's
  edge falloff stays outside the viewport.
* `app/src/shell/FullScreenPlayer.tsx` — `<MeshBackdrop>` is replaced by
  `<ArtworkBackdrop artworkUrl>`; the old component file stays in the tree
  untouched, unused (no file was removed).

## 8. The player's corner buttons and the scrollbar, from the Kotlin app

* The corner buttons are the Kotlin player's `LandscapeChromeButton`:
  32 px disc, 18 px glyph, an icon at two-thirds white that lights to full
  white over the faintest disc on hover — the on-dark tokens already spell
  those colours — and the port adds what was missing: the press squeeze
  (`active:scale-90`) riding the same 100 ms transition. The close glyph is
  18 px like the rest.
* `app/src/styles/global.css` — the scrollbar is the Kotlin app's
  `AppleScrollbar` (AppleScrollbar.jvm.kt): a 7 px hairline at
  `rgb(171 171 178 / 0.18)` that lights to `0.55` when the pointer comes for
  it, thumb never shorter than 26 px, circle-capped, over an invisible
  track. WebKit pseudo-elements only, and deliberately without the standard
  `scrollbar-color` pair: a set `scrollbar-width` switches Chromium
  (WebView2) to its native thin bar — rectangular, unrounded, no hover —
  and disables the pseudo-element styles outright. The shelves' and the
  sidebar's hide-outright utilities still outrank everything here;
  `UpNext`'s `[scrollbar-width:thin]` class is dropped for the same reason
  (it would have selected the native bar and defeated the Apple one).

## Verification this round

* `pnpm -r typecheck`: clean. `pnpm -r test`: 174/174 (16 new tests:
  `artworkBackdrop.test.ts` — the java.util.Random LCG and
  String.hashCode against independently computed reference values, the
  flip-and-proportions mesh properties, the seam row, seeded rotation,
  smoothstep control points, the HSL round-trip, the blur's bounded reach,
  and the nine-tenths-black cover keeping nine-tenths of the mesh dark).
  `pnpm --filter @bitchord/app build` (tsc + Vite production): clean.
* Rust: no full compile is possible on this host (no root; pkg-config finds
  neither gtk/webkit nor dbus), so the changed shell code was verified the
  earlier rounds' way:
  * **media.rs compiles verbatim** against a tauri- and souvlaki-shaped
    harness whose signatures were taken from the locked crate sources
    (tauri 2.12.5 webview_window.rs:1954/1966/2506/2735, app.rs
    `run_on_main_thread<F: FnOnce() + Send + 'static>`; souvlaki 0.8.3
    lib.rs/config.rs/platform\*) — the Linux branch for real, the Windows
    branch's `hwnd() -> Result<HWND>` + `HWND.0` against the crate source
    and island.rs's shipped precedent. Kept in
    `scripts/media_scratch/` of the fix bundle.
  * **agent.rs's new guard compiles and behaves** in a standalone harness
    (`scripts/agent_scratch/`): the verbatim `identity_cookies`,
    `identity_unchanged`, `fingerprint` and `on_service_page` against eight
    scenarios — the shim's subset report, the identical native dump, a
    PSIDTS rotation (all unchanged → window stays), the old fingerprint
    guard firing on the same session (the bug, reproduced), an account
    switch, an empty baseline (changed → close after verify), the Spotify
    compare, and the page gating that keeps a foreign session out of a
    Discord window's report stream.
  * **chrome.rs's macOS branch** is signature-checked against
    objc2-app-kit 0.3.2's generated sources: `NSWindow::standardWindowButton
    (&self, NSWindowButton) -> Option<Retained<NSButton>>` (NSWindow.rs:1645),
    `NSView::setHidden(&self, bool)` (NSView.rs:255, reachable through
    NSButton's Deref chain), `NSWindowButton::CloseButton/MiniaturizeButton/
    ZoomButton` (NSWindow.rs:322-329), and `WebviewWindow::ns_window() ->
    Result<*mut c_void>` (tauri webview_window.rs:1954). The non-macOS stub
    compiles in the same harness.
* The Rust toolchain on this host is rustc 1.99.1; `tauri 2.12.5` declares
  `rust-version = "1.90"`, and no code this round adds uses anything newer
  than what the tree already shipped (let-else, cfg attrs on let, inline
  format args).
