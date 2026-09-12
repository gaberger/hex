import type { GameState } from '../domain/types.ts';

export type { GameState };

/**
 * Where a game is kept. Every call returns its answer at once, so no task can
 * cut in half way through a change.
 */
export interface BoardStore {
  load(gameId: string): GameState | undefined;
  save(gameId: string, state: GameState): void;
  /** Without this, a Map keeps every game ever played for the life of the process. */
  remove(gameId: string): void;
}
