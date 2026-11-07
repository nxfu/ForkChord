import { Minus, Square, X } from 'lucide-react';
import { useEffect } from 'react';
import { useUi } from '@/state/ui';
import { useWindowChrome } from './useWindowChrome';

/**
 * The transparent top bar of the frameless desktop window.
 *
 * The native frame is gone (lib.rs builds the main window without
 * decorations on Windows and Linux): no title, no icon, no caption buttons —
 * the app's own page runs all the way to the top edge, so a tinted artist or
 * album page colours the top of the window like it colours the rest. What
 * the frame used to do, the page now does:
 *
 *  - the three window buttons, top right, in the app's own materials —
 *    transparent until hovered, the close glowing the system red;
 *  - dragging from empty top-of-window space (a plain mousedown within the
 *    bar's height that did not land on content: buttons, links and fields
 *    keep their clicks), and a double-click there toggles maximize — the
 *    caption's gestures, decided where the content lives.
 *
 * The two halves split by platform. The buttons replace the caption the
 * shell removed, so they draw on Windows and Linux alone; macOS keeps its
 * traffic lights over the glass and draws nothing. The drag zone is for
 * every desktop platform, macOS emphatically included: Overlay hides the
 * title behind the glass, and the webview runs the full height of the
 * window, so there is no native strip left to move the window by — without
 * these listeners the app is parked where it opened. (useWindowChrome marks
 * the split; the listeners are the part it turns on everywhere.)
 *
 * The bar itself is never a hit target: it paints nothing and lets every
 * pointer event through (`pointer-events: none`), so page content scrolled
 * under it stays clickable and only the buttons receive clicks. The drag
 * zone is a window-level listener, not an element, for the same reason.
 *
 * In the full-screen player the bar is gone entirely — buttons and drag
 * zone: that view is the app's own full-bleed stage with its own corner
 * chrome and its own way out, and a second row of window buttons floating
 * over it has nothing to do. It returns when the player closes.
 */

/** How far down the window a mousedown still counts as the bar. Matches the
 * button strip's height (`h-10`) and the macOS title-bar zone's spirit. */
const BAR_HEIGHT = 40;

/** A mousedown on any of these belongs to the page, never to the bar. */
const INTERACTIVE = 'button, a, input, textarea, select, label, [role="button"], [contenteditable="true"]';

type Internals = {
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
};

/** Best-effort by design: a failed window command is a missed click, not a
 * dead page. */
function press(cmd: string): void {
  const internals = (window as unknown as { __TAURI_INTERNALS__?: Internals }).__TAURI_INTERNALS__;
  void internals?.invoke(cmd).catch(() => undefined);
}

export function WindowControls() {
  const { present, platform } = useWindowChrome();
  // The full-screen player is the one view that owns the whole window: while
  // it is up, the bar stands down (buttons and drag gestures alike).
  const fullScreen = useUi((s) => s.fullScreen);
  const active = present && !fullScreen;
  // The caption buttons are the replacement for a removed native frame:
  // Windows and Linux. macOS keeps the traffic lights it was built with.
  const buttons = active && platform !== 'macos';

  useEffect(() => {
    if (!active) return;
    const onBar = (event: { clientY: number; button: number; target: EventTarget | null }) => {
      if (event.button !== 0 || event.clientY > BAR_HEIGHT) return false;
      const target = event.target;
      return !(target instanceof Element && target.closest(INTERACTIVE));
    };
    const onMouseDown = (event: MouseEvent) => {
      if (onBar(event)) press('window_start_drag');
    };
    const onDoubleClick = (event: MouseEvent) => {
      if (onBar(event)) press('window_toggle_maximize');
    };
    window.addEventListener('mousedown', onMouseDown);
    window.addEventListener('dblclick', onDoubleClick);
    return () => {
      window.removeEventListener('mousedown', onMouseDown);
      window.removeEventListener('dblclick', onDoubleClick);
    };
  }, [active]);

  if (!buttons) return null;

  return (
    <div className="pointer-events-none fixed inset-x-0 top-0 z-[calc(var(--z-contextual-menus)-1)] flex justify-end">
      {/* One strip, three buttons: the caption's geometry (40px tall, 44px a
          button), drawn in the app's own tokens instead of the system's. */}
      <div className="pointer-events-auto flex h-10">
        <button
          className="grid w-11 place-items-center text-secondary transition-colors duration-100 hover:bg-quinary hover:text-primary"
          aria-label="Minimize"
          title="Minimize"
          onClick={() => press('window_minimize')}
        >
          <Minus size={15} strokeWidth={1.8} />
        </button>
        <button
          className="grid w-11 place-items-center text-secondary transition-colors duration-100 hover:bg-quinary hover:text-primary"
          aria-label="Maximize"
          title="Maximize"
          onClick={() => press('window_toggle_maximize')}
        >
          <Square size={11} strokeWidth={2} />
        </button>
        <button
          className="grid w-11 place-items-center text-secondary transition-colors duration-100 hover:bg-[var(--systemRed)] hover:text-white"
          aria-label="Close"
          title="Close"
          onClick={() => press('window_hide')}
        >
          <X size={16} strokeWidth={1.8} />
        </button>
      </div>
    </div>
  );
}
