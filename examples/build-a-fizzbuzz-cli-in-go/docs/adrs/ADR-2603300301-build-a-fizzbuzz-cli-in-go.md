# ADR-230720T1430: FizzBuzz CLI Implementation via Hexagonal Adapters

## Status
proposed

## Date
2023-07-20

## Drivers
- User requirement for a CLI implementation of FizzBuzz
- Existing hexagonal architecture enforcement (ADR-001)
- Need to demonstrate CLI-adapter pattern in a simple use case
- ADR-019 requirement for CLI-MCP parity

## Context
The project requires a CLI implementation of FizzBuzz that adheres to hexagonal architecture principles. This must:
1. Isolate core business logic from implementation details
2. Demonstrate proper port/adapter separation
3. Maintain compatibility with existing CLI infrastructure
4. Follow ADR-001's port/interface-first approach

Key constraints:
- Must not violate hex boundary rules (domain imports only domain, ports import only domain, adapters never import other adapters)
- Must demonstrate proper dependency inversion
- Should be implementable in phases to avoid breaking existing architecture

## Decision
We will implement the FizzBuzz CLI as a primary adapter using the hexagonal architecture pattern. This involves:
1. Creating a `ports/cli` directory with a `FizzBuzzCLI` interface
2. Implementing the `FizzBuzzCLI` interface in `adapters/cli/fizzbuzz.go`
3. Using dependency injection to connect the CLI adapter to the domain logic
4. Adding the CLI to the composition root for activation

## Consequences

### Positive
- Demonstrates proper port/adapter separation for CLI implementation
- Maintains clean separation between business logic and CLI concerns
- Provides a testable implementation of the FizzBuzz algorithm
- Complies with ADR-001's architecture enforcement

### Negative
- Requires additional files and interfaces for minimal functionality
- Adds complexity for a simple problem (compared to direct implementation)
- May require learning curve for new developers unfamiliar with hexagonal patterns

### Neutral
- The FizzBuzz logic itself remains unchanged and testable
- No impact on existing CLI infrastructure or MCP parity requirements

## Implementation

### Phases
1. **Phase 1 (Domain & Ports)**: Create `domain/fizzbuzz.go` with core logic and `ports/cli/fizzbuzz_cli.go` interface
2. **Phase 2 (Adapters)**: Implement `adapters/cli/fizzbuzz.go` with CLI-specific logic
3. **Phase 3 (Composition)**: Add CLI to `composition/root.go` for activation

### Affected Layers
- [ ] domain/fizzbuzz.go
- [ ] ports/cli/fizzbuzz_cli.go
- [ ] adapters/cli/fizzbuzz.go
- [ ] composition/root.go

### Migration Notes
None (new feature implementation)