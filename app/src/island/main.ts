/**
 * The Dynamic Island's brain.
 *
 * State flows in one way: the shell evals `window.__IslandSync(<snapshot>)`
 * on every media update (island.rs::sync — the same snapshot the tray and
 * MPRIS read), and the shell evals `window.__IslandFocus(<bool>)` whenever
 * the main window's focus changes, with one boot-time ask (`island_focus_state`)
 * that covers an island born into an already-unfocused app. Presses flow out
 * through three commands, mirroring the tray: `island_action` for transport
 * (it presses the page's own media-session handlers — the island never talks
 * to the app page directly), `island_resize` to grow/shrink the OS window
 * around the animated card, `island_overlay` to hand the hidden rectangle
 * back to the desktop.
 *
 * Behaviour: the island is BitChord's glance, not its second player. It exists
 * only while the main window is *not* focused (with focus it would repeat what
 * the app's own screen already says), folds away when the player has nothing
 * to show — a pause is not nothing: the pill holds its ground, play button and
 * all — and a new track never expands it. The card grows only under the
 * user's own click, then collapses after a few seconds. A right-click
 * dismisses it for this stretch of playback; it comes back only after the
 * user has returned to BitChord and left it again, never on a song change.
 * Long titles and artists crawl instead of truncating (the tape below), and
 * the drawing follows the app's own scrubber (`useScrubber`): a `--progress`
 * transform on the fill, seek on release, steady playback drawn ten times a
 * second, drags at the frame rate.
 */

import { sizedArtwork } from '@/lib/artwork';

type Snapshot = {
  state: string;
  title: string | null;
  artist: string | null;
  album: string | null;
  artworkUrl: string | null;
  duration: number;
  position: number;
  rate: number;
  /** The track's like state; null while signed out or unknown. */
  liked: boolean | null;
};

type Host = { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };

declare global {
  interface Window {
    __IslandSync: (snapshot: Snapshot) => void;
    __IslandFocus: (focused: boolean) => void;
  }
}

const host: Host | null = (window as unknown as { __IslandHost?: Host }).__IslandHost ?? null;

/** Every press is best-effort: the island must survive a missing host (it
 * renders in a plain browser for design work) and a dead IPC alike. */
function invoke(cmd: string, args?: Record<string, unknown>): void {
  try {
    void host?.invoke(cmd, args);
  } catch {
    /* the overlay is cosmetic; nothing here may take the app down */
  }
}

/** The same, for the asks whose landing the page animates against: the shell
 * resolves the command once it has been handed to the main thread. */
function invokeAsync(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
  try {
    return host?.invoke(cmd, args) ?? Promise.resolve();
  } catch {
    return Promise.resolve();
  }
}

const island = document.getElementById('island')!;
const miniTitle = document.getElementById('mini-title')!;
const miniRemaining = document.getElementById('mini-remaining')!;
const fullTitle = document.getElementById('full-title')!;
const fullArtist = document.getElementById('full-artist')!;
const timeElapsed = document.getElementById('time-elapsed')!;
const timeRemaining = document.getElementById('time-remaining')!;
const fill = document.getElementById('scrub-fill')!;
const scrub = document.getElementById('scrub')!;
const rail = fill.parentElement!;
const playButton = document.getElementById('btn-play')!;
const likeButton = document.getElementById('btn-like') as HTMLButtonElement;
const fullArt = document.getElementById('full-art')!;
const artImages = Array.from(document.querySelectorAll<HTMLImageElement>('.art img'));
const artFrames = Array.from(document.querySelectorAll<HTMLElement>('.art'));

const COLLAPSE_AFTER_MS = 5000; // Airbeats' dwell time for an expanded card
const RESIZE_SETTLE_MS = 440; // the card's own width/height transition

// ---- The tape ---------------------------------------------------------------

/** Long names crawl instead of truncating. Each text line is a clipping box
 * (.mini-title, .title, .artist) over an inline .tape that carries the words;
 * when the words don't fit, the box gets the `scrolling` class, the distance
 * on --tape-shift and the pace on --tape-duration, and the CSS ping-pongs the
 * tape between its ends (island.css's tape-crawl). The box clips, so nothing
 * is ever cut — and a name that fits never crawls. */
const TAPE_SPEED = 24; // scrolled pixels per second: the "slowly" cadence
/** Sub-this-much overflow is kerning dust, not a second line worth of words. */
const TAPE_MIN_SHIFT = 4;

