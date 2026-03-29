

# ADR-230715: F1 Web App Hexagonal Layer Structure

## Status
proposed

## Date
2023-07-15

## Drivers
- User requirement for live F1 data integration
- Need to maintain hexagonal architecture compliance
- React/Vite frontend constraints
- Historical data persistence requirements

## Context
The application requires:
1. Live driver standings (real-time updates)
2. Round-by-round race results
3. Constructor championship tables
4. Driver profile statistics
5. Historical season data

Existing architecture enforces hexagonal boundaries with:
- Domain layer containing business logic
- Ports defining interfaces
- Adapters handling external integrations
- Use cases managing application logic
- Composition root for dependency injection

## Decision
We will implement the F1 application using a strict hexagonal architecture with the following layer structure:

1. **Domain Layer** (tier 0)
   - Core business entities: `Driver`, `Race`, `Season`, `Constructor`
   - Value objects: `Position`, `Points`, `LapTime`
   - Domain services: `StandingsCalculator`, `ResultValidator`

2. **Ports Layer** (tier 1)
   - Primary ports: `LiveStandingsProvider`, `RaceResultsProvider`
   - Secondary ports: `HistoricalDataProvider`, `DriverProfileRepository`

3. **Adapters Layer** (tier 2)
   - Primary adapters: `WebSocketLiveStandingsAdapter`, `RESTRaceResultsAdapter`
   - Secondary adapters: `SQLiteHistoricalDataAdapter`, `DriverProfileAPIAdapter`

4. **Use Cases Layer** (tier 3)
   - `GetLiveStandingsUseCase`
   - `GetRaceResultsUseCase`
   - `GetDriverProfileUseCase`
   - `GetHistoricalSeasonDataUseCase`

5. **Composition Root** (tier 4)
   - React frontend integration
   - Vite build configuration
   - Dependency injection setup

## Consequences

### Positive
- Clear separation of concerns for live vs historical data
- Easy adapter swapping for different data sources
- Testable domain logic without external dependencies
- Consistent API contract between layers

### Negative
- Increased boilerplate for adapter implementations
- Complex dependency graph for large datasets
- Potential performance overhead from adapter abstractions
- Learning curve for new team members

### Neutral
- No immediate impact on existing ADR-001 compliance
- Historical data storage decisions deferred to future ADR

## Implementation

### Phases
1. **Phase 1 (Domain & Ports)** - Build core domain models and ports layer (2 weeks)
2. **Phase 2 (Adapters)** - Implement primary adapters for live data (1 week)
3. **Phase 3 (Use Cases)** - Develop use cases for race results and standings (1 week)
4. **Phase 4 (Composition Root)** - Integrate with React/Vite (1 week)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new project implementation)