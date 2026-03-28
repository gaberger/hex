

# ADR-240530P1A: Simple Rust Hello-World CLI Adapter

## Status
proposed

## Date
2024-05-30

## Drivers
- User requirement for a minimal CLI demonstration
- Maintain hexagonal architecture compliance
- Preserve existing composition-root structure

## Context
The project requires a simple CLI that prints "Hello, World!" to demonstrate basic functionality. This must integrate with the existing hexagonal architecture while maintaining separation of concerns. The CLI must be implemented as an adapter layer that depends only on domain and usecase layers, not other adapters. The composition-root must remain the single entry point for all components.

## Decision
We will implement a CLI adapter that:
1. Creates a `HelloCommand` struct implementing `Command` from `clap`
2. Uses the `GreetingUseCase` from the domain layer to generate the message
3. Registers the command in the composition-root's `App` builder
4. Maintains strict dependency boundaries (CLI → usecases → domain)

## Consequences

### Positive
- Demonstrates CLI integration without violating architecture boundaries
- Maintains testability through dependency injection
- Keeps composition-root as the central wiring point

### Negative
- Adds minimal boilerplate for CLI setup
- Requires maintaining two separate command implementations (CLI vs MCP)

### Neutral
- Existing CLI parity requirements (ADR-019) remain satisfied through this implementation

## Implementation

### Phases
1. Phase 1: Implement `GreetingUseCase` and CLI adapter (tiers 3-4)
2. Phase 2: Integrate with composition-root (tier 5)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new component implementation)