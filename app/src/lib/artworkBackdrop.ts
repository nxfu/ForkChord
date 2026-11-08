/*
 * The Kotlin app's player backdrop, ported file for file:
 * sharedUi/src/jvmSharedMain/kotlin/com/music/bitchord/ui/player/ArtworkMeshBackdrop.kt
 * (plus its ColorUtils partner, ported as written "so the phone's numbers do
 * not move"). Two layers, both of which the Kotlin docstring explains better
 * than a summary can:
 *
 *  - The mesh. NOT a quantiser's opinion of what a cover is "about" — the
 *    reason a cover that is nine-tenths black with a red stripe painted the
 *    whole screen red in the old web backdrop. The cover is averaged into a
 *    6×6 grid of cell means (MESH_GRID), so proportions survive: nine-tenths
 *    black stays nine-tenths black. The rows are read bottom-up, the rows
 *    below the seam are cyclic-shifted sideways (and mirrored half the time,
 *    seeded per artwork), and the grid is resampled to 32×32 with a curve
 *    that flattens at every control point — no creases when magnified, and a
 *    GPU blur (32px, the Kotlin blurRadius) is polish rather than structure.
 *    Still, never animated: the Kotlin mesh replaced a blob mesh that paid a
 *    permanent 60 Hz blur for a wallpaper effect.
 *
 *  - The full-cover blur. One 120px decode (the mesh's own, so one fetch and
 *    one readback serve both layers), box-blurred 3 passes at radius
 *    min(w,h)/12 — "three small box passes approximate a broad Gaussian blur
 *    at thumbnail scale" — cached by URL, drawn cropped over the whole
 *    surface under a darker legibility scrim. This is the layer the tablet
 *    and desktop players live on, and the one that carries lyrics and queue.
 *
 * Both layers crossfade on a track change and never flicker to empty while a
 * decode is in flight: the last mesh on screen is held until the new one is
 * ready (the "wrong colour for one decode is the trade; absent colour was the
 * flicker" rule).
 */

/** Cells across the mesh. Coarse deliberately — see the module comment. */
export const MESH_GRID = 6;

/** The artwork decodes at this square size before anything reads it. */
export const MESH_PX = 120;

/** The grid is interpolated to this texture before the browser stretches it. */
export const MESH_TEX = 32;

/** Bounds the per-frame read of an oversized source at a fixed cost. */
export const MESH_SAMPLE = 128;

/** Decode attempts, and the wait between them, matching the Kotlin reader. */
export const MESH_ATTEMPTS = 4;
export const MESH_RETRY_DELAY_MS = 1500;

/** How long the mesh crossfade runs (Kotlin MESH_FADE_MS). */
export const MESH_FADE_MS = 900;

/** Saturation nudged back up, lightness floored off pure black (Kotlin lift). */
export const MESH_VIBRANCE = 1.12;
export const MESH_FLOOR = 0.045;

/** The full-cover blur's decode size, passes, and the scrim over it. */
export const FULL_BLUR_PASSES = 3;

/** How long the blur layer fades in over the mesh (Kotlin, 320 ms tween). */
export const FULL_BLUR_FADE_MS = 320;

/** The surface before any artwork has been read. Never a colour anyone chose. */
export const FALLBACK_BACKDROP = '#121212';

export const SCRIM_MESH = 'linear-gradient(180deg, rgb(0 0 0 / 0.06), rgb(0 0 0 / 0.30))';
export const SCRIM_FULL_BLUR =
  'linear-gradient(180deg, rgb(0 0 0 / 0.34), rgb(0 0 0 / 0.48) 55%, rgb(0 0 0 / 0.64))';

/** One mesh cell's mean colour. */
export type Rgb = [number, number, number];

/**
 * The mesh texture, as 32×32 RGBA — putImageData onto a canvas of the same
 * size and the browser stretches it with bilinear sampling, the web's
 * `drawImage(..., FilterQuality.Low)`.
 */
