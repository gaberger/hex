

# ADR-230720142300: Fibonacci CLI Adapter Implementation

## Status
proposed

## Date
2023-07-20

## Drivers
- Requirement to implement CLI interface for Fibonacci sequence generation
- Maintain hexagonal architecture compliance
- Enable testability through dependency injection
- Support future adapter variants (web, API)

## Context
The project requires a CLI interface for Fibonacci sequence generation. Existing architecture enforces hexagonal boundaries where domain logic must remain isolated from external concerns. The Fibonacci calculation must reside in the domain layer, while the CLI implementation must act as a port adapter that communicates with the domain through defined interfaces. This decision must respect existing ADR-001 (Hexagonal Foundation) and ADR-014 (Test Isolation) while enabling future adapter development.

## Decision
We will implement a CLI adapter that:
1. Creates a `FibonacciCLI` struct implementing the `FibonacciPort` interface
2. Uses dependency injection to inject the `FibonacciUseCase` into the CLI adapter
3. Implements the `execute` method to parse CLI arguments and call the use case
4. Formats and prints the sequence using the domain's `FibonacciSequence` output type
5. Follows strict layer boundaries: domain imports only domain, ports import only domain, adapters never import other adapters

## Consequences

### Positive
- Maintains clean separation between domain logic and CLI implementation
- Enables unit testing of CLI behavior without external dependencies
- Allows parallel development of CLI and domain logic
- Provides clear extension points for future adapter implementations

### Negative
- Adds boilerplate for CLI argument parsing and error handling
- Increases initial implementation complexity
- Requires additional dependency injection setup

### Neutral
- No immediate performance impact
- No changes to existing domain logic

## Implementation

### Phases
1. **Phase 1**: Implement domain layer with Fibonacci calculation logic
2. **Phase 2**: Create port interface and CLI adapter implementation
3. **Phase 3**: Add integration tests for CLI adapter

### Affected Layers
- [ ] domain/ (Fibonacci calculation logic)
- [ ] ports/ (FibonacciPort interface)
- [ ] adapters/primary/ (CLI adapter)
- [ ] usecases/ (FibonacciUseCase)
- [ ] composition-root (CLI initialization)

### Migration Notes
None - This is a new feature implementation.