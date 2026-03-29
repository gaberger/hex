

# ADR-230920T1430: SQLite Adapter for DriverStats

## Status
proposed

## Date
2023-09-20

## Drivers
- Requirement to support local development with lightweight database
- Need for isolated testing environment for DriverStats functionality
- Existing SQLite persistence in ADR-015 for Hub Swarm State suggests similar pattern

## Context
The DriverStats feature requires persistent storage for driver performance metrics. Current implementation uses an in-memory database which is insufficient for production scenarios. ADR-015 established SQLite as the persistence layer for Hub Swarm State, indicating a preference for SQLite across the system. This creates a need for a dedicated SQLite adapter in the secondary adapters layer to maintain consistency with existing persistence patterns while enabling local development and testing.

## Decision
We will implement a SQLite adapter for DriverStats in `adapters/secondary/` using the `sqlite3` driver. This adapter will implement the `DriverStatsPort` interface defined in `ports/` and will be registered in the composition root. The adapter will handle all database operations for DriverStats, including schema migrations and transaction management.

## Consequences

### Positive
- Enables local development with persistent storage
- Provides consistent persistence layer with Hub Swarm State
- Simplifies testing by allowing in-memory SQLite for unit tests
- Reduces dependency on external database services

### Negative
- Introduces SQLite-specific dependencies that may not be suitable for production
- Adds complexity to the adapter layer
- Potential performance limitations compared to production databases

### Neutral
- Maintains consistency with existing SQLite implementation in ADR-015

## Implementation

### Phases
1. Phase 1: Implement SQLite adapter for DriverStats in `adapters/secondary/` (Tier 3)
2. Phase 2: Integrate adapter into composition root and update DriverStats use cases (Tier 4)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/ (new)
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None