

# ADR-2303151200: FizzBuzz CLI Implementation

## Status
proposed

## Date
2023-03-15

## Drivers
- User requirement for a CLI implementation of FizzBuzz
- Hexagonal architecture enforcement for maintainability
- Need to demonstrate CLI adapter pattern in existing hex framework

## Context
The project requires a CLI implementation of FizzBuzz that adheres to hexagonal architecture principles. This must demonstrate the ports/adapters pattern while maintaining separation of concerns. Existing ADRs establish domain-driven design with strict layer boundaries (domain imports only domain, ports import only domain, adapters never import other adapters). The FizzBuzz logic must remain isolated from CLI concerns while allowing future adapter development (e.g., web, mobile).

## Decision
We will implement a FizzBuzz CLI using the hexagonal architecture pattern with the following structure:
1. **Domain Layer**: Contains the core FizzBuzz business logic (FizzBuzzRule, FizzBuzzGenerator)
2. **Ports Layer**: Defines CLI-specific interfaces (FizzBuzzCLIInputPort, FizzBuzzCLIOutputPort)
3. **Adapters Layer**: Implements CLI-specific functionality (FizzBuzzCLIAdapter)
4. **Composition Root**: Wires domain, ports, and adapters together

The CLI will be implemented as a primary adapter in `adapters/primary/cli/`. The domain will remain completely unaware of CLI implementation details. The FizzBuzz logic will be tested in isolation using dependency injection.

## Consequences

### Positive
- Clear separation of business logic from CLI concerns
- Easy to add new adapters (e.g., web, mobile) in the future
- Domain logic can be reused across different interfaces
- Testability maintained through dependency injection

### Negative
- Additional abstraction layer may increase initial complexity
- Requires more setup for CLI-specific functionality
- Domain layer remains pure with no CLI dependencies

### Neutral
- No immediate performance impact
- Maintains existing architecture consistency

## Implementation

### Phases
1. **Phase 1**: Implement domain layer with FizzBuzz rules and generator
2. **Phase 2**: Implement ports layer with CLI input/output interfaces
3. **Phase 3**: Implement CLI adapter with primary adapter pattern
4. **Phase 4**: Implement composition root to wire components

### Affected Layers
- [ ] domain/ (FizzBuzzRule, FizzBuzzGenerator)
- [ ] ports/ (FizzBuzzCLIInputPort, FizzBuzzCLIOutputPort)
- [ ] adapters/primary/ (FizzBuzzCLIAdapter)
- [ ] usecases/ (none)
- [ ] composition-root (main.go)

### Migration Notes
None