import { useEffect, useRef, type RefObject } from 'react';

/**
 * The ambience for Apple's animated sleeve: a tiny canvas that mirrors the
 * frames of the sleeve's own <video> — the same element the clip is already
 * playing in, so nothing is decoded, buffered or downloaded twice. Each
 * tick copies the current frame once into a square bitmap a few dozen
 * pixels wide, which the CSS then blurs into the glow under the same
 * filters the static artwork's copies carry; copying a 32px square is a
 * fraction of a millisecond a frame, and while the music is paused it
 * copies nothing at all. The canvas fades in on its first frame
 * (`data-live`), so until the clip actually plays, the static artwork's
 * ambience stands in for it — and would again if the canvas ever failed,
 * since it paints over those copies only once it has somewhere to paint.
 *
 * Drawing a media element into a canvas cannot fail on the streams this
 * rides: through hls.js the clip arrives as MediaSource buffers — a
 * same-origin blob — so the canvas stays clean, and even a cross-origin
 * fallback only taints it, which display never notices.
 */
export function MotionAmbience({
  source,
  size = 32,
  className,
}: {
  /** The sleeve's video element (MotionArtwork's), sampled in place. */
  source: RefObject<HTMLVideoElement | null>;
  /** The bitmap's side in pixels; small on purpose — the CSS blur does the rest. */
  size?: number;
  className?: string;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const el = canvas.current;
    if (!el) return;
    const ctx = el.getContext('2d');
    if (!ctx) return;
    let raf = 0;
    let stopped = false;
    const draw = () => {
      const video = source.current;
      // No element, no decoded frame yet, or the clip never arrived: the
      // static ambience underneath keeps showing, nothing is drawn.
      if (!video || video.readyState < 2 || !video.videoWidth) return;
      // Paused, the sleeve holds its frame — mirror that one frame and then
      // stop redrawing; the canvas keeps the last painted frame exactly as
      // the video keeps its own.
      if (video.paused && el.hasAttribute('data-live')) return;
      // object-cover's crop: the frame's centre square fills the bitmap,
      // the same framing the sleeve shows the clip at.
      const side = Math.min(video.videoWidth, video.videoHeight);
      ctx.drawImage(
        video,
        (video.videoWidth - side) / 2,
        (video.videoHeight - side) / 2,
        side,
        side,
        0,
        0,
        el.width,
        el.height,
      );
      el.setAttribute('data-live', '');
    };
    const loop = () => {
      if (stopped) return;
      draw();
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => {
      stopped = true;
      cancelAnimationFrame(raf);
      el.removeAttribute('data-live');
    };
  }, [source]);

  return <canvas ref={canvas} width={size} height={size} className={className} aria-hidden="true" />;
}
