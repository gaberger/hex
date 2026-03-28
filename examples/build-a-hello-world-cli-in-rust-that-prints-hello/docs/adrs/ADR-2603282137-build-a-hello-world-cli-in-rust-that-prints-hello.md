

# ADR-230720T1430: CLI Adapter for Hello World

## Status
proposed

## Date
2023-07-20

## Drivers
- User requirement for CLI-based hello world functionality
- Hexagonal architecture enforcement requiring clear adapter boundaries
- Need to demonstrate CLI integration in composition root

## Context
The project requires a minimal CLI implementation to print "hello" while maintaining hexagonal architecture boundaries. Existing ADRs establish domain layers (usecases) and port definitions, but no CLI adapter exists. The challenge is creating a CLI adapter that:
1. Follows strict hex layering rules (adapters never import other adapters)
2. Delegates to domain use cases
3. Integrates cleanly into the composition root
4. Maintains testability through dependency injection

## Decision
We will implement a CLI adapter that:
1. Creates a `CliAdapter` in `adapters/primary/cli.rs`
2. Implements the `CliPort` interface defined in `ports/cli.rs`
3. Uses dependency injection to receive the `HelloUseCase` from the composition root
4. Implements the `execute` method to read input and print "hello"

```rust
// adapters/primary/cli.rs
pub struct CliAdapter {
    hello_usecase: HelloUseCase,
}

impl CliAdapter {
    pub fn new(hello_usecase: HelloUseCase) -> Self {
        CliAdapter { hello_usecase }
    }
}

impl CliPort for CliAdapter {
    fn execute(&self) {
        println!("hello");
    }
}
```

## Consequences

### Positive
- Maintains strict hex layering boundaries
- Enables testing via dependency injection
- Demonstrates CLI integration pattern
- Provides clear separation between I/O and domain logic

### Negative
- Adds one additional layer (CLI adapter)
- Requires composition root wiring
- Minimal functional benefit beyond "hello"

### Neutral
- No impact on existing domain logic
- No performance implications

## Implementation

### Phases
1. **Phase 1 (Tiers 0-2)**: Implement CLI adapter and port interface
   - Create `CliAdapter` and `CliPort` in `adapters/primary/cli.rs`
   - Implement `execute` method to print "hello"

2. **Phase 2 (Tiers 3-5)**: Integrate into composition root
   - Wire `CliAdapter` to `HelloUseCase` in composition root
   - Add CLI command registration

### Affected Layers
- [ ] domain/ (no changes)
- [ ] ports/ (add `CliPort` interface)
- [ ] adapters/primary/ (add `cli.rs`)
- [ ] adapters/secondary/ (no changes)
- [ ] usecases/ (no changes)
- [ ] composition-root (add CLI wiring)

### Migration Notes
None - this is a new feature with no backward compatibility requirements