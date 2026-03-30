

#ADR-230920T1430: FizzBuzz CLI Implementation

## Status
proposed

## Date
2023-09-20

## Drivers
- Requirement to demonstrate CLI implementation within hexagonal architecture
- Need to validate CLI adapter layer implementation
- Demonstrate domain logic isolation

## Context
The project requires implementing a FizzBuzz CLI tool in Go while maintaining strict hexagonal architecture boundaries. This must demonstrate:
1. Domain layer isolation (FizzBuzz logic only)
2. Port definition for CLI interaction
3. Adapter implementation without violating layer dependencies
4. Composition root integration

Existing architecture enforces:
- Domain layer imports only domain
- Ports import only domain
- Adapters never import other adapters
- Use cases layer not required for simple computations

## Decision
We will implement a CLI adapter that:
1. Creates a `FizzBuzzPort` interface in `ports/` defining `GenerateFizzBuzz(int)`
2. Implements `FizzBuzzCLIAdapter` in `adapters/primary/cli/` that:
   - Reads integer from command line
   - Calls domain service
   - Prints results
3. Uses composition root to wire adapter to domain
4. Maintains strict dependency inversion (CLI adapter depends on port, not domain)

## Consequences

### Positive
- Demonstrates complete hexagonal layer implementation
- Provides testable CLI implementation
- Maintains architecture boundaries
- Enables future adapter development (web, API)

### Negative
- Over-engineering for simple problem
- Additional testing complexity
- Composition root complexity

### Neutral
- No immediate performance impact
- No new dependencies introduced

## Implementation

### Phases
1. Phase 1: Domain layer implementation (FizzBuzz logic)
2. Phase 2: Port definition and CLI adapter implementation
3. Phase 3: Composition root integration

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/cli/
- [ ] usecases/ (not required)
- [ ] composition-root

### Migration Notes
None (new implementation)