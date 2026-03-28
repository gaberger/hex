

# ADR-230712123456: Simple Greeter CLI Implementation

## Status
proposed

## Date
2023-07-12

## Drivers
- User requirement for a CLI-based greeting service
- Hexagonal architecture enforcement requirements
- Need to demonstrate CLI-MCP parity (ADR-019)

## Context
The project requires implementing a simple CLI greeter that outputs personalized greetings. This must adhere to hexagonal architecture principles where business logic resides in the domain layer, ports define interfaces, and adapters handle external interactions. The CLI will be the primary adapter for user interaction, requiring a console output port. Existing infrastructure includes established usecase patterns (ADR-006) and CLI-MCP parity requirements (ADR-019). The solution must avoid violating layer boundaries (ADR-001) while maintaining testability through dependency injection (ADR-014).

## Decision
We will implement a CLI greeter using hexagonal architecture with the following structure:
1. **Domain Layer**: Create a `Greeting` domain entity with a `greet` method
2. **Ports Layer**: Define a `ConsoleOutputPort` interface with a `write` method
3. **Adapters Layer**: Implement a `ConsoleAdapter` that implements `ConsoleOutputPort`
4. **Usecases Layer**: Create a `GreetUserUseCase` that uses the `Greeting` domain and `ConsoleOutputPort`
5. **Composition Root**: Initialize the `GreetUserUseCase` with the `ConsoleAdapter` and execute it

## Consequences

### Positive
- Maintains strict hexagonal architecture boundaries
- Enables easy testing of the greeting logic in isolation
- Allows future expansion to other output adapters (e.g., web, mobile)
- Demonstrates CLI-MCP parity as required by ADR-019

### Negative
- Adds infrastructure complexity for a simple greeting
- Requires additional code for adapter composition
- Initial implementation has limited functionality

### Neutral
- The solution follows established ADR patterns
- No immediate performance impact

## Implementation

### Phases
1. **Phase 1**: Implement domain entity and port interfaces (Tiers 3-2)
2. **Phase 2**: Create console adapter implementation (Tier 1)
3. **Phase 3**: Build usecase and composition root (Tiers 3-1)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new feature implementation)