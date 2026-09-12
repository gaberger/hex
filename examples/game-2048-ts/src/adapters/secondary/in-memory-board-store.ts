import type { BoardStore, GameState } from '../../ports/board-store.ts';

/** A Map behind the store. It dies with the process, and does not pretend otherwise. */
export function createInMemoryBoardStore(): BoardStore {
  const games = new Map<string, GameState>();
  return {
    load(gameId: string): GameState | undefined {
      return games.get(gameId);
    },
    save(gameId: string, state: GameState): void {
      games.set(gameId, state);
    },
    remove(gameId: string): void {
      games.delete(gameId);
    },
  };
}
