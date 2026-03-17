# hello-hex

Minimal example demonstrating `@anthropic-hex/hex` with hexagonal architecture.

## Structure

```
src/
  core/
    domain/greeting.ts       # Pure value object (Greeting)
    ports/greeting-port.ts   # Input + Output port interfaces
    usecases/greet-usecase.ts # Business logic (composes ports)
  adapters/
    primary/cli-adapter.ts   # Reads CLI args
    secondary/console-output-adapter.ts  # Writes to stdout
  composition-root.ts        # Wires adapters to ports
  main.ts                    # Entry point
```

## Usage

```bash
bun install
bun run start           # prints "Hello, World! Welcome to hex."
bun run start Alice     # prints "Hello, Alice! Welcome to hex."
bun test                # runs domain + use case tests
```

## Architecture validation

```bash
npx @anthropic-hex/hex analyze .
```
