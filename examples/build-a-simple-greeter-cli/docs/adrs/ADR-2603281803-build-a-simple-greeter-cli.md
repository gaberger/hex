

# ADR-230315: Simple Greeter CLI Implementation

## Status
proposed

## Date
2023-03-15

## Drivers
- User need for a CLI-based greeting feature
- Hexagonal architecture enforcement requirements
- Existing CLI command parity requirement (ADR-019)

## Context
The project requires implementing a simple CLI greeter that displays a personalized greeting message. This must adhere to hexagonal architecture principles where the domain logic is isolated from implementation details. The CLI will serve as a primary adapter for user interaction, requiring clear separation between business rules and technical implementation. Existing CLI infrastructure (ADR-019) provides a command framework but lacks greeting functionality. The solution must maintain testability through dependency injection and avoid violating hex boundary rules.

## Decision
We will implement a GreetUser use case in the domain layer, create a CLI adapter in adapters/primary/cli, and establish a dependency relationship where the CLI adapter uses the GreetUser port. The implementation will follow these specific hex layer interactions:
1. Domain layer: Create GreetUserUseCase with a single execute method
2. Ports layer: Define GreetUserPort interface with execute method
3. Adapters/primary: Implement CLIAdapter that implements GreetUserPort
4. Composition root: Wire CLIAdapter to GreetUserPort in main()

## Consequences

### Positive
- Clear separation of domain logic from CLI implementation
- Maintain testability through dependency injection
- Enforces hex boundary rules (CLI adapter only depends on ports)
- Complies with existing CLI command parity requirement

### Negative
- Additional code complexity for a simple feature
- Requires maintaining two separate layers (ports and adapters)
- Initial setup requires more infrastructure than a monolithic approach

### Neutral
- No immediate performance impact
- No changes to existing data storage or persistence layers

## Implementation

### Phases
1. Phase 1: Implement domain layer GreetUserUseCase and ports layer GreetUserPort
2. Phase 2: Implement CLIAdapter in adapters/primary/cli that uses GreetUserPort
3. Phase 3: Update composition root to wire CLIAdapter to GreetUserPort

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new feature implementation)