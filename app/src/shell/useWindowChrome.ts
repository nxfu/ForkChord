import { useEffect, useState } from 'react';

/**
 * Which parts of the window's chrome the page owns, and on which platform.
 *
 * `present` is true whenever the desktop shell owns the window — every Tauri
 * platform of this frameless build, macOS included. It puts the page in
 * charge of the *gestures a title bar handles*: dragging from empty
 * top-of-window space and double-click to maximize. That includes macOS,
 * because Overlay keeps the traffic lights but hides the title behind the
 * glass, and a webview that runs the full height of the window leaves no
 * native strip to grab — without the page's drag zone the window simply
 * cannot be moved (which it could not, before).
 *
 * `platform` separates the two halves of that job. The three caption buttons
 * replace the frame the shell removed — Windows and Linux only; macOS keeps
 * its native traffic lights over the glass, and drawing a second set on top
 * of them would be nonsense. The drag zone is platform-independent.
 *
 * The answer arrives with the shell's `desktop_info` round trip, so a fresh
 * window doesn't guess wrong for a frame; every reader re-renders when it
 * lands. In a plain browser there is neither window nor shell: present is
 * false and stays false.
 */

type Internals = {
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
};

type DesktopInfo = { platform: string };

export type WindowChrome = { present: boolean; platform: string };

const BROWSER: WindowChrome = { present: false, platform: 'browser' };

let cached: WindowChrome | null = null;
const waiters = new Set<(info: WindowChrome) => void>();

function publish(value: WindowChrome) {
  cached = value;
  for (const waiter of waiters) waiter(value);
}

/** Asks the shell once; in a browser there is no shell and never a bar. */
function probe() {
  const internals = (window as unknown as { __TAURI_INTERNALS__?: Internals }).__TAURI_INTERNALS__;
  if (!internals) {
    publish(BROWSER);
    return;
  }
  void internals
    .invoke('desktop_info')
    .then((info) => publish({ present: true, platform: (info as DesktopInfo).platform }))
    .catch(() => publish(BROWSER));
}

export function useWindowChrome(): WindowChrome {
  const [info, setInfo] = useState<WindowChrome>(cached ?? BROWSER);
  useEffect(() => {
    if (cached !== null) {
      setInfo(cached);
      return;
    }
    waiters.add(setInfo);
    if (cached === null && waiters.size === 1) probe();
    return () => {
      waiters.delete(setInfo);
    };
  }, []);
  return info;
}
