# My Hex Project

Scaffolded with [hex-intf](https://github.com/your-org/hex-intf).

## Quick Start

```bash
bun install
bun run dev
```

## Commands

| Command | Description |
|---------|-------------|
| `bun run dev` | Start dev server with watch |
| `bun test` | Run tests |
| `bun run build` | Build for production |
| `bun run check` | Type-check without emitting |

## Architecture

```
src/
  core/
    domain/        Domain entities and value objects
    ports/         Port interfaces (input + output)
    usecases/      Use case implementations
  adapters/
    primary/       Driving adapters (CLI, HTTP, etc.)
    secondary/     Driven adapters (DB, FS, API, etc.)
  infrastructure/  Cross-cutting concerns
  composition-root.ts
```