export interface MeshTexture {
  width: number;
  height: number;
  data: Uint8ClampedArray<ArrayBuffer>;
}

/** The pre-blurred full-cover bitmap, same consumption as the mesh's. */
export interface BlurBitmap {
  width: number;
  height: number;
  data: Uint8ClampedArray<ArrayBuffer>;
}

/**
 * Java String.hashCode, over UTF-16 units — the seed [rotatedBelowSeam] and
 * kotlin.random.Random(seed) are both driven by, so the same artwork lands in
 * the same arrangement as it does on the phone.
 */
export function hashUrl(url: string): number {
  let hash = 0;
  for (let i = 0; i < url.length; i++) {
    hash = (Math.imul(31, hash) + url.charCodeAt(i)) | 0;
  }
  return hash;
}

/**
 * java.util.Random, whose LCG kotlin.random.Random(seed) is on the JVM — the
 * desktop app this ports from runs there. BigInt arithmetic, because the
 * multiplier does not fit a float's exact range; called twice per artwork.
 */
export class JavaRandom {
  private seed: bigint;

  private static readonly MULTIPLIER = 0x5deece66dn;
  private static readonly ADDEND = 0xbn;
  private static readonly MASK = (1n << 48n) - 1n;

  constructor(seed: number) {
    this.seed = (BigInt(seed) ^ JavaRandom.MULTIPLIER) & JavaRandom.MASK;
  }

  private next(bits: number): number {
    this.seed = (this.seed * JavaRandom.MULTIPLIER + JavaRandom.ADDEND) & JavaRandom.MASK;
    return Number(this.seed >> BigInt(48 - bits));
  }

  nextBoolean(): boolean {
    return this.next(1) !== 0;
  }

  nextInt(bound: number): number {
    if (bound <= 0) throw new RangeError('bound must be positive');
    // java.util.Random.nextInt: rejection-sample until the modulo is fair.
    for (;;) {
      const bits = this.next(31);
      const value = bits % bound;
      // The overflow check that keeps the modulo fair on any bound.
      if (bits - value + (bound - 1) <= 0x7fffffff) return value;
    }
  }
}

/**
 * Averages [pixels] (RGBA, [width]×[height]) into the grid, flipped top to
 * bottom as it is read — row 0 of the result is the artwork's own bottom
 * edge, the order the backdrop draws down the screen. Strided by
 * [sampleLimit] per axis, so a larger-than-asked decode costs a bounded read.
 */
export function meshCells(pixels: Uint8ClampedArray, width: number, height: number): Rgb[] {
  const cols = Math.min(MESH_GRID, width);
  const rows = Math.min(MESH_GRID, height);
  const rowStep = Math.max(1, Math.floor(height / MESH_SAMPLE));
  const colStep = Math.max(1, Math.floor(width / MESH_SAMPLE));

  const cells = rows * cols;
  const red = new Float64Array(cells);
  const green = new Float64Array(cells);
  const blue = new Float64Array(cells);
  const count = new Int32Array(cells);

  for (let y = 0; y < height; y += rowStep) {
    // Flipped as it is read.
    const rowBase = Math.trunc(((height - 1 - y) * rows) / height) * cols;
    for (let x = 0; x < width; x += colStep) {
      const cell = rowBase + Math.trunc((x * cols) / width);
      const at = (y * width + x) * 4;
      red[cell] = (red[cell] ?? 0) + pixels[at]!;
      green[cell] = (green[cell] ?? 0) + pixels[at + 1]!;
      blue[cell] = (blue[cell] ?? 0) + pixels[at + 2]!;
      count[cell] = (count[cell] ?? 0) + 1;
    }
  }

  const out: Rgb[] = new Array(rows * cols);
  for (let cell = 0; cell < cells; cell++) {
    const n = Math.max(1, count[cell]!);
    out[cell] = [
      Math.round(red[cell]! / n),
      Math.round(green[cell]! / n),
      Math.round(blue[cell]! / n),
    ];
  }
  return out;
}

