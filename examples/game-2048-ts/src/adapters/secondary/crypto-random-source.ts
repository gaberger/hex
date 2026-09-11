import type { RandomSource } from '../../ports/random-source.ts';

/** One `crypto.getRandomValues` call gives the unsigned 32-bit starting seed. */
export function createCryptoRandomSource(): RandomSource {
  return {
    nextSeed(): number {
      const buffer = new Uint32Array(1);
      crypto.getRandomValues(buffer);
      return buffer[0] >>> 0;
    },
  };
}
