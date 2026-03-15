# Flappy Bird — hex-intf Example

A complete Flappy Bird game demonstrating hexagonal architecture with the hex-intf framework. Pure domain logic is fully testable with zero browser dependencies.

## Prerequisites

- bun (or node >= 20)
- modern browser
- git

## Quick Start

```bash
cd examples/flappy-bird
bun install
bun run dev
```

This opens the game in your browser at http://localhost:5173.

## Available Scripts

| Command | Description |
|---------|-------------|
| `bun install` | Install dependencies |
| `bun run dev` | Start Vite dev server with HMR (port 5173) |
| `bun run build` | Build for production to `dist/` |
| `bun run preview` | Preview production build |
| `bun test` | Run 30 unit tests (physics, game state, game engine) |

## How to Play

- **Click**, **tap**, or press **Space** to flap
- Avoid the pipes
- Score increments each time you pass through a gap
- High score persists in localStorage

## Architecture

This project uses [hex-intf](https://github.com/ruvnet/hex-intf) hexagonal architecture:

```
src/
├── core/
│   ├── ports/index.ts        # 5 ports: Game, Render, Audio, Storage, Input
│   ├── domain/
│   │   ├── physics.ts        # Pure functions: gravity, collision, pipe gen
│   │   └── game-state.ts     # Immutable state transitions
│   └── usecases/
│       └── game-engine.ts    # Orchestrates game loop via port injection
├── adapters/
│   ├── primary/
│   │   └── browser-input.ts  # Click/touch/spacebar listener
│   └── secondary/
│       ├── canvas-renderer.ts     # HTML5 Canvas 2D rendering
│       ├── browser-audio.ts       # Web Audio oscillator beeps
│       └── localstorage-adapter.ts # High score persistence
└── main.ts                   # Composition root + game loop
```

### Why This Architecture?

- **Domain is 100% testable**: `physics.ts` and `game-state.ts` have zero browser deps — tests run in Bun in 8ms
- **Adapters are swappable**: Replace `canvas-renderer.ts` with a WebGL or Phaser adapter without touching game logic
- **Composition root is the only wiring point**: `main.ts` is the single file that imports both ports and adapters

## Tests

```bash
bun test
```

30 tests covering:
- **Physics** (13 tests): gravity, flap, collision detection, bounds checking, pipe generation
- **Game State** (10 tests): immutable transitions, scoring, game over, reset
- **Game Engine** (7 tests): use case with mocked audio/storage ports
