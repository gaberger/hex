# Build spec — 2048 game core in TypeScript (hexagonal)

Target: `examples/game-2048-ts`.
Gate: `npm test`.
Status: synthesized from Design 0 and Design 1 plus both red-team reports.
This spec is disposable. The gate is the contract.

## 0. What you build

You build the rules of 2048, and nothing else.

Think of a board game in a box. This is the rulebook and the pieces. It is
not the table, the players, or the room. There is no screen, no mouse, and
no network.

## 1. The gate, fixed first

The committed gate could not run. That was fatal, and both red teams found
it. The old test script used `node --test --experimental-strip-types`. That
flag needs Node 22.6 or higher. This machine has Node 20.20.2 on the PATH
and Node 18.19.1 at `/usr/bin/node`. No Node 22 is installed.

The new gate compiles first, then runs the compiled tests:

    "test": "rm -rf dist && tsc && node --test dist/*.test.js"

The gate keeps its meaning: the types must check, AND every test must pass.
Only the runner changed. Four properties are proven by experiment, not by
argument:

1. Correct code exits 0.
2. A wrong expectation exits 1.
3. An `enum` or a bad type-import exits 2, at compile time.
4. An empty `src/` exits 2. The vacuous gate is closed.

`rm -rf dist` is not decoration. Without it, a deleted test file keeps
passing from its old compiled copy. This was measured.

`tsconfig.json` now sets four flags that turn prose rules into build errors:

| Flag | What it stops |
|---|---|
| `allowImportingTsExtensions` | lets you write `./row.ts`, as the challenge demands |
| `rewriteRelativeImportExtensions` | rewrites `.ts` to `.js` on emit, so the import works at runtime |
| `erasableSyntaxOnly` | stops `enum`, `namespace`, and constructor parameter properties |
| `verbatimModuleSyntax` | forces `import type` for types |

Do not change these flags. They are the reason the build cannot drift.

## 2. What was cut, and why

Design 0 shipped a ledger, per-move records, version numbers, optimistic
commits, a per-game queue, and crash replay. Design 1 shipped versions,
compare-and-swap, a dedupe ring, and a retry budget.

All of it is deleted.

The challenge asks for one player and an in-memory store. A crash-recovery
machine cannot help a store that dies with the process. Design 1's own
test 6 was impossible under its own retry budget: 50 racing moves need up
to 49 retries each, the budget was 8, so 41 moves vanish.

The replacement is smaller and stronger. **Nothing in this program is
async.** There is no `await`, so no task can cut in half way through a
change. A version check has nothing to protect against. Section 6 gives
the test that keeps this true.

Five ideas from the designs survive, because they are good:

1. The flat 16-cell grid.
2. Merging by consuming the partner index, not by a "did merge" flag.
3. The seed lives inside the state, and the next seed is a pure function.
4. Game-over checks right and down only.
5. Test files sit flat in `src/`.

## 3. The data model

**The grid is one flat list of 16 numbers.** Not four lists inside a list.

Think of a street of 16 houses, numbered 0 to 15. House `r * 4 + c` is row
`r`, column `c`. One copy, one compare, one freeze. No inner row can leak
between two boards.

    export type Cell = number;            // 0 = empty, else a power of two >= 2
    export type Grid = readonly Cell[];   // exactly 16 cells
    export type Row = readonly [Cell, Cell, Cell, Cell];
    export type Direction = 'left' | 'right' | 'up' | 'down';
    export type GameStatus = 'playing' | 'over';

    export interface GameState {
      readonly grid: Grid;
      readonly score: number;
      readonly seed: number;       // unsigned 32-bit
      readonly moveCount: number;
      readonly status: GameStatus;
      readonly won: boolean;
    }

`Direction` and `GameStatus` are string unions. They are not enums.
`erasableSyntaxOnly` rejects an enum, so this rule cannot be forgotten.

### Freezing is deep, or it is a lie

`Object.freeze` is shallow. Both red teams broke the designs here in three
lines. A sealed envelope does not protect a loose letter inside it.

Write one helper, `freezeState`, and call it at every point that makes a
state:

1. `Object.freeze(state.grid)`.
2. `Object.freeze(state)`.

Both calls. Every time. Section 6 test 15 proves a write throws.

## 4. The public API

### 4.1 `src/domain/row.ts`

    export function slideRowLeft(row: Row): { row: Row; gained: number }

