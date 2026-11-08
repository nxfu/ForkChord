/**
 * BitChord desktop shim — injected into the main window before the app loads.
 *
 * This file is the reason the web frontend runs byte-for-byte unchanged:
 *
 *  1. Bridge relay. The page already speaks `@bitchord/shared/bridge` over
 *     `window.postMessage` (its Firefox transport). Here the "extension" is
 *     the Rust core: requests go over Tauri IPC, responses come back as the
 *     same envelopes, and the core's pushes arrive through a function call
 *     (`__BitChordBridgeEvent`) that this relay re-posts as bridge events.
 *  2. Media session. `navigator.mediaSession` is replaced with a shim wired
 *     to the Rust side so the tray, the Dynamic Island and the platform
 *     players all see the track. The OS-level players are the shell's own:
 *     MPRIS on Linux and SMTC on Windows (souvlaki in media.rs), fed by the
 *     same throttled `media_update` stream the tray reads. The shim does not
 *     forward anything to the webview's native session — a passthrough to
 *     WebView2's own media session used to stand in for SMTC on Windows, but
 *     that session never surfaces at system level, and where the runtime
 *     does surface it, forwarding would put a second player beside the
 *     shell's. The shim installs synchronously — the app's first
 *     `setActionHandler` call must land on it.
 *  3. External links. `window.open` and target=_blank anchors go to the
 *     shell: sign-in origins open inside the app's agent window, everything
 *     else opens in the system browser.
 *  4. The page's network. The web app fetches YouTube from the page itself,
 *     and in the browser that worked only because the extension's
 *     declarativeNetRequest rules dressed the requests in YouTube's headers
 *     and answered them with Access-Control-Allow-Origin — youtubei answers
 *     CORS with nothing at all, and googlevideo serves the YouTube origin
 *     alone. A webview has no DNR, so this shim claims those fetches and
 *     hands them to the Rust core, which sends them under the same rules
 *     (`web/extension/lib/rules.ts`, mirrored in core/src/http.rs). Anything
 *     else fetches natively first, exactly as on the site, and only a
 *     network-error refusal (a cross-origin host that answers no CORS — the
 *     Listen Together server) retries through the core.
 *  5. No link previews on macOS. WKWebView hangs a tooltip off every anchor
 *     with an href — "BitChord / tauri://localhost/home" under the cursor —
 *     and JavaScript cannot turn that off: the tooltip is computed from the
 *     hit-tested element, not from any event the page can cancel. What the
 *     page CAN do is not have the href. On macOS the shim moves every
 *     anchor's href into data-href (and keeps the element tabbable), which
 *     kills the tooltip; navigation is untouched, because React Router's
 *     links are click handlers that never read the attribute, and the
 *     target=_blank interception below reads data-href as its fallback.
 */
