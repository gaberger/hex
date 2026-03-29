

# ADR-230720001: Create DriverStatsUseCase in usecases/

## Status
proposed

## Date
2023-07-20

## Drivers
- User requirement to generate driver statistics
- Hexagonal architecture enforcement requiring use case implementation
- Need to maintain separation of concerns between domain logic and infrastructure

## Context
The project requires a new use case to generate statistics for drivers. This must be implemented within the hexagonal architecture pattern enforced by the `hex` framework. The use case will need to interact with domain entities and ports while maintaining strict boundaries between layers. Existing ADRs establish that use cases reside in the `usecases/` directory and must not import from other hex layers (ports, adapters, domain). The DriverStatsUseCase will need to be implemented in isolation from infrastructure concerns while providing a clear interface for potential adapters.

## Decision
We will create a new `DriverStatsUseCase` implementation in the `usecases/` directory. This use case will:
1. Implement the `DriverStatsUseCase` interface defined in the domain layer
2. Use only domain layer entities and ports (no infrastructure imports)
3. Be implemented as a pure function with no side effects
4. Return a `DriverStats` domain object containing aggregated statistics

## Consequences

### Positive
- Maintains strict hexagonal architecture boundaries
- Enables easy testing without infrastructure dependencies
- Allows potential reuse across different adapters
- Keeps domain logic isolated from implementation details

### Negative
- Requires additional infrastructure to implement the use case
- May introduce duplication if similar statistics are needed elsewhere
- Adds one more file to maintain in the codebase

### Neutral
- The implementation will be straightforward given the existing architecture

## Implementation

### Phases
1. Phase 1: Create the DriverStatsUseCase implementation in usecases/
2. Phase 2: Implement the DriverStats domain object in domain/
3. Phase 3: Create an adapter to connect the use case to the statistics service

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new feature implementation)