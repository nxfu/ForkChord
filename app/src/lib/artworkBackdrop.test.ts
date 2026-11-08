import { describe, expect } from 'vitest';
import { it } from 'vitest';
import {
  JavaRandom,
  boxBlur,
  hashUrl,
  hslToRgb,
  liftCell,
  meshCells,
  meshOf,
  rgbToHsl,
  rotatedBelowSeam,
  resampleCells,
} from './artworkBackdrop';

/**
 * Reference values produced by the algorithms the Kotlin app actually runs:
 * java.util.Random's LCG (kotlin.random.Random(seed) on the JVM) and Java
 * String.hashCode, computed independently in Python. The backdrop must land
 * in the same arrangement as the phone's for the same artwork.
 */
describe('hashUrl', () => {
  it('matches Java String.hashCode', () => {
    expect(hashUrl('')).toBe(0);
    expect(hashUrl('hello')).toBe(99162322);
    expect(hashUrl('https://lh3.googleusercontent.com/xyz=w120-h120-l90-rj')).toBe(1564953380);
  });
});

describe('JavaRandom', () => {
  it('reproduces java.util.Random(seed)', () => {
    const random = new JavaRandom(42);
    // seed42: bool=True int6=3 bool=True int6=2 bool=False int6=1 …
    expect([random.nextBoolean(), random.nextInt(6)]).toStrictEqual([true, 3]);
    expect([random.nextBoolean(), random.nextInt(6)]).toStrictEqual([true, 2]);
    expect([random.nextBoolean(), random.nextInt(6)]).toStrictEqual([false, 1]);
    expect([random.nextBoolean(), random.nextInt(6)]).toStrictEqual([false, 2]);
    expect([random.nextBoolean(), random.nextInt(6)]).toStrictEqual([true, 5]);
    expect([random.nextBoolean(), random.nextInt(6)]).toStrictEqual([true, 2]);
  });

  it('reproduces a negative seed', () => {
    const random = new JavaRandom(-12345);
    expect(random.nextBoolean()).toBe(true);
    expect(random.nextInt(6)).toBe(5);
    expect(random.nextInt(6)).toBe(4);
    expect(random.nextInt(6)).toBe(2);
  });

  it('stays in range', () => {
    const random = new JavaRandom(7);
    for (let i = 0; i < 200; i++) {
      const value = random.nextInt(6);
      expect(value).toBeGreaterThanOrEqual(0);
      expect(value).toBeLessThan(6);
    }
  });
});

describe('meshCells', () => {
  it('flips rows: cell row 0 is the source\u2019s bottom edge', () => {
    // 6×1 pixels, one per grid row: 0 white, 1..5 black.
    const width = 6;
    const height = 6;
    const pixels = new Uint8ClampedArray(width * height * 4);
    for (let y = 0; y < height; y++) {
      for (let x = 0; x < width; x++) {
        const value = y === 0 ? 255 : 0;
        const at = (y * width + x) * 4;
        pixels[at] = value;
        pixels[at + 1] = value;
        pixels[at + 2] = value;
        pixels[at + 3] = 255;
      }
    }
    const cells = meshCells(pixels, width, height);
    expect(cells).toHaveLength(6 * 6);
    // Grid row 0 holds the FLIPPED read: the white row (source y=0) ends up in
    // grid row 5, the bottom, because grid rows run bottom-up.
    expect(cells[0]).toStrictEqual([0, 0, 0]);
    expect(cells[5 * 6]).toStrictEqual([255, 255, 255]);
  });

  it('keeps proportions: a quarter of the frame stays a quarter of the mesh', () => {
    // Left half white, right half black — six columns of means must come out
    // 3 white, 3 black.
    const width = 6;
    const height = 1;
    const pixels = new Uint8ClampedArray(width * height * 4);
    for (let x = 0; x < width; x++) {
      const value = x < 3 ? 255 : 0;
      pixels[x * 4] = value;
      pixels[x * 4 + 1] = value;
      pixels[x * 4 + 2] = value;
      pixels[x * 4 + 3] = 255;
    }
    const cells = meshCells(pixels, width, height);
    expect(cells.slice(0, 3).every(([r]) => r === 255)).toBe(true);
    expect(cells.slice(3).every(([r]) => r === 0)).toBe(true);
  });
});

describe('rotatedBelowSeam', () => {
  const grid = [
    [10, 0, 0],
    [20, 0, 0],
    [30, 0, 0],
    [40, 0, 0],
    [50, 0, 0],
    [60, 0, 0],
    [70, 0, 0],
    [80, 0, 0],
    [90, 0, 0],
  ] as [number, number, number][];
  const cols = 3;
  const rows = 3;

  it('leaves the seam row literal', () => {
    const rotated = rotatedBelowSeam(grid, cols, rows, 1);
    expect(rotated.slice(0, 3)).toStrictEqual(grid.slice(0, 3));
  });

  it('is a cyclic shift: neighbours survive, content moves', () => {
    const rotated = rotatedBelowSeam(grid, cols, rows, 1);
    // Row 1's three cells must be exactly the source row 1's cells, carried
    // around the row.
    expect([...rotated.slice(3, 6)].sort((a, b) => a[0] - b[0])).toStrictEqual(
      [...grid.slice(3, 6)].sort((a, b) => a[0] - b[0]),
    );
  });

  it('is seeded: the same artwork lands in the same arrangement', () => {
    expect(rotatedBelowSeam(grid, cols, rows, 42)).toStrictEqual(rotatedBelowSeam(grid, cols, rows, 42));
  });
});

