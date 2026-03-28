

# ADR-230720001000: Rust CLI Hello World Implementation

## Status
proposed

## Date
2023-07-20

## Drivers
- User requirement for a minimal CLI that prints "hello"
- Hexagonal architecture enforcement requiring proper layer separation
- Need to demonstrate CLI integration within existing hex framework

## Context
The project requires a simple CLI application in Rust that prints "hello" to standard output. This must be implemented within the existing hexagonal architecture enforced by the hex framework. The CLI represents a new adapter layer that must interface with the domain layer through ports while maintaining strict separation of concerns. Existing ADRs establish the framework's hexagonal structure (ADR-001) and multi-language support (ADR-003), requiring this CLI to follow the same pattern. The implementation must avoid violating layer boundaries (ADR-014) and maintain testability through dependency injection.

## Decision
We will implement a CLI adapter in the `adapters/primary/cli` directory that:
1. Implements the `ports/cli` interface defined in `ports/cli.rs`
2. Uses dependency injection to receive the domain's `HelloService` through the `HelloServicePort`
3. Creates a composition root that wires the CLI adapter to the domain layer
4. Follows the `adapters/primary` naming convention for CLI implementations

## Consequences

### Positive
- Maintains strict hexagonal architecture boundaries
- Enables future CLI enhancements through adapter pattern
- Keeps domain layer pure and testable
- Demonstrates proper CLI integration within existing framework

### Negative
- Adds one extra layer for a simple operation
- Requires additional composition root setup
- May introduce minor performance overhead

### Neutral
- No immediate impact on existing functionality
- No changes to domain logic

## Implementation

### Phases
1. **Phase 1**: Implement domain layer with `HelloService` trait and implementation
2. **Phase 2**: Create CLI port interface in `ports/cli.rs`
3. **Phase 3**: Develop CLI adapter in `adapters/primary/cli/cli.rs`
4. **Phase 4**: Update composition root to include CLI adapter

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new component implementation)