import { newGame } from '../domain/game.ts';
import type { GameState } from '../domain/types.ts';
import type { BoardStore } from '../ports/board-store.ts';
import type { RandomSource } from '../ports/random-source.ts';

/** The caller supplies the id. The program never invents one. */
export function startGame(
  store: BoardStore,
  random: RandomSource,
  gameId: string,
): GameState {
  if (store.load(gameId) !== undefined) {
    throw new Error(`a game with the id "${gameId}" already exists`);
  }
  const state = newGame(random.nextSeed());
  store.save(gameId, state);
  return state;
}
