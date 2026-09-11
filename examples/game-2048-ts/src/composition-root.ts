import { createCryptoRandomSource } from './adapters/secondary/crypto-random-source.ts';
import { createInMemoryBoardStore } from './adapters/secondary/in-memory-board-store.ts';
import type { MoveResult } from './domain/game.ts';
import type { Direction, GameState } from './domain/types.ts';
import type { BoardStore } from './ports/board-store.ts';
import type { RandomSource } from './ports/random-source.ts';
import { getGame } from './usecases/get-game.ts';
import { playMove } from './usecases/play-move.ts';
import { startGame } from './usecases/start-game.ts';

export interface Deps {
  readonly store: BoardStore;
  readonly random: RandomSource;
}

export interface Game2048 {
  startGame(gameId: string): GameState;
  playMove(gameId: string, direction: Direction): MoveResult;
  getGame(gameId: string): GameState | undefined;
}

/**
 * The only file that knows an adapter exists.
 *
 * `overrides` lets a test wire a fixed seed without importing an adapter, so
 * the real wiring gets a test.
 */
export function createGame2048(overrides: Partial<Deps> = {}): Game2048 {
  const store = overrides.store ?? createInMemoryBoardStore();
  const random = overrides.random ?? createCryptoRandomSource();

  return {
    startGame: (gameId: string) => startGame(store, random, gameId),
    playMove: (gameId: string, direction: Direction) => playMove(store, gameId, direction),
    getGame: (gameId: string) => getGame(store, gameId),
  };
}