/** The one liberty taken with the artwork's colours (Kotlin [Int.lifted]). */
export function liftCell([r, g, b]: Rgb): Rgb {
  const hsl = rgbToHsl(r, g, b);
  hsl[1] = Math.min(1, hsl[1] * MESH_VIBRANCE);
  hsl[2] = Math.max(MESH_FLOOR, hsl[2]);
  return hslToRgb(hsl[0], hsl[1], hsl[2]);
}

/**
 * com.music.bitchord.ui.graphics.ColorUtils.RGBToHSL, ported as written —
 * "so the phone's numbers do not move".
 */
export function rgbToHsl(r: number, g: number, b: number): [number, number, number] {
  const rf = r / 255;
  const gf = g / 255;
  const bf = b / 255;

  const max = Math.max(rf, Math.max(gf, bf));
  const min = Math.min(rf, Math.min(gf, bf));
  const deltaMaxMin = max - min;

  let h: number;
  let s: number;
  const l = (max + min) / 2;

  if (max === min) {
    h = 0;
    s = 0;
  } else {
    if (max === rf) {
      h = ((gf - bf) / deltaMaxMin) % 6;
    } else if (max === gf) {
      h = (bf - rf) / deltaMaxMin + 2;
    } else {
      h = (rf - gf) / deltaMaxMin + 4;
    }
    s = deltaMaxMin / (1 - Math.abs(2 * l - 1));
  }

  h = (h * 60) % 360;
  if (h < 0) h += 360;

  return [Math.min(360, Math.max(0, h)), Math.min(1, Math.max(0, s)), Math.min(1, Math.max(0, l))];
}

/** com.music.bitchord.ui.graphics.ColorUtils.HSLToColor, same port. */
export function hslToRgb(h: number, s: number, l: number): Rgb {
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const m = l - 0.5 * c;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));

  const hueSegment = Math.trunc(h) / 60;

  let r = 0;
  let g = 0;
  let b = 0;

  const segment = Math.floor(hueSegment);
  if (segment === 0) {
    r = 255 * (c + m);
    g = 255 * (x + m);
    b = 255 * m;
  } else if (segment === 1) {
    r = 255 * (x + m);
    g = 255 * (c + m);
    b = 255 * m;
  } else if (segment === 2) {
    r = 255 * m;
    g = 255 * (c + m);
    b = 255 * (x + m);
  } else if (segment === 3) {
    r = 255 * m;
    g = 255 * (x + m);
    b = 255 * (c + m);
  } else if (segment === 4) {
    r = 255 * (x + m);
    g = 255 * m;
    b = 255 * (c + m);
  } else {
    r = 255 * (c + m);
    g = 255 * m;
    b = 255 * (x + m);
  }

  const round = (channel: number) => Math.min(255, Math.max(0, Math.round(channel)));
  return [round(r), round(g), round(b)];
}

/**
 * Every row but row 0 — the seam row [meshCells] keeps literal — rotated
 * sideways by the same random amount, and mirrored left-to-right half the
 * time. The Kotlin comment is the design brief: a flip alone reads as a
 * mirror, a per-cell shuffle reads as a mosaic, and a cyclic shift keeps
 * every cell's exact neighbours so whatever gradient existed between them
 * survives untouched. Seeded, so the same artwork lands in the same
 * arrangement each time it is read.
 */
export function rotatedBelowSeam(cells: Rgb[], cols: number, rows: number, seed: number): Rgb[] {
  if (rows <= 1) return cells;
  const random = new JavaRandom(seed);
  const mirror = random.nextBoolean();
  const shift = random.nextInt(cols);
  const out = cells.slice();
  for (let row = 1; row < rows; row++) {
    const base = row * cols;
    for (let x = 0; x < cols; x++) {
      const src = mirror ? cols - 1 - x : x;
      out[base + x] = cells[base + ((src + shift) % cols)]!;
    }
  }
  return out;
}