(function () {
  'use strict';
  var internals = window.__TAURI_INTERNALS__;
  if (!internals || window.__BITCHORD_DESKTOP__) return;
  var invoke = function (cmd, args) {
    return internals.invoke(cmd, args);
  };
  var desktop = { shimInstalled: true, platform: 'linux', version: '' };
  window.__BITCHORD_DESKTOP__ = desktop;

  invoke('desktop_info')
    .then(function (info) {
      desktop.platform = info.platform;
      desktop.version = info.version;
    })
    .catch(function () {
      /* stays 'linux': full replacement, which is correct on this OS too */
    });

  // ---- 1. The bridge relay -------------------------------------------------

  var CHANNEL = 'bitchord-bridge';
  var isBridgeRequest = function (v) {
    return (
      v && typeof v === 'object' &&
      v.channel === CHANNEL &&
      v.direction === 'to-extension' &&
      typeof v.id === 'string' &&
      typeof v.method === 'string'
    );
  };

  window.addEventListener('message', function (event) {
    // The page's relay transport answers only its own window (client.ts).
    if (event.source !== window || event.origin !== window.location.origin) return;
    var data = event.data;
    if (!isBridgeRequest(data)) return;
    invoke('bridge_call', { request: data })
      .then(function (response) {
        window.postMessage(response, window.location.origin);
      })
      .catch(function (error) {
        window.postMessage(
          {
            channel: CHANNEL,
            protocol: 1,
            direction: 'to-page',
            id: data.id,
            ok: false,
            error: { code: 'internal', message: String(error && error.message ? error.message : error) },
          },
          window.location.origin
        );
      });
  });

  /** The core pushes auth/Discord state changes through here (called by eval). */
  window.__BitChordBridgeEvent = function (payloadJson) {
    try {
      var payload = JSON.parse(payloadJson);
      window.postMessage({ channel: CHANNEL, direction: 'event', payload: payload }, window.location.origin);
    } catch (error) {
      /* a malformed push is dropped, like any other page message */
    }
  };

  // ---- 2. The media session shim -------------------------------------------

  var mediaHandlers = {};
  var lastPositionSent = 0;
  var lastPositionValue = 0;
  var lastDurationSent = 0;
  var lastRateSent = 1;
  var lastMetaSent = '';
  var lastStateSent = '';

  if (typeof window.MediaMetadata !== 'function') {
    // WebKitGTK ships no MediaMetadata; the app constructs one on every track.
    window.MediaMetadata = function (init) {
      init = init || {};
      this.title = init.title || '';
      this.artist = init.artist || '';
      this.album = init.album || '';
      this.artwork = init.artwork || [];
    };
  }

  var sendMedia = function (payload) {
    invoke('media_update', { update: payload }).catch(function () {});
  };

  function largestArtwork(entries) {
    if (!entries || !entries.length) return null;
    var best = null;
    for (var i = 0; i < entries.length; i++) {
      var src = entries[i] && entries[i].src;
      if (!src) continue;
      var size = parseInt((entries[i].sizes || '0').split('x')[0], 10) || 0;
      if (!best || size >= best.size) best = { src: src, size: size };
    }
    return best ? best.src : null;
  }

  (function installMediaSession() {
    var session = {
      /** The page assigns a MediaMetadata (or null). */
      set metadata(value) {
        var payload = null;
        if (value) {
          payload = {
            title: value.title || '',
            artist: value.artist || '',
            album: value.album || '',
            artworkUrl: largestArtwork(value.artwork),
          };
        }
        var key = payload ? JSON.stringify(payload) : '';
        if (key !== lastMetaSent) {
          lastMetaSent = key;
          sendMedia({ kind: 'metadata', metadata: payload });
          // A track switch re-arms the position throttle: the new track's own
          // setPositionState (its durationchange / seeked) must never be the
          // update that gets swallowed, or the island and the desktop's media
          // UI keep riding the previous track's timeline.
          lastPositionSent = 0;
        }
      },
      get metadata() {
        return null;
      },
      /** The page assigns 'playing' | 'paused' (and reads it). */
      set playbackState(state) {
        if (state !== lastStateSent) {
          lastStateSent = state;
          sendMedia({ kind: 'playback', state: state });
        }
      },
      get playbackState() {
        return lastStateSent;
      },
      setActionHandler: function (action, handler) {
        mediaHandlers[action] = handler;
      },
      setPositionState: function (state) {
        if (!state) return;
        var now = Date.now();
        var duration = Number(state.duration) || 0;
        var position = Number(state.position) || 0;
        var rate = Number(state.playbackRate) || 1;
        // Where the shell's snapshot should be by now, had playback been
        // steady since the last update we sent.
        var projected = lastPositionValue;
        if (lastStateSent === 'playing' && lastPositionSent > 0) {
          projected += ((now - lastPositionSent) / 1000) * lastRateSent;
        }
        // Switching tracks (a duration the shell has never seen) and seeks
        // (position off the projected line) must land at once: a swallowed
        // update here desynced the island and the media UI until the next
        // user-initiated seek. Steady playback stays throttled to ~900 ms;
        // the next event corrects it.
        var drifted = Math.abs(position - projected) > 1.25;
        if (now - lastPositionSent < 900 && duration === lastDurationSent && !drifted) return;
        lastPositionSent = now;
        lastPositionValue = position;
        lastDurationSent = duration;
        lastRateSent = rate;
        sendMedia({ kind: 'position', duration: duration, position: position, rate: rate });
      },
    };

    try {
      Object.defineProperty(navigator, 'mediaSession', {
        configurable: true,
        get: function () {
          return session;
        },
      });
    } catch (error) {
      return;
    }

    /**
     * The shell plays the user's buttons here: the tray, MPRIS, the macOS
     * Now Playing widget. `arg` carries a seek time or offset in seconds.
     */
    window.__BitChordMediaAction = function (action, arg) {
      var handler = mediaHandlers[action];
      if (!handler) return;
      try {
        if (action === 'seekto') handler({ seekTime: typeof arg === 'number' ? arg : undefined });
        else if (action === 'seekforward') handler({ seekOffset: typeof arg === 'number' ? arg : 10 });
        else if (action === 'seekbackward') handler({ seekOffset: typeof arg === 'number' ? arg : 10 });
        else handler();
      } catch (error) {
        /* A dead handler must not take the shell down. */
      }
    };
  })();

  // ---- 3. External links ----------------------------------------------------

  var originalOpen = window.open ? window.open.bind(window) : null;
  window.open = function (url, target, features) {
    if (url && /^https?:/i.test(String(url))) {
      invoke('open_external', { url: String(url) }).catch(function () {});
      return null;
    }
    return originalOpen ? originalOpen(url, target, features) : null;
  };

  document.addEventListener(
    'click',
    function (event) {
      var node = event.target;
      while (node && node !== document) {
        if (node.nodeType === 1 && node.tagName === 'A') break;
        node = node.parentNode;
      }
      if (!node || node === document || node.nodeType !== 1) return;
      if ((node.getAttribute('target') || '').toLowerCase() !== '_blank') return;
      // The href may have moved to data-href (§5, the macOS link preview):
      // both spellings route to the shell.
      var href = node.getAttribute('href') || node.getAttribute('data-href');
      if (href && /^https?:/i.test(href)) {
        event.preventDefault();
        invoke('open_external', { url: href }).catch(function () {});
      }
    },
    true
  );

  // ---- 4. The page's network -------------------------------------------------

  var originalFetch = typeof fetch === 'function' ? fetch.bind(window) : null;

  /**
   * The rules of `web/extension/lib/rules.ts::youtubeRules`, as a matcher.
   * Returns the origin to impersonate (`xorigin` adds X-Origin, the
   * InnerTube-only header), `null` for a matched rule with no header
   * changes, or undefined when the fetch must stay with the browser. jnn is
   * deliberately unmatched, like in the rules: BotGuard's Create allows any
   * origin and refuses a rewritten YouTube one.
   */
  var pageRuleFor = function (url) {
    if (url.protocol !== 'https:' && url.protocol !== 'http:') return undefined;
    var host = url.hostname;
    var path = url.pathname;
    if ((host === 'music.youtube.com' || host === 'www.youtube.com') && path.indexOf('/youtubei/') === 0) {
      return {
        origin: host === 'music.youtube.com' ? 'https://music.youtube.com' : 'https://www.youtube.com',
        xorigin: true,
      };
    }
    if (host === 'www.youtube.com' && (path === '/sw.js_data' || path === '/iframe_api' || path.indexOf('/s/player/') === 0)) {
      return { origin: null, xorigin: false };
    }
    if (host === 'googlevideo.com' || host.slice(0 - '.googlevideo.com'.length) === '.googlevideo.com') {
      // The rule was the wildcard ||googlevideo.com/*c=VISIONOS: its direct
      // URLs carry no Origin or Referer at all, or googlevideo cuts the
      // stream off mid-track with a 403.
      if (url.href.indexOf('c=VISIONOS') !== -1) return { origin: null, xorigin: false };
      return { origin: 'https://www.youtube.com', xorigin: false };
    }
    return undefined;
  };

  var toBase64 = function (bytes) {
    var binary = '';
    var chunk = 0x8000;
    for (var i = 0; i < bytes.length; i += chunk) {
      binary += String.fromCharCode.apply(null, bytes.subarray(i, i + chunk));
    }
    return btoa(binary);
  };

  var base64ToBytes = function (value) {
    var binary = atob(value);
    var bytes = new Uint8Array(binary.length);
    for (var i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return bytes;
  };

  var abortError = function () {
    return new DOMException('The user aborted a request.', 'AbortError');
  };

  /** Rejects [pending] the moment [signal] aborts, like a real fetch would. */
  var guardAbort = function (pending, signal) {
    if (!signal) return pending;
    if (signal.aborted) return Promise.reject(abortError());
    return new Promise(function (resolve, reject) {
      var onAbort = function () {
        reject(abortError());
      };
      signal.addEventListener('abort', onAbort, { once: true });
      pending.then(
        function (value) {
          signal.removeEventListener('abort', onAbort);
          resolve(value);
        },
        function (error) {
          signal.removeEventListener('abort', onAbort);
          reject(error);
        }
      );
    });
  };

  /** The core's HttpResponse, as a Response the page can read like any other. */
  var buildPageResponse = function (response) {
    var headers = new Headers();
    var skipped = { 'content-encoding': 1, 'content-length': 1, 'transfer-encoding': 1, connection: 1, 'keep-alive': 1 };
    for (var key in response.headers) {
      if (!Object.prototype.hasOwnProperty.call(response.headers, key)) continue;
      if (skipped[key.toLowerCase()]) continue; // reqwest decoded the body; the stale metadata would lie
      try {
        headers.set(key, response.headers[key]);
      } catch (error) {
        /* an invalid name is dropped, like a browser would */
      }
    }
    var body = null;
    if (response.status !== 204 && response.status !== 304) {
      body = response.bodyEncoding === 'base64' ? base64ToBytes(response.body) : response.body;
    }
    return new Response(body, { status: response.status, headers: headers });
  };

  var dispatchPageFetch = function (payload, signal, fallback) {
    var pending = internals
      .invoke('page_fetch', { request: payload, fallback: fallback })
      .then(buildPageResponse, function (error) {
        // The shape a failed fetch rejects with.
        throw new TypeError(String(error && error.message ? error.message : error || 'Failed to fetch'));
      });
    return guardAbort(pending, signal);
  };

  var pageFetch = function (url, input, init, rule) {
    var request = input instanceof Request ? input : null;
    var method = String((init && init.method) || (request && request.method) || 'GET').toUpperCase();

    var headerMap = {};
    var source = null;
    try {
      if (init && init.headers) source = new Headers(init.headers);
      else if (request) source = request.headers;
    } catch (error) {
      source = null;
    }
    if (source) source.forEach(function (value, key) {
      headerMap[key] = value;
    });

    var rawBody = init && init.body != null ? init.body : null;
    var preparing;
    if (rawBody == null && request && method !== 'GET' && method !== 'HEAD') {
      preparing = request.text().catch(function () {
        return null;
      });
    } else {
      preparing = Promise.resolve(rawBody);
    }

    var signal = (init && init.signal) || (request && request.signal) || null;
    var fallback = !!rule.fallback;

    return preparing.then(function (raw) {
      var payload = { url: url.href, method: method, headers: headerMap, body: null, body_encoding: null };
      if (raw != null) {
        if (typeof raw === 'string') {
          payload.body = raw;
        } else if (raw instanceof ArrayBuffer) {
          payload.body = toBase64(new Uint8Array(raw));
          payload.body_encoding = 'base64';
        } else if (ArrayBuffer.isView(raw)) {
          payload.body = toBase64(new Uint8Array(raw.buffer, raw.byteOffset, raw.byteLength));
          payload.body_encoding = 'base64';
        } else if (typeof Blob !== 'undefined' && raw instanceof Blob) {
          return raw.arrayBuffer().then(function (buffer) {
            payload.body = toBase64(new Uint8Array(buffer));
            payload.body_encoding = 'base64';
            return dispatchPageFetch(payload, signal, fallback);
          });
        } else {
          // URLSearchParams, FormData, a stream: nothing the matched
          // endpoints send. Leave the request with the browser.
          return originalFetch(input, init);
        }
      }
      return dispatchPageFetch(payload, signal, fallback);
    });
  };

  if (originalFetch) {
    window.fetch = function (input, init) {
      var url;
      try {
        url = input instanceof Request ? new URL(input.url) : new URL(String(input), window.location.href);
      } catch (error) {
        return originalFetch(input, init);
      }
      var rule = pageRuleFor(url);
      if (rule) return pageFetch(url, input, init, rule);
      // Everything else behaves exactly like on the site. A cross-origin
      // server that answers no CORS at all (the Listen Together server; the
      // site reached it same-origin) fails here as a TypeError, which is
      // when the core serves it instead — no impersonation, the page's own
      // headers, https only.
      return originalFetch(input, init).catch(function (error) {
        var crossOrigin = url.origin !== window.location.origin;
        if (!crossOrigin || url.protocol !== 'https:' || !(error instanceof TypeError)) throw error;
        return pageFetch(url, input, init, { origin: null, xorigin: false, fallback: true });
      });
    };
  }

  // ---- 5. No link previews on macOS ------------------------------------------

  /**
   * Moves [element]'s href into data-href, keeping the anchor tabbable so
   * keyboard navigation and the focus prefetches (PrefetchLink) still see it.
   * React Router's Link is a click handler that never reads href from the
   * DOM, so navigation is untouched; the WKWebView tooltip, which reads the
   * attribute at hit-test time, loses its subject.
   */
  var stripLinkPreview = function (element) {
    var href = element.getAttribute('href');
    if (href == null) return;
    element.removeAttribute('href');
    element.setAttribute('data-href', href);
    if (!element.hasAttribute('tabindex')) element.setAttribute('tabindex', '0');
  };

  /** Strips every anchor under [root], [root] itself included. */
  var stripLinkPreviewTree = function (root) {
    if (root.nodeType === 1) {
      if (root.tagName === 'A') stripLinkPreview(root);
      if (root.querySelectorAll) {
        var anchors = root.querySelectorAll('a[href]');
        for (var i = 0; i < anchors.length; i++) stripLinkPreview(anchors[i]);
      }
    }
  };

  var pendingStrip = 0;
  var installLinkPreviewStrip = function () {
    if (typeof MutationObserver !== 'function') return;
    // React writes href back whenever a Link's destination changes, and new
    // links mount with the app's pages; both re-enter here.
    var observer = new MutationObserver(function (mutations) {
      // Coalesced to one pass per frame: a track list mounting fires hundreds
      // of records, and each one only needs the same tree walk.
      if (pendingStrip) return;
      pendingStrip = requestAnimationFrame(function () {
        pendingStrip = 0;
        for (var i = 0; i < mutations.length; i++) {
          var record = mutations[i];
          if (record.type === 'attributes') {
            stripLinkPreview(record.target);
          } else {
            for (var j = 0; j < record.addedNodes.length; j++) {
              stripLinkPreviewTree(record.addedNodes[j]);
            }
          }
        }
      });
    });
    observer.observe(document, { childList: true, subtree: true, attributes: true, attributeFilter: ['href'] });
    // The app may have rendered while desktop_info was in flight.
    stripLinkPreviewTree(document.documentElement);
  };

  if (typeof MutationObserver !== 'function') {
    /* ancient engine, no tooltip worth the polyfill */
  } else if (desktop.platform === 'macos') {
    installLinkPreviewStrip();
  } else {
    // The platform lands asynchronously (desktop_info above); the observer
    // catches everything rendered before then.
    invoke('desktop_info')
      .then(function (info) {
        if (info.platform === 'macos') installLinkPreviewStrip();
      })
      .catch(function () {});
  }
})();
