/**
 * One unpredictable starting seed, not a stream. Every later draw is pure, so
 * the adapter is asked once per game and never again.
 */
export interface RandomSource {
  nextSeed(): number;
}
