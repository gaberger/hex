

# ADR-240627142300: F1 Race Standings Web App Hexagonal Implementation

## Status
proposed

## Date
2024-06-27

## Drivers
- User requirement to display real-time F1 race standings
- Existing hexagonal architecture constraints (ADR-001)
- Need for testable data access layer
- Separation of concerns between domain logic and infrastructure

## Context
The application must fetch F1 race standings from an external API and display them in a web interface. This requires:
1. Domain modeling for race standings
2. Data access layer for API integration
3. Web presentation layer
4. Adherence to hexagonal architecture boundaries
5. Testability of all layers

Existing constraints:
- Hexagonal architecture enforced (ADR-001)
- Ports must only depend on domain
- Adapters must not depend on other adapters
- Usecases must orchestrate domain and ports

## Decision
We will implement the F1 standings feature using hexagonal architecture with the following structure:

1. **Domain Layer**: Create `race_standing` entity and `standings_service` usecase
2. **Ports Layer**: Define `standings_repository` interface
3. **Adapters Layer**: Implement `api_standings_adapter` using F1 API client
4. **Web Layer**: Create HTTP controller that uses `standings_service`

```go
// Domain layer
type RaceStanding struct {
    Position int
    Driver   string
    Team     string
    Points   int
}

// Ports layer
type StandingsRepository interface {
    FetchStandings() ([]RaceStanding, error)
}

// Adapters layer
type ApiStandingsAdapter struct {
    client *http.Client
}

func (a *ApiStandingsAdapter) FetchStandings() ([]RaceStanding, error) {
    // API call implementation
}
```

## Consequences

### Positive
- Clear separation between business logic and infrastructure
- Easy to mock data access for testing
- Independent deployment of API adapter
- Domain model remains pure and testable

### Negative
- Increased boilerplate for interface definitions
- Requires additional infrastructure setup
- Initial development time for adapter implementation

### Neutral
- No immediate performance impact
- No database migration required
- No breaking changes to existing API

## Implementation

### Phases
1. **Phase 1 (Domain & Ports)**: Implement domain entities and repository interface
2. **Phase 2 (Adapters)**: Build API adapter implementation
3. **Phase 3 (Web)**: Integrate with web layer controller

### Affected Layers
- [x] domain/
- [x] ports/
- [x] adapters/primary/
- [ ] adapters/secondary/
- [x] usecases/
- [ ] composition-root

### Migration Notes
None required. This implementation follows existing ADR-001 architecture pattern.