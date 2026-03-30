

# ADR-230315123456:F1 Race Standings Web App with Hexagonal Architecture

## Status
proposed

## Date
2023-03-15

## Drivers
- User requirement for a sophisticated F1 standings web app with multiple data views
- Existing hexagonal architecture enforcement via hex framework
- Need for clean separation between UI, business logic, and data layers
- TypeScript + React + Vite stack constraints

## Context
The application requires multiple data views (live standings, race results, constructor tables, driver profiles, historical data) with real-time updates. The existing hexagonal architecture (ADR-001) mandates strict layer boundaries: domain layer contains business logic, ports define interfaces, adapters implement concrete integrations, and use cases orchestrate operations. The React frontend must integrate with this architecture while leveraging Vite for development. Key challenges include maintaining clean boundaries between UI and domain logic, ensuring testability, and managing data flow between layers.

## Decision
We will implement the F1 standings web app using a hexagonal architecture pattern enforced by the hex framework. The React frontend will reside in the `adapters/primary` layer, communicating with the domain layer via ports defined in `ports`. Use cases in `usecases` will orchestrate operations between domain entities and adapters. The composition root will wire dependencies between layers. All data sources (API, mock data) will be implemented as secondary adapters in `adapters/secondary`.

## Consequences

### Positive
- Clear separation of concerns between UI and business logic
- Enhanced testability through dependency injection
- Easy swapping of data sources (e.g., mock vs. real API)
- Scalability through modular component design

### Negative
- Increased initial setup complexity
- Learning curve for developers new to hexagonal architecture
- Potential for over-engineering in simple cases

### Neutral
- Requires strict adherence to layer boundaries
- May necessitate additional infrastructure for data synchronization

## Implementation

### Phases
1. **Phase 1**: Implement domain layer with core entities (Driver, Race, Constructor) and ports (DriverRepository, RaceRepository, etc.)
2. **Phase 2**: Build use cases (GetLiveStandings, GetRaceResultsByRound, GetDriverProfile, etc.) and primary adapter (React frontend)
3. **Phase 3**: Implement secondary adapters (API client, mock data provider) and composition root

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new project)