This is the heart. Everything else reuses it.

Picture books on a shelf. You push them all to the left end. Two equal
books glue into one thicker book. A thicker book cannot glue again in the
same push.

Three steps:

1. **Squeeze.** Copy the non-zero cells into a short list, in order.
2. **Merge.** Walk left to right. If this cell equals the next cell, write
   their sum, add that sum to `gained`, then **step forward by two**.
3. **Pad.** Fill with zeros up to four cells.

Step 2 holds the whole correctness story. Do not use a "already merged"
flag. Consume the partner by skipping its index. A merged cell is written
to the output and never read again in this pass. The rule cannot merge
twice, because nothing is left to merge with.

`gained` is the sum of the **merged results**, not of the inputs. `[2,2]`
scores 4, not 2. That is the classic scoring bug.

### 4.2 `src/domain/grid.ts`

    export const EMPTY_GRID: Grid;
    export const LANES: Readonly<Record<Direction, readonly (readonly number[])[]>>;
    export function gridEquals(a: Grid, b: Grid): boolean;
    export function emptyCells(grid: Grid): readonly number[];
    export function hasTile(grid: Grid, value: number): boolean;

You write the slide once. You get four directions by reading the board
through four fixed lists of positions.

Picture a tray of glasses. You do not learn four pushes. You turn the
tray, and you push left.

`LANES` holds four lanes per direction, and four positions per lane:

- `left`, lane `r`: `[4r, 4r+1, 4r+2, 4r+3]`
- `right`: the same, reversed
- `up`, lane `c`: `[c, c+4, c+8, c+12]`
- `down`: the same, reversed

A move reads four cells along a lane, calls `slideRowLeft`, and writes the
result back along the same lane. This is index arithmetic only.

One wrong lane table is invisible to a merge test. Section 6 test 9 checks
every table against a hand-written board, and test 10 checks that each
table is a permutation of 0 to 15.

### 4.3 `src/domain/rng.ts`

    export function nextSeed(seed: number): number;
    export function randomBelow(seed: number, bound: number): { value: number; seed: number };

The seed lives in the state. The mixer is pure. Use a `mulberry32` style
32-bit mixer: multiply, shift, exclusive-or. Do not use `BigInt`. Keep the
result unsigned with `>>> 0`.

`randomBelow` returns a whole number from 0 to `bound - 1`, plus the next
seed. Compute it as `Math.floor((r / 2 ** 32) * bound)`.

**`randomBelow` must throw when `bound` is less than 1.** A red team showed
that a silent `nextInt(0)` returns 0 and then overwrites a real tile. One
`if` closes it.

### 4.4 `src/domain/game.ts`

    export function newGame(seed: number): GameState;
    export function spawnTile(grid: Grid, seed: number):
      { grid: Grid; seed: number; index: number; value: number } | null;
    export function isGameOver(grid: Grid): boolean;
    export function applyMove(state: GameState, direction: Direction): MoveResult;

    export type MoveOutcome = 'moved' | 'blocked' | 'rejected';
    export interface MoveResult {
      readonly state: GameState;
      readonly outcome: MoveOutcome;
      readonly gained: number;
      readonly spawnedAt: number | null;
      readonly spawnedValue: number | null;
    }

`MoveResult` tells the caller what the move did. A caller must never have
to compare two boards to find out.

**`spawnTile` never loops.** It builds the list of empty cells in rising
order, draws one index, then draws the value. It returns `null` when there
is no empty cell. Guess-and-retry hangs forever on a full board, so it is
banned.

The two draws happen in a fixed order: **first the cell, then the value.**
Determinism tests lock this order in, so write it down once and keep it.

**The value rule:** draw `randomBelow(seed, 10)`. A 0 gives a 4. Anything
else gives a 2. So a 4 appears one time in ten.

`isGameOver(grid)` answers in this order, and stops at the first "no":

1. Is any cell empty? Then it is not over.
2. Does any cell equal its right neighbour? Then it is not over.
3. Does any cell equal its lower neighbour? Then it is not over.
4. Otherwise it is over.

Left and up ask the same question a second time. Do not check them.

**`applyMove` runs these steps, in this exact order:**

1. If `state.status` is `'over'`, return `outcome: 'rejected'` with the
   same state. A dead game accepts no moves.
2. Slide all four lanes for the direction. Collect the new grid and the
   total `gained`.