/** A face's boxes are only measurable while the card is laid out at that
 * face's own width — the other face is absolutely positioned over it and
 * borrows whatever width the card currently has. Measuring the expanded
 * card's lines while the pill is up would read the pill's 216px as their
 * box, and the crawl would start against a lie. */
const TAPE_FACE_WIDTH = { mini: 216, full: 432 } as const;

interface Tape {
  box: HTMLElement;
  tape: HTMLElement;
  /** Which face of the card the box lives on ('mini' or 'full'). */
  face: keyof typeof TAPE_FACE_WIDTH;
  /** The shift currently applied, so a re-measure can tell change from churn. */
  applied: number;
}

function makeTape(id: string, face: Tape['face']): Tape {
  const box = document.getElementById(id)!;
  const tape = box.querySelector<HTMLElement>('.tape')!;
  return { box, tape, face, applied: 0 };
}

const tapes = [makeTape('mini-title', 'mini'), makeTape('full-title', 'full'), makeTape('full-artist', 'full')];

/** Measures one line and arms or disarms its crawl. Skipped while the card is
 * mid-morph (the ResizeObserver below re-fires once the width settles), and
 * restarted rather than nudged when an already-crawling tape's distance
 * changes — a running animation can't have its keyframe distance swapped
 * under it without a jump, but it can start over from its rest. */
function measureTape(entry: Tape): void {
  if (Math.abs(island.clientWidth - TAPE_FACE_WIDTH[entry.face]) > 0.5) return;
  const shift = Math.round(entry.tape.getBoundingClientRect().width - entry.box.clientWidth);
  const scrolling = shift >= TAPE_MIN_SHIFT;
  if (!scrolling) {
    entry.applied = 0;
    entry.box.classList.remove('scrolling');
    return;
  }
  // One-way is 36% of the cycle (the keyframes hold the rest), so the pace
  // on screen stays TAPE_SPEED regardless of how far the words reach.
  const duration = shift / (TAPE_SPEED * 0.36);
  entry.box.style.setProperty('--tape-shift', `${-shift}px`);
  entry.box.style.setProperty('--tape-duration', `${duration.toFixed(2)}s`);
  if (entry.box.classList.contains('scrolling') && Math.abs(shift - entry.applied) <= 1) return;
  entry.applied = shift;
  entry.box.classList.remove('scrolling');
  void entry.box.offsetWidth; // flush, so a re-armed crawl starts from its rest
  entry.box.classList.add('scrolling');
}

function measureTapes(): void {
  for (const entry of tapes) measureTape(entry);
}

// The card's width is transitioned, so a morph re-fires the observers frame
// by frame until the destination width lands and the guard above lets the
// measurement through — the crawl always settles against the real box.
const tapeObserver = new ResizeObserver(measureTapes);
for (const entry of tapes) tapeObserver.observe(entry.box);
// Fonts arrive after the first paint, and a swap changes the *words'* width,
// not the box's — the observers never see it. `fonts.ready` is not enough: it
// can resolve before the @font-face downloads have even started (the loading
// they trigger begins at first render), so the settle is also taken from the
// FontFaceSet's own `loadingdone` — the event that fires when the real faces
// finish — with the ladder as backstop for late swaps.
if (document.fonts?.ready) {
  void document.fonts.ready.then(() => {
    measureTapes();
    requestAnimationFrame(measureTapes);
    window.setTimeout(measureTapes, 300);
    window.setTimeout(measureTapes, 900);
  });
}
document.fonts?.addEventListener?.('loadingdone', measureTapes);

/** The last snapshot the shell sent. */
let snap: Snapshot | null = null;
/** performance.now() when that snapshot's position was true. */
let anchor = 0;
/** The visible state, also mirrored into the OS window's size. */
let mode: 'hidden' | 'mini' | 'full' = 'hidden';
/** The metadata the card is currently displaying, for change detection. */
let renderedKey = '';
let collapseTimer: number | null = null;
let pendingShrink: number | null = null;
/** Fraction while dragging the seek line, else null. */
let dragging: number | null = null;
let rafId = 0;
let lastDraw = -Infinity;
/** Stamps each expand request; a collapsed (or re-hidden) card leaves the
 * late-arriving window-grow with nothing to animate. */
let expandToken = 0;
/** Whether BitChord owns the focus. The shell evals every change; the one
 * boot-time query covers an island that loads while the app is already in the
 * background. Optimistically focused: until the answer lands, the card stays
 * away — a wrong guess would flash the island over the app's own screen. */
