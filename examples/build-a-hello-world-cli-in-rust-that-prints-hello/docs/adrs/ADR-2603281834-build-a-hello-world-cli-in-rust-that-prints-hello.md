

# ADR-230720T1430: Rust CLI Hello World Implementation

## Status
proposed

## Date
2023-07-20

## Drivers
- User requirement for a minimal CLI application
- Hexagonal architecture enforcement requirements
- Need to demonstrate CLI adapter implementation

## Context
The project requires a minimal CLI application that prints "hello" to demonstrate CLI adapter implementation within the hexagonal architecture. This must adhere to existing architecture constraints:
- Hexagonal architecture enforced by hex framework
- Domain layer must remain pure and unexposed
- Ports must define interfaces without implementation
- Adapters must not import other adapters
- Use cases must be testable in isolation

## Decision
We will implement a CLI application that prints "hello" using the hexagonal architecture pattern. This will be achieved by:
1. Creating a `hello` use case in the domain layer that returns "hello"
2. Implementing a primary CLI adapter that invokes the use case
3. Using the `hex` framework's dependency injection to connect the adapter to the use case
4. Ensuring the CLI adapter only depends on the use case port

## Consequences

### Positive
- Demonstrates proper CLI adapter implementation
- Maintains clean separation between domain logic and infrastructure
- Provides testable entry point for CLI operations
- Follows existing architecture patterns

### Negative
- Initial setup complexity for minimal functionality
- Requires additional infrastructure for dependency injection
- May introduce unnecessary abstraction for trivial functionality

### Neutral
- No immediate performance impact
- No changes to existing data models or persistence layers

## Implementation

### Phases
1. **Phase 1**: Implement domain use case and port
   - Create `hello` use case in `domain/usecases`
   - Define `HelloUseCase` port in `ports/usecases`
   - Implement unit tests for use case

2. **Phase 2**: Implement primary CLI adapter
   - Create `cli` adapter in `adapters/cli`
   - Implement `run` function using dependency injection
   - Add integration test for CLI output

### Affected Layers
- [ ] domain/usecases/hello.rs
- [ ] ports/usecases/hello.rs
- [ ] adapters/cli/cli.rs
- [ ] composition-root/cli.rs

### Migration Notes
None (new feature implementation)