3. If the new grid equals the old grid, return `outcome: 'blocked'` with
   the same state, `gained: 0`, and no spawn. Nothing else changes.
4. Spawn one tile on the new grid. A changed grid always has an empty
   cell, so `null` here is a programmer error. Throw on it.
5. Add `gained` to the score. Add 1 to `moveCount`. Set `won` to
   `state.won || hasTile(gridAfterSpawn, 2048)`.
6. Set `status` from `isGameOver(gridAfterSpawn)`.
7. Freeze and return `outcome: 'moved'`.

Step 6 runs **after** the spawn, never before. A spawn that fills the last
cell can end the game. Check it too early and you call a dead board
`playing`.

`newGame(seed)` starts from `EMPTY_GRID` and spawns **two** tiles. Each
spawn advances the seed. Score is 0, `moveCount` is 0, `won` is false, and
`status` is `'playing'`.

`won` is a sticky flag, not a status. Play continues after 2048. A won game
is never re-lost.

### 4.5 `src/ports/`

    // random-source.ts
    export interface RandomSource { nextSeed(): number; }

    // board-store.ts
    export interface BoardStore {
      load(gameId: string): GameState | undefined;
      save(gameId: string, state: GameState): void;
      remove(gameId: string): void;
    }

Both ports are synchronous. This is the correctness strategy, so it is not
a detail.

`RandomSource` is a one-shot, not a stream. It gives one unpredictable
starting seed to a new game. Every later draw is pure, and comes from the
seed inside the state. So a caller can replay any game exactly.

`BoardStore` has `remove` because a `Map` with no delete leaks every game
for the life of the process. Both designs missed this.

`ports/` imports value types from `domain/` only. That is allowed.

### 4.6 `src/adapters/secondary/`

- `in-memory-board-store.ts` — a `Map<string, GameState>` behind the three
  methods. No `await`, no clone. It stores frozen values, so a shared
  reference is safe.
- `crypto-random-source.ts` — one call to `crypto.getRandomValues` on a
  `Uint32Array` of length 1.

An adapter imports from `ports/` only. An adapter never imports another
adapter.

### 4.7 `src/composition-root.ts`

    export interface Deps { store: BoardStore; random: RandomSource; }
    export interface Game2048 {
      startGame(gameId: string): GameState;
      playMove(gameId: string, direction: Direction): MoveResult;
      getGame(gameId: string): GameState | undefined;
    }
    export function createGame2048(overrides?: Partial<Deps>): Game2048;

This is the only file that names an adapter. `overrides` lets a test wire a
fixed seed without importing an adapter, so the real wiring is finally
covered by a test. A red team called this out: can a user actually start
the thing?

### 4.8 `src/usecases/`

- `start-game.ts` — `startGame(deps, gameId)`. Throw if the id exists.
  Draw one seed. Call `newGame`. Save. Return the state.
- `play-move.ts` — `playMove(deps, gameId, direction)`. Load, or throw if
  missing. Call `applyMove`. **Save only when the outcome is `'moved'`.**
  Return the `MoveResult`.
- `get-game.ts` — `getGame(deps, gameId)`. Load and return.

The caller supplies `gameId`. The program never invents one. An id made by
`crypto.randomUUID()` inside the root is unseeded luck outside any port,
and it breaks replay.

A use case imports `domain/` and `ports/` only.

## 5. The correctness strategy, in one paragraph

There is no concurrency to manage, so do not build a system to manage it.
Every value is frozen, and every rule is a pure function. The only change
to the world is one `Map.set` inside the in-memory store. No function in
the program is `async`, so no task can stop half way through that change.
A reviewer can check the whole claim with one command:
`grep -rnE '\basync\b|\bawait\b' src/` must print nothing. Test 20 makes
that a gate failure instead of a hope.

## 6. The test plan

All test files sit **flat** in `src/`. The gate globs `dist/*.test.js`,
which is not recursive. A test in `src/domain/` compiles to `dist/domain/`
and never runs. A gate that silently tests nothing is worse than no gate.

Fakes live inside the test files. Do not add a fake adapter file.

### The four the challenge names

1. **A row slides left.** `[0,2,0,4]` gives `[2,4,0,0]`.
2. **Two equal tiles merge once, not twice.** `[2,2,2,2]` gives
   `[4,4,0,0]`. Assert it is **not** `[8,0,0,0]`.
