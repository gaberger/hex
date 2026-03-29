

# ADR-230905: F1 Web App Hexagonal Implementation

## Status
proposed

## Date
2023-10-05

## Drivers
- User requirement for a sophisticated F1 web application with multiple data views
- Existing hexagonal architecture enforcement via hex framework
- Need to maintain clean separation of concerns across layers

## Context
The project requires implementing a multi-view F1 web application (live standings, race results, constructor tables, driver profiles, historical data) using TypeScript/React. The existing architecture (ADR-001) mandates hexagonal boundaries where domain logic must remain isolated from external concerns. The application must handle real-time data updates while maintaining testability and maintainability. The hex framework enforces strict layer boundaries: domain imports only domain, ports import only domain, adapters never import other adapters, and usecases coordinate between domain and ports.

## Decision
We will implement the F1 application using the established hexagonal architecture layers:
1. **Domain Layer**: Create core F1 entities (Driver, Race, Constructor) with business rules
2. **Ports Layer**: Define interfaces for data access (DriverRepository, RaceRepository)
3. **Adapters Layer (Primary)**: Implement React components as web adapters
4. **Adapters Layer (Secondary)**: Create API clients for F1 data sources
5. **Usecases Layer**: Implement business logic coordinating domain and ports
6. **Composition Root**: Initialize dependencies and wire components

## Consequences

### Positive
- Clear separation of concerns enables independent development
- Testability improved through dependency injection
- Easy swapping of data sources (e.g., mock vs real API)
- Maintainability through bounded contexts

### Negative
- Increased initial setup complexity
- Learning curve for new developers
- Potential for over-engineering in simple cases

### Neutral
- No immediate performance impact
- No changes to existing build processes

## Implementation

### Phases
1. **Phase 1**: Domain and ports layer implementation (Tier 0-1)
   - Create F1 domain entities and repositories
   - Define ports interfaces
   - Implement basic usecases

2. **Phase 2**: Adapters and composition root (Tier 2-3)
   - Build React components
   - Implement API clients
   - Configure dependency injection

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new implementation)