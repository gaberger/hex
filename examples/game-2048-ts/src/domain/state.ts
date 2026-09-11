import type { GameState } from './types.ts';

/**
 * Object.freeze is shallow, so a frozen state with a loose grid is not frozen.
 * Every state leaves the domain through here, and both calls happen every time.
 */
export function freezeState(state: GameState): GameState {
  Object.freeze(state.grid);
  return Object.freeze(state);
}
