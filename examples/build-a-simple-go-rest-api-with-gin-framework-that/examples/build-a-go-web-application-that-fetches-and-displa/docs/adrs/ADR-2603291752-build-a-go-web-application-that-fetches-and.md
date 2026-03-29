

# ADR-230315123456: F1 Standings Web App with Hexagonal Architecture

## Status
proposed

## Date
2023-03-15

## Drivers
- User requirement for a web application displaying F1 standings
- Existing hexagonal architecture enforcement via hex framework
- Need for clean separation of concerns in Go implementation

## Context
The application must fetch real-time F1 driver standings from the Ergast API and render them in an HTML table at localhost:8080. The project already enforces hexagonal architecture (ports & adapters) through the hex framework. Key constraints include:
- Must maintain strict layer boundaries (domain → ports → adapters)
- Existing infrastructure includes SQLite persistence (ADR-015) and Git worktrees (ADR-004)
- Must integrate with existing multi-language support (ADR-003)
- Must comply with existing ADR lifecycle tracking (ADR-012)

## Decision
We will implement a hexagonal architecture solution with the following structure:
1. **Domain Layer**: Create `domain/driver_standing.go` defining the `DriverStanding` struct and business rules
2. **Use Cases**: Implement `usecases/fetch_standings.go` as the application service layer
3. **Ports**: Create `ports/api.go` defining the `FetchStandingsPort` interface
4. **Adapters**: Implement `adapters/ergast_adapter.go` as the HTTP client adapter
5. **Composition Root**: Configure dependencies in `main.go` using hex's dependency injection

The decision affects layers 0 (domain), 1 (usecases), 2 (ports), and 3 (adapters) in the hexagonal architecture tiers.

## Consequences

### Positive
- Clear separation of concerns enables easier testing
- Adheres to existing architectural standards
- Facilitates future API changes through adapter abstraction
- Maintains testability via dependency injection

### Negative
- Additional abstraction layer may increase complexity
- Requires maintaining multiple small files
- HTTP request handling adds latency

### Neutral
- No immediate performance impact
- No data migration requirements
- No backward compatibility concerns

## Implementation

### Phases
1. **Phase 1**: Implement domain model and use cases (2 days)
2. **Phase 2**: Build API port and adapter (1 day)
3. **Phase 3**: Integrate with web framework (1 day)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None