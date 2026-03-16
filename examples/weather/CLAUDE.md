# Hexagonal Architecture Project

## Behavioral Rules

- ALWAYS read a file before editing it
- NEVER commit secrets, credentials, or .env files
- ALWAYS run `bun test` after making code changes
- ALWAYS run `bun run build` before committing

## Hexagonal Architecture Rules (ENFORCED)

1. **domain/** must only import from **domain/**
2. **ports/** may import from **domain/** but nothing else
3. **usecases/** may import from **domain/** and **ports/** only
4. **adapters/primary/** may import from **ports/** only
5. **adapters/secondary/** may import from **ports/** only
6. **adapters must NEVER import other adapters** (cross-adapter coupling)
7. **composition-root** is the ONLY file that imports from adapters
8. All relative imports MUST use `.js` extensions (NodeNext module resolution)

## File Organization

```
src/
  core/
    domain/          # Pure business logic, zero external deps
    ports/           # Typed interfaces (contracts between layers)
    usecases/        # Application logic composing ports
  adapters/
    primary/         # Driving adapters (CLI, HTTP, browser input)
    secondary/       # Driven adapters (DB, API, filesystem)
  composition-root   # Wires adapters to ports (single DI point)
```

## Security

- Never commit `.env` files â use `.env.example`
- Primary adapters MUST NOT use `innerHTML`/`outerHTML`/`insertAdjacentHTML` with any data that originates outside the domain layer. Use `textContent` or DOM APIs (`createElement`) instead.

## On Startup

When a new conversation begins, a SessionStart hook runs `scripts/hex-startup.sh` which outputs the project status. You MUST:

1. Read the hook output (it appears in a system-reminder) to understand project progress
2. Read `PRD.md` for the full project scope
3. **Immediately present** the user with:
   - Project name and goal (from PRD)
   - Pipeline progress (which hex layers are done vs todo)
   - The recommended next step
   - Ask what they'd like to work on
4. Do NOT wait for the user to ask — proactively guide them

## Development Pipeline (follow this order)

1. **Domain** — Define entities and value objects in `domain/`
2. **Ports** — Define typed interfaces (contracts) in `ports/`
3. **Use Cases** — Implement business logic in `usecases/`, importing only domain + ports
4. **Adapters** — Implement primary (input) and secondary (output) adapters
5. **Composition Root** — Wire adapters to ports in `composition-root`
6. **Tests** — Unit tests (London-school mocks) + integration tests
7. **Validate** — Run `hex-intf analyze .` to check architecture health
