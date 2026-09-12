import { applyMove } from '../domain/game.ts';
import type { MoveResult } from '../domain/game.ts';
import type { Direction } from '../domain/types.ts';
import type { BoardStore } from '../ports/board-store.ts';

/** The store is written only when the outcome is 'moved'. */
export function playMove(
  store: BoardStore,
  gameId: string,
  direction: Direction,
): MoveResult {
  const current = store.load(gameId);
  if (current === undefined) {
    throw new Error(`no game with the id "${gameId}"`);
  }
  const result = applyMove(current, direction);
  if (result.outcome === 'moved') {
    store.save(gameId, result.state);
  }
  return result;
}
