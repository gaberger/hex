# Flappy Bird — hex-intf Example

A Flappy Bird clone built with hexagonal architecture (ports and adapters), demonstrating the **behavioral-specification-first** pipeline.

## Prerequisites

- [Bun](https://bun.sh/) >= 1.0
- Node.js >= 20 (for Vite)

## Quick Start

```bash
cd examples/flappy-bird
bun install
bun run dev
```

## Scripts

| Script | Description |
|--------|-------------|
| `bun run dev` | Start Vite dev server (port 3000) |
| `bun run build` | Production build to `dist/` |
| `bun run preview` | Preview production build |
| `bun test tests/` | Run all tests |
| `bun test tests/unit/` | Run unit tests only |
| `bun test tests/property/` | Run property-based tests only |
| `bun test tests/smoke/` | Run smoke tests only |
| `bun run typecheck` | TypeScript type checking |

## Architecture

```
src/
  core/
    ports/index.ts          # Interfaces: IGamePort, IRenderPort, IAudioPort, IStoragePort, IInputPort
    domain/
      physics.ts            # Pure physics functions (gravity, flap, collision)
      game-state.ts         # Immutable state transitions
    usecases/
      game-engine.ts        # Game engine orchestrating domain + ports
  adapters/
    primary/
      browser-input.ts      # Click/touch/keyboard input
    secondary/
      canvas-renderer.ts    # HTML5 Canvas 2D rendering
      browser-audio.ts      # Web Audio API oscillators
      localstorage-adapter.ts # localStorage for high scores
  main.ts                   # Composition root + game loop

specs/
  behavioral-specs.md       # Acceptance criteria (written BEFORE code)

tests/
  unit/                     # Unit tests for domain functions
  property/                 # Property-based tests for invariants
  smoke/                    # Integration smoke tests simulating gameplay
```

## Behavioral Specs Drive Validation

The `specs/behavioral-specs.md` file defines **what the game does** before any code is written. Each spec (BS-1 through BS-13) becomes an acceptance criterion:

- **Unit tests** reference spec IDs (e.g., "BS-4: ceiling does NOT kill")
- **Property tests** verify invariants (e.g., "flap always produces negative velocity")
- **Smoke tests** simulate real play sequences (e.g., "flap 3 times, bird stays alive")

This pipeline prevents the three bugs found in the original implementation:

1. **Double-negation bug** -- caught by property test "applyFlap always returns negative"
2. **Ceiling-kills bug** -- caught by spec BS-4 driving the correct test assertion
3. **State-race bug** -- caught by smoke test "first flap transitions AND applies velocity"

## Sign Convention

| Quantity | Sign | Meaning |
|----------|------|---------|
| Y-axis | + downward | Screen coordinates |
| Gravity | + (980) | Accelerates bird downward |
| Flap strength | - (-280) | Sets velocity upward |
| Velocity > 0 | falling | Bird descending |
| Velocity < 0 | rising | Bird ascending |
