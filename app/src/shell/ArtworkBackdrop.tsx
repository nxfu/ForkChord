import { useEffect, useState } from 'react';
import { artworkAtPixels } from '@/lib/artwork';
import {
  FALLBACK_BACKDROP,
  FULL_BLUR_FADE_MS,
  MESH_FADE_MS,
  MESH_PX,
  SCRIM_FULL_BLUR,
  SCRIM_MESH,
  fullBlurFor,
  hasFullBlur,
  meshFor,
  type BlurBitmap,
  type MeshTexture,
} from '@/lib/artworkBackdrop';

/*
 * The full-screen player's backdrop, ported from the Kotlin app's
 * ArtworkMeshBackdrop.kt, both layers of it:
 *
 *  - the mesh underneath: the artwork's own colours, in the artwork's own
 *    proportions, averaged into a grid and smoothed — nine-tenths black stays
 *    nine-tenths black, where the old web backdrop's quantiser painted the
 *    whole screen red;
 *  - the full-cover blur over it: the same decode box-blurred at thumbnail
 *    scale and cropped across the whole surface under the darker legibility
 *    scrim — the layer the tablet and desktop players of the Kotlin app live
 *    on, and the one that carries the lyrics and the queue.
 *
 * Both stay mounted for the whole song (the Kotlin screen mounts both for the
 * same reason: "only these retained layers' alpha changes … so the full-cover
 * blur is neither rebuilt nor switched in on a hard frame boundary"). The mesh
 * the screen already holds stays up until the new track's is decoded — wrong
 * colour for one decode is the trade; absent colour was the flicker — and a
 * cached blur swaps in without a fade, exactly as the Kotlin cache does.
 */

interface MeshLayer {
  key: number;
  texture: MeshTexture;
}

let meshKey = 0;

export function ArtworkBackdrop({ artworkUrl }: { artworkUrl: string | null }) {
  // The mesh layers on screen, oldest first. A new track appends and fades in
  // over the previous one; the previous is pruned once the fade has finished.
  const [meshes, setMeshes] = useState<MeshLayer[]>([]);
  // The blurred cover, and whether the layer is up at all: a decode that has
  // to come over the wire fades the old blur out first (the mesh shows), the
  // way Kotlin's imageAlpha animates to 0 while the image is null.
  const [blur, setBlur] = useState<BlurBitmap | null>(null);
  const [blurUp, setBlurUp] = useState(false);

  useEffect(() => {
    if (!artworkUrl) return;
    let live = true;
    // One decode serves both layers, at the size the mesh reads (120px is a
    // variant the CDN caches — lib/artwork.ts).
    const sized = artworkAtPixels(artworkUrl, MESH_PX);
    setBlurUp(hasFullBlur(sized));
    void meshFor(sized).then((texture) => {
      if (!live || !texture) return;
      meshKey += 1;
      const layer: MeshLayer = { key: meshKey, texture };
      setMeshes((current) => [...current.slice(-1), layer]);
    });
    void fullBlurFor(sized).then((bitmap) => {
      if (!live || !bitmap) return;
      setBlur(bitmap);
      setBlurUp(true);
    });
    return () => {
      live = false;
    };
  }, [artworkUrl]);

  // Once the newest mesh has finished fading in, the screen keeps only it.
  useEffect(() => {
    if (meshes.length <= 1) return;
    const timer = setTimeout(() => setMeshes((current) => current.slice(-1)), MESH_FADE_MS + 100);
    return () => clearTimeout(timer);
  }, [meshes]);

  return (
    <div className="absolute inset-0 -z-1 overflow-hidden" style={{ backgroundColor: FALLBACK_BACKDROP }} aria-hidden="true">
      {/* The mesh, under its light scrim. The layer crossfades out as the blur
          arrives (Kotlin's `alpha = 1f - fullArtworkBackdropAlpha`, 360 ms),
          so the two scrims never stack at full strength. */}
      <div
        className="absolute inset-0 transition-opacity duration-360 ease-[cubic-bezier(0.4,0,0.2,1)]"
        style={{ opacity: blurUp ? 0 : 1 }}
      >
        {meshes.map((mesh, index) => (
          <div
            // The newest layer fades in over whatever is under it; the one
            // before it is only drawable until the fade has finished with it.
            key={mesh.key}
            className={index === meshes.length - 1 && meshes.length > 1 ? 'absolute -inset-[6%] animate-[backdrop-fade_0.9s_cubic-bezier(0.4,0,0.2,1)]' : 'absolute -inset-[6%]'}
            style={{ filter: 'blur(32px)' }}
          >
            <MeshCanvas texture={mesh.texture} />
            {/* Enough of a scrim to keep white text off a bright sleeve, and
                no more — these are the sleeve's own colours. */}
            <div className="absolute inset-0" style={{ backgroundImage: SCRIM_MESH }} />
          </div>
        ))}
      </div>

      {/* The full-cover blur, over its darker scrim. Fades in over 320 ms when
          a decode lands; a cached cover swaps without a fade. */}
      <div
        className="absolute inset-0 transition-opacity ease-out"
        style={{ opacity: blurUp ? 1 : 0, transitionDuration: `${FULL_BLUR_FADE_MS}ms` }}
      >
        {blur && <BlurCanvas bitmap={blur} />}
        <div className="absolute inset-0" style={{ backgroundImage: SCRIM_FULL_BLUR }} />
      </div>
    </div>
  );
}

/** Puts [data] on the canvas once it exists — a callback ref, so the draw
 * happens at mount with no extra render. */
function drawBitmap(data: MeshTexture['data'], width: number, height: number) {
  return (element: HTMLCanvasElement | null): void => {
    if (!element) return;
    element.getContext('2d')?.putImageData(new ImageData(data, width, height), 0, 0);
  };
}

/** The 32×32 mesh texture, stretched to the surface by the browser's
 * bilinear sampler — one canvas, one sample, no per-frame work. */
function MeshCanvas({ texture }: { texture: MeshTexture }) {
  return (
    <canvas
      ref={drawBitmap(texture.data, texture.width, texture.height)}
      width={texture.width}
      height={texture.height}
      className="absolute inset-0 size-full"
    />
  );
}

/** The pre-blurred cover, cropped across the surface — Kotlin's
 * `ContentScale.Crop` is the canvas's own object-fit. */
function BlurCanvas({ bitmap }: { bitmap: BlurBitmap }) {
  return (
    <canvas
      ref={drawBitmap(bitmap.data, bitmap.width, bitmap.height)}
      width={bitmap.width}
      height={bitmap.height}
      className="absolute inset-0 size-full object-cover"
    />
  );
}
