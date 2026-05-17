# hex-life

Conway's Game of Life on a **hexagonal** grid (6 neighbours per cell), implemented as a hexagonal-architecture example.

```
cargo run -p hex-life
```

Press Enter to step one generation. `n 20` to advance 20. `r` to reset. `q` to quit.

## Why this exists

Demonstrates that the hex-architecture rules enforced by `hex analyze` work for non-trivial software, not just for hex-nexus itself. Built tonight (2026-05-09) as a proof artifact alongside the typed-tool-library SOP work (ADR-2605082500). The whole example was built inside hex's own `hex analyze` 100/100 grade — zero boundary violations.

## Rules — Bays B2/S34 hexagonal Life

- **Birth:** a dead cell with **exactly 2** live neighbours becomes alive.
- **Survive:** a live cell with **3 or 4** live neighbours stays alive.
- **Die:** otherwise.

Carter Bays' rule for hexagonal Life — produces gliders and oscillators the way Conway's B3/S23 does on a square grid.

## Architecture

```
src/
  domain/             pure rules + types, ZERO external deps
    coord.rs            axial (q, r) hex coordinate, 6-neighbour function
    cell.rs             Alive | Dead
    grid.rs             sparse Grid (only live cells stored)
    rules.rs            tick(&Grid) -> Grid  (the B2/S34 rule)
  ports/              typed contracts
    display.rs          IDisplayPort
    input.rs            IInputPort + Command enum
  usecases/           composes domain + ports, no I/O
    game_loop.rs        GameLoop::run(input, display) loop
  adapters/           concrete I/O
    primary/cli.rs      stdin reader, parses Step / StepN(N) / Reset / Quit
    secondary/ascii_display.rs  staggered ASCII renderer
  composition_root.rs  the ONLY file that imports from adapters/
  main.rs              wires composition + runs
tests/
  known_patterns.rs   end-to-end: isolated dies, Y-center survives, tick is deterministic
```

### Boundary rules verified

- `domain/` imports only `domain/`
- `ports/` imports `domain/` only
- `usecases/` imports `domain/` + `ports/` only
- `adapters/primary/` and `adapters/secondary/` import `ports/` only
- Adapters never import other adapters
- `composition_root.rs` is the ONLY file that imports adapters

Run `hex analyze .` from the workspace root — this example is part of the 719-file scan that grades A+ (100/100).

## Tests

```
cargo test -p hex-life
```

15 unit tests across coord/grid/rules/game_loop/cli + 6 integration tests over known patterns.

## Extending

Want a web frontend? Add `src/adapters/primary/web.rs` implementing `IInputPort` over WebSocket and `src/adapters/secondary/canvas.rs` implementing `IDisplayPort` over a JSON event stream. The domain + game_loop don't change. That's the point.
