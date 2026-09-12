/**
 * A mulberry32 mixer. Pure: the same seed always gives the same next seed.
 * Everything stays unsigned 32-bit with `>>> 0`, so there is no BigInt.
 */
export function nextSeed(seed: number): number {
  let t = (seed + 0x6d2b79f5) >>> 0;
  t = Math.imul(t ^ (t >>> 15), t | 1) >>> 0;
  t = (t ^ (t + Math.imul(t ^ (t >>> 7), t | 61))) >>> 0;
  return (t ^ (t >>> 14)) >>> 0;
}

export interface Draw {
  /** In the range 0 (included) to `bound` (excluded). */
  readonly value: number;
  /** The seed to carry into the next draw. */
  readonly seed: number;
}

/**
 * Draw one number below `bound`.
 *
 * A bound under 1 throws. A silent 0 would return position 0 every time and
 * overwrite a real tile, and nothing would report the fault.
 */
export function randomBelow(seed: number, bound: number): Draw {
  if (!Number.isInteger(bound) || bound < 1) {
    throw new RangeError(`randomBelow needs a whole bound of 1 or more, got ${bound}`);
  }
  const next = nextSeed(seed);
  return { value: next % bound, seed: next };
}
