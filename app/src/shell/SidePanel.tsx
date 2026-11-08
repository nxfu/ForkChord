import { lazy, Suspense } from 'react';
import { useUi } from '@/state/ui';
import { useWindowChrome } from './useWindowChrome';
import { UpNext } from './UpNext';

// am-lyrics is large; it loads the first time the panel opens (prompt §3).
const LyricsPanel = lazy(() => import('./LyricsPanel').then((m) => ({ default: m.LyricsPanel })));

/**
 * Up Next / Lyrics glass panel (report §7).
 *
 * Where the shell draws caption buttons (the frameless window's top-right
 * strip), the panel's own top row — the lyrics provider picker with its
 * settings gear, and Up Next's AutoPlay/options buttons — would sit inside
 * the strip: on Windows the gear landed inside the close button, unclickable
 * and half-hidden under it. The panel clears the strip with the same 40px
 * the caption bar is tall, desktop only (the web app has no caption, and on
 * macOS the traffic lights hang over the sidebar, not this panel; the
 * full-screen player stands the caption down and hides this panel anyway).
 */
export function SidePanel() {
  const panel = useUi((s) => s.sidePanel);
  // Hidden under the full-screen player, which has its own lyrics: one am-lyrics at a time.
  const fullScreen = useUi((s) => s.fullScreen);
  const { present, platform } = useWindowChrome();
  const underCaption = present && platform !== 'macos';
  if (!panel || fullScreen) return null;

  return (
    <aside
      className={`absolute inset-y-0 right-0 z-(--z-web-chrome) flex w-(--side-panel-width) animate-slide-in flex-col glass-material shadow-glass max-sm:w-full${underCaption ? ' pt-10' : ''}`}
      aria-label={panel === 'up-next' ? 'Up Next' : 'Lyrics'}
    >
      {panel === 'up-next' ? (
        <UpNext />
      ) : (
        <Suspense fallback={null}>
          <LyricsPanel />
        </Suspense>
      )}
    </aside>
  );
}
