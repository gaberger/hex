

# ADR-230720142300: Driver-Centric Domain Layer for F1 Standings

## Status
proposed

## Date
2023-07-20

## Drivers
- Need to manage complex driver-related data (positions, stats, profiles)
- Requirement for live data updates
- Separation of concerns between data access and business logic
- Compliance with hexagonal architecture boundaries

## Context
The F1 standings app requires managing driver-specific data across multiple views (live standings, profiles, historical data). Current architecture lacks clear separation between driver business logic and data access. Hexagonal architecture enforces strict layer boundaries, requiring all external dependencies (APIs, databases) to be accessed through ports. The existing domain layer is underdeveloped, creating technical debt and complicating future extensions.

## Decision
We will implement a driver-centric domain layer with the following structure:
1. **Domain Layer**: Create `driver/` directory with:
   - `Driver` entity (ID, name, nationality, team, position, points)
   - `DriverStandings` aggregate (position, points, wins, podiums)
   - `DriverProfile` aggregate (stats, career highlights)
   - `DriverUseCase` interface for all driver-related operations
2. **Ports Layer**: Define interfaces for:
   - `DriverRepository` (getDriver, getStandings, getProfile)
   - `LiveDriverStream` (for real-time updates)
3. **Adapters Layer**: Implement API adapters for:
   - `F1ApiAdapter` (fetching live data)
   - `LocalStorageAdapter` (for historical data caching)
4. **Composition Root**: Initialize use cases with appropriate adapters

## Consequences

### Positive
- Clear separation between driver business logic and data access
- Easier testing through mock repositories
- Simplified future extensions (e.g., adding new stats)
- Compliance with hexagonal architecture boundaries

### Negative
- Initial implementation complexity for new developers
- Requires careful adapter implementation to avoid leaks
- Potential performance overhead from multiple adapters

### Neutral
- No immediate impact on UI layer
- Maintains existing ADR-001 hexagonal architecture compliance

## Implementation

### Phases
1. **Phase 1 (Domain & Ports)**: Implement core driver entities and interfaces (2 weeks)
2. **Phase 2 (Adapters)**: Build API and local storage adapters (3 weeks)
3. **Phase 3 (Use Cases)**: Implement driver use cases using ports (1 week)
4. **Phase 4 (Composition Root)**: Integrate with existing composition root (1 week)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None - This implementation follows existing ADR-001 boundaries and doesn't require data migration.