let appFocused = true;
/** A right-click dismissal lasts until the user has been back in BitChord
 * (which clears it, see __IslandFocus) and left again. Song changes, play and
 * pause must not undo it. */
let dismissed = false;

function fmt(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '0:00';
  const total = Math.floor(seconds);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${s.toString().padStart(2, '0')}`;
}

/** Position carried forward by the rate — the shell throttles its updates
 * (~900 ms); this is what makes the line move like the app's does. */
function livePosition(now: number): number {
  if (!snap) return 0;
  if (dragging !== null) return dragging * (snap.duration > 0 ? snap.duration : 0);
  const base = snap.position;
  if (snap.state !== 'playing') return base;
  return base + ((now - anchor) / 1000) * (snap.rate || 1);
}

function draw(now: number): void {
  if (!snap) return;
  const duration = snap.duration > 0 ? snap.duration : 0;
  const position = Math.min(Math.max(livePosition(now), 0), duration || Math.max(0, livePosition(now)));
  const progress = duration > 0 ? Math.min(1, Math.max(0, position / duration)) : 0;
  fill.style.setProperty('--progress', progress.toFixed(4));
  timeElapsed.textContent = fmt(position);
  const remaining = duration > 0 ? fmt(Math.max(0, duration - position)) : '0:00';
  timeRemaining.textContent = `-${remaining}`;
  miniRemaining.textContent = duration > 0 ? `-${remaining}` : '';
}

/** The scrubber's idle cadence: ten draws a second, frame-rate while a drag
 * holds the line, nothing at all when hidden (a hidden webview paints
 * nothing, but neither should its loop burn cycles). The mini pill draws too:
 * its remaining-time label rides the same clock. */
function tick(now: number): void {
  rafId = 0;
  if (!snap || mode === 'hidden') return;
  const due = dragging !== null || now - lastDraw >= 100;
  if (due) {
    lastDraw = now;
    draw(now);
  }
  rafId = requestAnimationFrame(tick);
}

function schedule(): void {
  if (!rafId && mode !== 'hidden') rafId = requestAnimationFrame(tick);
}

function setClasses(next: typeof mode): void {
  island.classList.toggle('hidden', next === 'hidden');
  island.classList.toggle('mini', next === 'mini');
  island.classList.toggle('full', next === 'full');
}

/** The spawn: the card pops in from nothing — a quick grow with a settle at
 * the end, all inside the pill's own box so the OS window (which hugs the
 * card) never clips it. Applied whenever the card comes out of 'hidden'; the
 * class removes itself on animationend, and a mode change first tears any
 * running spawn down so a fresh one starts from its own first frame. */
function playSpawn(): void {
  island.classList.remove('spawn');
  void island.offsetWidth; // flush, so removing and adding restarts the animation
  island.classList.add('spawn');
}

island.addEventListener('animationend', () => {
  island.classList.remove('spawn');
});

/** Two painted frames. The shell resolves `island_resize` when the main
 * thread has the geometry task; by the compositor's next paint that task has
 * run, so the morph never starts inside a window still clipping it. (In a
 * plain browser there is no shell and no clipping; the promise resolves at
 * once and the frames are all the wait there is.) */
function nextPaint(): Promise<void> {
  return new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  });
}

function applyMode(next: typeof mode): void {
  if (mode === next) return;
  const previous = mode;
  mode = next;
  // The window hugs the card. Expanding grows the window first and only then
  // starts the morph — a 216px window would clip the growing card's first
  // frames and read as a stutter. Collapsing animates at once (the big window
  // has room), then shrinks once the card has settled — whatever it collapsed
  // into: a pause while expanded must hand the small window back too, or the
  // resumed pill sits in a 432px window that eats clicks around it.
  if (next === 'full') {
    if (pendingShrink !== null) {
      clearTimeout(pendingShrink);
      pendingShrink = null;
    }
    const token = ++expandToken;
    const spawning = previous === 'hidden';
    void invokeAsync('island_resize', { expanded: true })
      .then(() => nextPaint())
      .then(() => {
        if (expandToken !== token || mode !== 'full') return;
        setClasses('full');
        if (spawning) playSpawn();
        // The morph's width transition can drop its last ResizeObserver
        // callback (browsers coalesce late frames), which would leave the
        // tape's crawl distance measured against a mid-flight width. Once
        // the transition has settled, measure against the settled box. The
        // crawl starts 0.8s after re-arming, so this lands before it moves.
        window.setTimeout(measureTapes, RESIZE_SETTLE_MS + 80);
      });
  } else {
    ++expandToken;
    setClasses(next);
    // Out of nothing: the pill pops in (see playSpawn). Between the two
    // visible states the card simply morphs — no spawn there.
    if (previous === 'hidden') playSpawn();
    if (previous === 'full') {
      if (pendingShrink !== null) clearTimeout(pendingShrink);
      pendingShrink = window.setTimeout(() => {
        pendingShrink = null;
        invoke('island_resize', { expanded: false });
      }, RESIZE_SETTLE_MS);
    }
    // Same settle for the collapse: the mini face gets its real width back.
    window.setTimeout(measureTapes, RESIZE_SETTLE_MS + 80);
  }
  invoke('island_overlay', { hidden: next === 'hidden' });
  lastDraw = -Infinity;
  schedule();
}

function cancelCollapse(): void {
  if (collapseTimer !== null) {
    clearTimeout(collapseTimer);
    collapseTimer = null;
  }
}

function scheduleCollapse(): void {
  cancelCollapse();
  collapseTimer = window.setTimeout(() => {
    collapseTimer = null;
    if (mode === 'full') applyMode('mini');
  }, COLLAPSE_AFTER_MS);
}

function expand(): void {
  applyMode('full');
  scheduleCollapse();
}

function collapse(): void {
  cancelCollapse();
  if (mode === 'full') applyMode('mini');
}

/** Renders metadata; returns whether the track itself changed. */
function renderMetadata(s: Snapshot): boolean {
  const key = `${s.title ?? ''}\u0000${s.artist ?? ''}\u0000${s.artworkUrl ?? ''}`;
  const changed = key !== renderedKey && Boolean(s.title || s.artworkUrl);
  renderedKey = key;
  miniTitle.firstElementChild!.textContent = s.title ?? '';
  fullTitle.firstElementChild!.textContent = s.title ?? '';
  fullArtist.firstElementChild!.textContent = s.artist ?? '';
  fullTitle.title = s.title ?? '';
  // New words, new width. Harmless when nothing changed: an armed crawl whose
  // distance re-measures the same is left alone.
  measureTapes();
  const src = s.artworkUrl ? sizedArtwork(s.artworkUrl, 48) : '';
  for (const img of artImages) {
    if (img.getAttribute('src') !== src) {
      img.classList.remove('loaded');
      if (src) img.src = src;
      else img.removeAttribute('src');
    }
  }
  for (const frame of artFrames) frame.classList.toggle('bare', !s.artworkUrl);
  return changed;
}

function renderPlayback(s: Snapshot): void {
  const playing = s.state === 'playing';
  playButton.classList.toggle('paused', !playing);
  playButton.setAttribute('aria-label', playing ? 'Pause' : 'Play');
}

/** The like button: hidden while signed out or before the first answer,
 * the key-colour filled star when liked. An older shell's snapshot (or a
 * plain browser) may not carry the field at all — read as null, hidden. */
function renderLike(s: Snapshot): void {
  const liked = s.liked === true;
  likeButton.hidden = s.liked == null;
  likeButton.classList.toggle('liked', liked);
  likeButton.setAttribute('aria-pressed', String(liked));
  likeButton.setAttribute('aria-label', liked ? 'Unfavorite' : 'Favorite');
}

/** The one policy for what the island shows, applied on every input — a media
 * sync, a focus change, the boot-time focus answer. Nothing to show wins first
 * (a track-less player is nothing — no title and no artwork), then BitChord's
 * own focus (the page already shows everything the card would say), then a
 * dismissal. A pause is *not* nothing: the pill holds its ground with its play
 * button, like the phone's does — only the user's own right-click sends it
 * away mid-track. Only when there is a track, BitChord is in the background
 * and the user hasn't dismissed the island does the card exist; coming out of
 * 'hidden' it spawns.
 * Song changes are *not* an input: they refresh what the card shows, never
 * whether it shows. */
function reconcile(): void {
  if (!snap || (!snap.title && !snap.artworkUrl)) {
    cancelCollapse();
    applyMode('hidden');
    return;
  }
  if (appFocused || dismissed) {
    cancelCollapse();
    applyMode('hidden');
    return;
  }
  if (mode === 'hidden') applyMode('mini');
}

window.__IslandSync = (raw: Snapshot) => {
  snap = raw;
  anchor = performance.now();

  renderMetadata(raw);
  renderPlayback(raw);
  renderLike(raw);
  reconcile();

  lastDraw = -Infinity;
  schedule();
};

/** The shell's focus push (island.rs::notify_main_focus), and the same place
 * a dismissal is cleared: being back in BitChord re-arms the island, so it
 * returns only when the user leaves the app again — never on a song switch. */
window.__IslandFocus = (focused: boolean) => {
  appFocused = focused;
  if (focused) dismissed = false;
  reconcile();
};

// The boot-time ask, so an island created while BitChord is already in the
// background (the settings toggle, or a session restored to the tray) knows
// it may show before the next Focused event crosses the shell.
void invokeAsync('island_focus_state').then((focused) => {
  appFocused = focused !== false;
  reconcile();
});

// ---- The presses -----------------------------------------------------------

// The card expands on a click of the pill itself; the controls are excluded,
// like Airbeats excluded its own.
island.addEventListener('click', (event) => {
  if (mode !== 'mini') return;
  if ((event.target as HTMLElement | null)?.closest('button')) return;
  expand();
});

// A right-click sends the island away for this stretch of listening — not a
// collapse to mini, a dismissal. It returns only through reconcile's rules:
// the user has to have been back in BitChord (which clears `dismissed` in
// __IslandFocus) and left the app again. Nothing else — not a song change,
// not play or pause — revives it.
island.addEventListener('contextmenu', (event) => {
  event.preventDefault();
  dismissed = true;
  cancelCollapse();
  applyMode('hidden');
});

document.getElementById('btn-previous')!.addEventListener('click', () => {
  invoke('island_action', { action: 'previoustrack' });
  scheduleCollapse();
});

document.getElementById('btn-next')!.addEventListener('click', () => {
  invoke('island_action', { action: 'nexttrack' });
  scheduleCollapse();
});

playButton.addEventListener('click', () => {
  // Explicit play/pause, resolved locally — the same resolution the tray's
  // Play/Pause item makes, so both stay honest against the same snapshot.
  invoke('island_action', { action: snap?.state === 'playing' ? 'pause' : 'play' });
  scheduleCollapse();
});

// Like: pressed through the page's own toggle (the same mutation the app's
// Favorite stars run), with the flip drawn at once — the round trip through
// the page's mutation and back rides the next sync, which confirms it or
// draws the truth over the guess.
likeButton.addEventListener('click', () => {
  const liked = !likeButton.classList.contains('liked');
  likeButton.classList.toggle('liked', liked);
  likeButton.setAttribute('aria-pressed', String(liked));
  likeButton.setAttribute('aria-label', liked ? 'Unfavorite' : 'Favorite');
  invoke('island_action', { action: 'togglelike' });
  scheduleCollapse();
});

// The artwork raises the app, as the LCD's opens the full-screen player.
fullArt.addEventListener('click', () => {
  invoke('island_action', { action: 'open' });
});

// ---- The seek line ----------------------------------------------------------

function fractionAt(event: PointerEvent): number {
  const rect = rail.getBoundingClientRect();
  if (rect.width <= 0) return 0;
  return Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width));
}

scrub.addEventListener('pointerdown', (event) => {
  if (!snap || snap.duration <= 0) return;
  dragging = fractionAt(event);
  scrub.classList.add('dragging');
  scrub.setPointerCapture(event.pointerId);
  lastDraw = -Infinity;
  schedule();
});

scrub.addEventListener('pointermove', (event) => {
  if (dragging === null) return;
  dragging = fractionAt(event);
  lastDraw = -Infinity;
  schedule();
});

const endDrag = (event: PointerEvent) => {
  if (dragging === null) return;
  const fraction = fractionAt(event);
  dragging = null;
  scrub.classList.remove('dragging');
  if (snap && snap.duration > 0) {
    invoke('island_action', { action: 'seekto', arg: fraction * snap.duration });
    // The shell's next position sync confirms it; hold the picked spot until
    // then so the line does not snap backwards.
    snap.position = fraction * snap.duration;
    anchor = performance.now();
  }
  scheduleCollapse();
};

scrub.addEventListener('pointerup', endDrag);
scrub.addEventListener('pointercancel', endDrag);

// ---- Window manners ---------------------------------------------------------

// Focus leaving the island folds it back, the way the phone's card folds when
// you touch elsewhere. The auto-collapse timer covers the no-click case.
window.addEventListener('blur', () => {
  if (dragging !== null) return;
  collapse();
});

// Artwork arrives or fails independent of the state syncs.
for (const img of artImages) {
  img.addEventListener('load', () => img.classList.add('loaded'));
  img.addEventListener('error', () => {
    img.classList.remove('loaded');
    img.parentElement?.classList.add('bare');
  });
}
