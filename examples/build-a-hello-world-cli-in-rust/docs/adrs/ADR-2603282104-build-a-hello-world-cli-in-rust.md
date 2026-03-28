

# ADR-230315: CLI Adapterfor Hello World

## Status
proposed

## Date
2023-03-15

## Drivers
- User requirement for a CLI interface to the system
- Hexagonal architecture enforcement requiring primary adapters
- Need to demonstrate CLI implementation pattern

## Context
The project requires a CLI interface for user interaction. Hexagonal architecture mandates that primary adapters (like CLI) must not depend on secondary adapters or other layers. This CLI must implement the ports defined in the domain layer without violating architectural boundaries. The system currently has no CLI implementation, and this will be the first primary adapter built.

## Decision
We will implement a CLI adapter in `adapters/primary/cli` that implements the `OutputPort` and `InputPort` interfaces defined in the domain layer. The CLI will handle user input/output for the "hello world" command while maintaining strict separation from domain logic. Implementation will follow these rules:
1. CLI will only call domain use cases via ports
2. Domain ports will only depend on domain interfaces
3. CLI will not import any secondary adapters or other primary adapters
4. CLI will be implemented in a new module within `adapters/primary/cli`

## Consequences

### Positive
- Maintains architectural integrity by enforcing port-adapter separation
- Enables easy testing of CLI logic in isolation
- Provides clear separation between user interface and business logic
- Demonstrates proper CLI implementation pattern for future adapters

### Negative
- Initial setup requires defining new ports/interfaces
- CLI implementation will be more verbose than direct function calls
- Requires additional infrastructure for testing CLI interactions

### Neutral
- No immediate performance impact
- No changes to existing domain logic
- No database or external service dependencies introduced

## Implementation

### Phases
1. Phase 1: Define CLI ports and interfaces in domain layer
2. Phase 2: Implement CLI adapter using the defined ports
3. Phase 3: Add CLI command registration and testing

### Affected Layers
- [ ] domain/ports
- [ ] domain/usecases
- [ ] adapters/primary/cli
- [ ] composition-root

### Migration Notes
None (new component implementation)