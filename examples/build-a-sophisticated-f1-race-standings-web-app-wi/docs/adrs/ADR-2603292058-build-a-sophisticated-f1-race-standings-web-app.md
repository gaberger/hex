

# ADR-230315T1200: F1 Web App with Hexagonal Layers

## Status
proposed

## Date
2023-03-15

## Drivers
- User requirement for live driver standings and historical season data
- Need for maintainable, testable architecture
- Separation of concerns across multiple data sources
- Compliance with existing hexagonal architecture framework

## Context
The application requires multiple data sources (live API, historical DB) and presentation layers (driver standings, constructor tables, profiles). Existing ADRs establish hexagonal architecture as the foundational pattern, with strict layer boundaries (domain imports only domain, ports import only domain, adapters never import other adapters). The stack uses TypeScript + React, requiring clear separation between business logic and UI implementation.

## Decision
We will implement the F1 web app using hexagonal architecture tiers 0-3 (domain, ports, adapters, usecases) with React as the presentation adapter. The domain layer will contain race-related business logic, ports will define interfaces for data access, and adapters will implement these interfaces for API and database connections. Usecases will orchestrate domain logic and port interactions. React components will consume usecases via ports, maintaining strict layer boundaries.

## Consequences

### Positive
- Clear separation between business logic and UI implementation
- Easier testing of domain logic and data access
- Simplified maintenance of race-related business rules
- Clear upgrade path for data source changes

### Negative
- Increased initial setup complexity
- Requires careful dependency management
- Learning curve for team members new to hexagonal architecture

### Neutral
- No immediate performance impact
- No changes to existing ADR compliance requirements

## Implementation

### Phases
1. **Phase 1**: Implement domain layer with race entity definitions and usecases for standings calculations
2. **Phase 2**: Create ports and adapters for API data access
3. **Phase 3**: Develop React components consuming usecases via ports

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None