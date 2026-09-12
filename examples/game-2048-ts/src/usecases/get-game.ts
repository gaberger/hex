import type { GameState } from '../domain/types.ts';
import type { BoardStore } from '../ports/board-store.ts';

export function getGame(store: BoardStore, gameId: string): GameState | undefined {
  return store.load(gameId);
}
