

# ADR-2405301200: Implement CLI Primary Adapter with FizzBuzz Port

## Status
proposed

## Date
2024-05-30

## Drivers
- Requirement to implement CLI primary adapter (ADR-019)
- Need for consistent FizzBuzz implementation across CLI and MCP
- Hexagonal architecture enforcement requiring port-adapter separation

## Context
The CLI requires a primary adapter to handle user input/output for FizzBuzz operations. Existing architecture mandates that primary adapters implement ports defined in the `ports/` layer. The FizzBuzz port (ADR-003) defines the interface for FizzBuzz operations, which must now be implemented by the CLI adapter. This decision must respect hex boundary rules: the CLI adapter (adapters/primary/cli/) should only import domain and ports layers, never other adapters. The implementation must align with ADR-019's requirement for CLI-MCP parity, ensuring the CLI adapter mirrors MCP functionality through the FizzBuzz port interface.

## Decision
We will implement the CLI primary adapter using the FizzBuzz port interface. The CLI adapter will implement the `FizzBuzzPort` interface defined in `ports/fizz_buzz.ts`, handling user input parsing and output formatting while delegating core logic to the domain layer. This maintains strict port-adapter separation, with the CLI adapter acting as the adapter layer (tier 3) that translates between user interactions and domain use cases.

## Consequences

### Positive
- Enforces hexagonal architecture boundaries by preventing CLI adapter from importing other adapters
- Provides consistent FizzBuzz implementation across CLI and MCP
- Simplifies testing by isolating CLI-specific concerns from domain logic

### Negative
- Adds complexity to CLI adapter implementation for input/output handling
- Requires additional validation for user input against FizzBuzz rules
- Potential for slight performance overhead due to port abstraction

### Neutral
- No immediate impact on existing domain or secondary adapters

## Implementation

### Phases
1. **Phase 1**: Create CLI adapter implementation of FizzBuzzPort, handling user input parsing and output formatting
2. **Phase 2**: Integrate CLI adapter with existing domain logic via FizzBuzzPort interface

### Affected Layers
- [ ] domain/ (no direct changes)
- [ ] ports/ (FizzBuzzPort interface already exists)
- [ ] adapters/primary/cli/ (new implementation)
- [ ] adapters/secondary/ (no changes)
- [ ] usecases/ (no direct changes)
- [ ] composition-root (no changes)

### Migration Notes
None