3. **A move that changes nothing.** A board already packed left, moved
   left, returns `outcome: 'blocked'`, the same grid, the same score, and
   the same seed. Assert the fake store's own write counter did not move.
4. **Game over on a full board.** A full board with no equal neighbours
   reports `over`. Change one tile to make a pair, and assert `playing`.

### Merge direction, the second-most-common 2048 bug

`[2,2,2,2]` is symmetric. It cannot tell a leftmost-first rule from a
rightmost-first rule. These cases can:

5. `[0,2,2,2]` left gives `[4,2,0,0]`. The **left** pair merges. This is
   the single best merge test.
6. `[2,2,2,0]` right gives `[0,0,2,4]`. The **right** pair merges.
7. `[4,2,2,0]` left gives `[4,4,0,0]`.
8. `[2,2,4,4]` left gives `[4,8,0,0]`, not `[16,0,0,0]`.

For each case, assert the wrong answer is absent, not only that the right
answer is present.

### The lane tables

9. **Four directions against a hand-written board.** Take one asymmetric
   board. Write the expected 16 cells for left, right, up, and down, by
   hand, in the test file. A backwards `down` table passes every merge
   test, so this is the only thing that catches it.
10. **Each lane table is a permutation of 0 to 15.** Sort the 16 indexes
    and compare to `[0..15]`.

### Independent oracles, done properly

A red team measured that random grids are game-over 13 times in 100,000.
A random-grid property test is a green light attached to nothing. So:

11. **Build full, no-merge boards on purpose.** Fill the board with a
    pattern where no neighbour repeats, for example the repeating rows
    `2 4 8 16` / `4 2 16 8`. Assert `over`. Then lower one cell to make a
    pair, and assert `playing`.
12. **A hand-written verdict table.** Six boards, six verdicts, written by
    a person. Do not build the second opinion out of `applyMove`, because
    `applyMove` is the code under test.
13. **The sum rule.** After a move, the total of all tiles equals the
    total before, plus the spawned value. Merges never change the total.
    Keep this test, and know its limit: it cannot tell left from right,
    and it cannot see a wrong merge pair. It is a net, not the defence.
14. **Determinism.** The same starting seed and the same move list give
    byte-identical boards and scores, over ten runs.

### The claims that must not rot

15. **A frozen grid throws.** `state.grid[0] = 99` must throw a
    `TypeError`. Modules are strict mode, so it will, once the grid is
    frozen too.
16. **Spawn on a full board returns `null`.** It does not hang and it does
    not throw.
17. **`randomBelow(seed, 0)` throws.**
18. **A game that is `over` rejects a move.** The outcome is `'rejected'`
    and the state is unchanged.
19. **The store forgets.** `remove` makes `load` return `undefined`.
20. **No `async`, no `await`.** Read every `.ts` file under `src/` and
    assert the words do not appear outside a comment. This converts the
    correctness strategy into a gate.
21. **The composition root starts.** Import `createGame2048`, wire a fixed
    seed through `overrides`, start a game, play ten moves, and assert the
    score and grid are sane. This is the "can a user start the thing"
    test.
22. **Score after a merge.** `[2,2,0,0]` left gives `gained` 4. Not 2.
23. **A new game has exactly two tiles**, each a 2 or a 4.

## 7. Build order

1. `domain/row.ts` and its tests. No port exists yet, and none is needed.
2. `domain/grid.ts` with the four lane tables. Tests 9 and 10.
3. `domain/rng.ts`. Test 17.
4. `domain/game.ts`. Tests 1 to 8, 11 to 18, 22, 23.
5. `ports/`.
6. `adapters/secondary/`. Test 19.
7. `usecases/`.
8. `composition-root.ts`. Test 21.
9. Test 20 last, because it guards every file.

Run `npm test` after each step.

## 8. Known limits, said out loud

1. **The in-memory store does not survive the process.** It is not a
   database and it does not pretend to be one. A file adapter is a later
   job with a gate of its own.
2. **One process only.** Two Node worker threads on one board are out of
   scope. The port is the seam if that ever changes.
3. **`randomBelow` has a tiny modulo bias.** With a bound of 16 or 10 it
   is far below anything a player can see. It is written here so nobody
   spends an afternoon on it.
4. **Replay is exact only while the rules are fixed.** A saved seed
   replays the same luck. It does not protect you from a merge rule you
   change next month.