/** Flat at both ends, so no cell boundary shows up as a crease once magnified. */
function smoothstep(t: number): number {
  const x = Math.min(1, Math.max(0, t));
  return x * x * (3 - 2 * x);
}

function lerpChannel(from: number, to: number, t: number): number {
  if (t <= 0) return from;
  if (t >= 1) return to;
  return Math.round(from + (to - from) * t);
}

/**
 * Smoothly resamples a cols×rows grid of control points up to a size×size
 * texture — Kotlin [IntArray.resampled], which is what lets the GPU blur be
 * narrow: the creases are resampled away before they are magnified.
 */
export function resampleCells(cells: Rgb[], cols: number, rows: number, size: number): MeshTexture {
  const data = new Uint8ClampedArray(size * size * 4);
  for (let ty = 0; ty < size; ty++) {
    const fy = ((ty + 0.5) / size) * rows - 0.5;
    const y0 = Math.min(rows - 1, Math.max(0, Math.floor(fy)));
    const y1 = Math.min(rows - 1, y0 + 1);
    const wy = smoothstep(fy - y0);
    for (let tx = 0; tx < size; tx++) {
      const fx = ((tx + 0.5) / size) * cols - 0.5;
      const x0 = Math.min(cols - 1, Math.max(0, Math.floor(fx)));
      const x1 = Math.min(cols - 1, x0 + 1);
      const wx = smoothstep(fx - x0);
      for (let channel = 0; channel < 3; channel++) {
        const top = lerpChannel(cells[y0 * cols + x0]![channel]!, cells[y0 * cols + x1]![channel]!, wx);
        const bottom = lerpChannel(cells[y1 * cols + x0]![channel]!, cells[y1 * cols + x1]![channel]!, wx);
        data[(ty * size + tx) * 4 + channel] = lerpChannel(top, bottom, wy);
      }
      data[(ty * size + tx) * 4 + 3] = 255;
    }
  }
  return { width: size, height: size, data };
}

/**
 * The mesh for the artwork's pixels, end to end: mean per cell, the one
 * liberty per cell, the rotation, the resample. `seed` is the artwork URL's
 * hash, exactly as the Kotlin caller passes it.
 */
export function meshOf(pixels: Uint8ClampedArray, width: number, height: number, seed: number): MeshTexture {
  const cols = Math.min(MESH_GRID, width);
  const rows = Math.min(MESH_GRID, height);
  const cells = meshCells(pixels, width, height).map(liftCell);
  const rotated = rotatedBelowSeam(cells, cols, rows, seed);
  return resampleCells(rotated, cols, rows, MESH_TEX);
}

/**
 * Three small box passes approximate a broad Gaussian blur at thumbnail
 * scale (Kotlin [ImageBitmap.boxBlurred]) — a sliding-window sum, so each
 * pass is linear in the pixels it touches. In place on [data]'s channels;
 * alpha is left at 255.
 */
export function boxBlur(data: Uint8ClampedArray<ArrayBuffer>, width: number, height: number, passes: number): Uint8ClampedArray<ArrayBuffer> {
  if (width < 2 || height < 2) return data;
  let source = data;
  let target = new Uint8ClampedArray(data.length);
  const radius = Math.max(2, Math.floor(Math.min(width, height) / 12));

  for (let pass = 0; pass < passes; pass++) {
    boxBlurPass(source, target, width, height, radius, true);
    boxBlurPass(target, source, width, height, radius, false);
  }
  return source;
}

