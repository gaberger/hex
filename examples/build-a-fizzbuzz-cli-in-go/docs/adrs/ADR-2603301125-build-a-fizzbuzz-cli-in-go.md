

# ADR-230817143000:FizzBuzz CLI Implementation

## Status
proposed

## Date
2023-08-17

## Drivers
- User requirement for a FizzBuzz CLI in Go
- Existing hexagonal architecture enforcement via hex framework
- Need to demonstrate CLI adapter implementation in Go

## Context
The project requires implementing a FizzBuzz CLI application in Go while maintaining strict adherence to hexagonal architecture principles. This involves creating a domain layer with core business logic, ports defining interfaces for external interactions, and an adapter layer for CLI input/output. The solution must demonstrate proper layer isolation where domain code imports only domain, ports import only domain, and adapters never import other adapters. The Go implementation must integrate with existing project infrastructure including dependency injection and test isolation mechanisms established in ADR-014.

## Decision
We will implement a FizzBuzz CLI using hexagonal architecture tiers:
1. **Domain Layer**: Create `fizzbuzz/domain` package with `FizzBuzz` struct and `Generate` method implementing core logic
2. **Ports Layer**: Define `FizzBuzzPort` interface in `fizzbuzz/ports` with `Generate` method signature
3. **Adapters Layer**: Implement `fizzbuzz/adapters/primary/cli` package with `NewCLIAdapter` function creating CLI-specific logic
4. **Composition Root**: Use `hex.New` to wire domain, ports, and CLI adapter together

## Consequences

### Positive
- Demonstrates proper hexagonal architecture implementation in Go
- Maintains strict layer isolation as per ADR-001
- Provides testable domain logic through dependency injection
- Shows CLI adapter implementation pattern for future CLI tools

### Negative
- Over-engineering for simple FizzBuzz logic
- Additional boilerplate for minimal functionality
- Potential performance overhead from interface indirection

### Neutral
- No significant impact on existing architecture
- Reinforces Go best practices for interface design
- Maintains consistency with TypeScript/Rust implementations

## Implementation

### Phases
1. **Phase 1**: Implement domain layer with FizzBuzz logic (Tiers 0-1)
2. **Phase 2**: Implement ports layer with interface definition (Tier 2)
3. **Phase 3**: Implement CLI adapter layer with input/output handling (Tier 3)
4. **Phase 4**: Compose components in composition root (Tier 4)

### Affected Layers
- [ ] domain/fizzbuzz
- [ ] ports/fizzbuzz
- [ ] adapters/primary/cli
- [ ] composition-root

### Migration Notes
None (new implementation)