describe('resampleCells', () => {
  it('keeps the control points at the texel centres', () => {
    const cells: [number, number, number][] = [
      [255, 0, 0],
      [0, 255, 0],
      [0, 0, 255],
      [255, 255, 0],
    ];
    const { data, width, height } = resampleCells(cells, 2, 2, 4);
    expect(width).toBe(4);
    expect(height).toBe(4);
    // Texel (0,0) maps to fy=fx=-0.125 — clamped to the corner control point,
    // red, exactly.
    expect(data[0]).toBe(255);
    expect(data[1]).toBe(0);
    expect(data[2]).toBe(0);
  });
});

describe('HSL, as the Kotlin ColorUtils spells it', () => {
  it('round-trips colours', () => {
    for (const [r, g, b] of [
      [255, 0, 0],
      [0, 255, 0],
      [0, 0, 255],
      [128, 128, 128],
      [255, 255, 255],
      [0, 0, 0],
      [180, 40, 60],
    ] as [number, number, number][]) {
      const [h, s, l] = rgbToHsl(r, g, b);
      expect(hslToRgb(h, s, l)).toStrictEqual([r, g, b]);
    }
  });

  it('lifts saturation and floors lightness, nothing else', () => {
    // A dull maroon: the mean of a red stripe and the black around it.
    const [r, g, b] = liftCell([120, 20, 20]);
    const [, s] = rgbToHsl(r, g, b);
    const [, originalS] = rgbToHsl(120, 20, 20);
    expect(s).toBeCloseTo(Math.min(1, originalS * 1.12), 5);
    // Pure black gets the floor, so the blur finds an edge. The channel
    // round-trip quantises (255 * 0.045 rounds to 11), so the floor is
    // asserted through the colour itself, as the Kotlin code's does.
    const [br, bg, bb] = liftCell([0, 0, 0]);
    expect([br, bg, bb]).toStrictEqual([11, 11, 11]);
  });
});

describe('boxBlur', () => {
  it('keeps a uniform field uniform', () => {
    const width = 12;
    const height = 12;
    const data = new Uint8ClampedArray(width * height * 4).fill(0);
    for (let i = 0; i < width * height; i++) {
      data[i * 4] = 200;
      data[i * 4 + 1] = 100;
      data[i * 4 + 2] = 50;
      data[i * 4 + 3] = 255;
    }
    const blurred = boxBlur(data, width, height, 3);
    for (let i = 0; i < width * height; i++) {
      expect([blurred[i * 4], blurred[i * 4 + 1], blurred[i * 4 + 2]]).toStrictEqual([200, 100, 50]);
    }
  });

  it('smears an impulse into its neighbours', () => {
    const width = 24;
    const height = 24;
    const data = new Uint8ClampedArray(width * height * 4);
    for (let i = 0; i < width * height; i++) data[i * 4 + 3] = 255;
    const centre = (12 * width + 12) * 4;
    data[centre] = 255;
    const blurred = boxBlur(data, width, height, 3);
    // The impulse itself is dimmed, its neighbours lifted — and the reach is
    // bounded by the radius (max(2, min(24, 24) / 12) = 2, three passes → 6px).
    expect(blurred[centre]!).toBeLessThan(255);
    expect(blurred[centre + 4]!).toBeGreaterThan(0);
    expect(blurred[centre + 3 * width * 4]!).toBeGreaterThan(0);
    expect(blurred[centre + 10 * width * 4]!).toBe(0);
  });
});

describe('meshOf', () => {
  it('produces a MESH_TEX texture of opaque pixels', () => {
    const width = 120;
    const height = 120;
    const pixels = new Uint8ClampedArray(width * height * 4);
    for (let i = 0; i < width * height; i++) {
      // A nine-tenths black cover with a red stripe: the mesh must stay
      // mostly black, the property the whole port exists for.
      const red = i % width > 108 ? 220 : 10;
      pixels[i * 4] = red;
      pixels[i * 4 + 1] = 5;
      pixels[i * 4 + 2] = 5;
      pixels[i * 4 + 3] = 255;
    }
    const mesh = meshOf(pixels, width, height, hashUrl('test'));
    expect(mesh.width).toBe(32);
    expect(mesh.height).toBe(32);
    let dark = 0;
    let brightest = 0;
    for (let i = 0; i < 32 * 32; i++) {
      if (mesh.data[i * 4]! < 100) dark++;
      brightest = Math.max(brightest, mesh.data[i * 4]!);
    }
    // The stripe is nine percent of the cover, and a 6-wide grid can only put
    // it in whole cells: one column of six turns bright, plus the interpolation
    // band around it. Three quarters of the surface staying dark is the
    // proportion surviving; a quantiser's answer would have painted all of it.
    expect(dark).toBeGreaterThanOrEqual(32 * 32 * 0.75);
    // …and the stripe itself is still there: structure, not a flat wash.
    expect(brightest).toBeGreaterThan(100);
  });
});
