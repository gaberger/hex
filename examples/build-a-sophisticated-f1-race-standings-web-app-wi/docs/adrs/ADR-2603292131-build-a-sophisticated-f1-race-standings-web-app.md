

# ADR-230715T1430: F1 Web App Hexagonal Implementation

## Status
proposed

## Date
2023-07-15

## Drivers
- User requirement for comprehensive F1 data visualization
- Existing hexagonal architecture constraints
- Need for testable domain logic
- React frontend integration requirements

## Context
The project requires implementing a multi-faceted F1 web application with live data, historical records, and detailed driver/constructor statistics. The existing hexagonal architecture (ADR-001) mandates strict layer boundaries: domain layer contains pure business logic, ports define interfaces, and adapters handle external integrations. The React frontend (ADR-003) will consume domain services through primary adapters. The system must support real-time updates, historical data queries, and complex statistical calculations while maintaining testability and maintainability.

## Decision
We will implement the F1 web app using hexagonal architecture with the following structure:
1. **Domain Layer**: Create F1-specific entities (Driver, Constructor, Race, Season) with business rules for standings calculation
2. **Ports Layer**: Define interfaces for:
   - `RaceResultPort` (primary adapter for race data)
   - `DriverProfilePort` (primary adapter for driver stats)
   - `HistoricalDataPort` (secondary adapter for season archives)
3. **Primary Adapters**: Implement React-friendly web controllers:
   - `WebRaceResultAdapter` (HTTP API handler)
   - `WebDriverProfileAdapter` (React component wrapper)
4. **Secondary Adapters**: Create data source implementations:
   - `ApiRaceResultAdapter` (F1 API integration)
   - `DatabaseHistoricalAdapter` (SQLite persistence)
5. **Use Cases**: Implement application logic in `F1UseCase` class handling:
   - Live standings calculation
   - Round-specific results
   - Historical season comparisons

## Consequences

### Positive
- Clear separation of concerns between frontend and domain logic
- Easy testing of domain logic without UI dependencies
- Simplified API integration through adapter pattern
- Maintainable code structure for complex F1 calculations

### Negative
- Increased initial setup complexity for new developers
- Potential performance overhead from adapter layer
- Requires careful API rate limiting implementation
- Additional infrastructure for data synchronization

### Neutral
- Learning curve for team members new to hexagonal architecture
- Potential for over-engineering in simple cases

## Implementation

### Phases
1. **Phase 1 (Domain & Ports)**: Implement core F1 entities and ports layer (Tiers 0-2)
2. **Phase 2 (Primary Adapters)**: Build React-friendly web controllers (Tiers 3-4)
3. **Phase 3 (Secondary Adapters)**: Implement data source integrations (Tiers 5-6)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new implementation)