function boxBlurPass(
  source: Uint8ClampedArray<ArrayBuffer>,
  target: Uint8ClampedArray<ArrayBuffer>,
  width: number,
  height: number,
  radius: number,
  horizontal: boolean,
): void {
  const major = horizontal ? width : height;
  const minor = horizontal ? height : width;
  const window = radius * 2 + 1;

  for (let fixed = 0; fixed < minor; fixed++) {
    let red = 0;
    let green = 0;
    let blue = 0;
    const pixel = (at: number): number => {
      const position = Math.min(major - 1, Math.max(0, at));
      return horizontal ? (fixed * width + position) * 4 : (position * width + fixed) * 4;
    };
    for (let offset = -radius; offset <= radius; offset++) {
      const at = pixel(offset);
      red += source[at]!;
      green += source[at + 1]!;
      blue += source[at + 2]!;
    }
    for (let moving = 0; moving < major; moving++) {
      const index = horizontal ? (fixed * width + moving) * 4 : (moving * width + fixed) * 4;
      target[index] = red / window;
      target[index + 1] = green / window;
      target[index + 2] = blue / window;
      target[index + 3] = 255;
      const leaving = pixel(moving - radius);
      const entering = pixel(moving + radius + 1);
      red += source[entering]! - source[leaving]!;
      green += source[entering + 1]! - source[leaving + 1]!;
      blue += source[entering + 2]! - source[leaving + 2]!;
    }
  }
}

/** A session's worth of meshes, at four kilobytes of texture each. */
const meshCache = new Map<string, MeshTexture>();
const MESH_CACHE_ENTRIES = 64;

/** A handful of recent covers; each entry is only ~57 KiB at 120×120 RGBA. */
const blurCache = new Map<string, BlurBitmap>();
const FULL_BLUR_CACHE_ENTRIES = 8;

function rememberCache<T>(cache: Map<string, T>, entries: number, key: string, value: T): void {
  cache.delete(key);
  cache.set(key, value);
  const oldest = cache.keys().next();
  if (cache.size > entries && !oldest.done) cache.delete(oldest.value);
}

/** Decodes [url] into RGBA pixels at [px], square, via a canvas readback. */
async function readPixels(
  url: string,
  px: number,
): Promise<{ pixels: Uint8ClampedArray<ArrayBuffer>; width: number; height: number }> {
  const image = new Image();
  image.crossOrigin = 'anonymous';
  image.referrerPolicy = 'no-referrer';
  image.src = url;
  await image.decode();
  const canvas = document.createElement('canvas');
  canvas.width = px;
  canvas.height = px;
  const context = canvas.getContext('2d', { willReadFrequently: true });
  if (!context) throw new Error('no 2d context');
  context.drawImage(image, 0, 0, px, px);
  const imagePixels = context.getImageData(0, 0, px, px);
  return { pixels: imagePixels.data as unknown as Uint8ClampedArray<ArrayBuffer>, width: px, height: px };
}

/** Whether [url]'s mesh is already in the cache — known synchronously, so the
 * caller can decide whether to hold what is on screen or expect a decode. */
export function hasMesh(url: string): boolean {
  return meshCache.has(url);
}

/** The same, for the full-cover blur. */
export function hasFullBlur(url: string): boolean {
  return blurCache.has(url);
}

/**
 * The mesh for [url], from and cached across calls. Returns null when the
 * artwork cannot be read at all (a broken URL) — the caller keeps whatever
 * colours were already on screen.
 */
export async function meshFor(url: string): Promise<MeshTexture | null> {
  const cached = meshCache.get(url);
  if (cached) return cached;
  try {
    const { pixels, width, height } = await readPixels(url, MESH_PX);
    const mesh = meshOf(pixels, width, height, hashUrl(url));
    rememberCache(meshCache, MESH_CACHE_ENTRIES, url, mesh);
    return mesh;
  } catch {
    return null;
  }
}

/**
 * The pre-blurred full-cover bitmap for [url], from the same decode the mesh
 * asks for — one fetch and one readback serve both layers.
 */
export async function fullBlurFor(url: string): Promise<BlurBitmap | null> {
  const cached = blurCache.get(url);
  if (cached) return cached;
  try {
    const { pixels, width, height } = await readPixels(url, MESH_PX);
    const blurred = boxBlur(pixels, width, height, FULL_BLUR_PASSES);
    const bitmap: BlurBitmap = { width, height, data: blurred };
    rememberCache(blurCache, FULL_BLUR_CACHE_ENTRIES, url, bitmap);
    return bitmap;
  } catch {
    return null;
  }
}
