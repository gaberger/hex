

# ADR-230315: HexagonalBackend for F1 Standings

## Status
proposed

## Date
2023-03-15

## Drivers
- Need to maintain strict hexagonal architecture boundaries (ADR-001)
- Require testable domain logic independent of external systems
- Support multiple data sources (API, mock, historical DB)
- Enable parallel development of frontend and backend

## Context
The F1 standings app requires complex data processing across multiple domains (driver stats, race results, constructor standings). While ADR-001 established hexagonal architecture as the foundational pattern, the current implementation lacks explicit domain layer isolation and port/adapter definitions for data sources. The existing ADR-005 quality gates require testability, but current test coverage is limited. The app must support both live data (via API) and historical data (via mock/DB) without coupling domain logic to implementation details.

## Decision
We will implement a strict hexagonal architecture with dedicated domain, ports, and adapters layers. The domain layer will contain all business logic (standings calculation, stat aggregation), ports will define interfaces for data access (RaceResultsPort, DriverStatsPort), and adapters will implement these ports for different data sources (APIAdapter, MockAdapter, HistoricalDBAdapter). This enforces ADR-001's boundary rules while enabling ADR-005 testability requirements.

## Consequences

### Positive
- Clear separation of domain logic from external systems
- Simplified testing through mock adapters
- Future-proof data source changes
- Parallel development of frontend and backend

### Negative
- Initial setup complexity for new developers
- Additional infrastructure for adapter management
- Potential performance overhead from adapter abstraction

### Neutral
- Existing API integration will require adapter refactoring
- Historical data storage decisions deferred to future ADR

## Implementation

### Phases
1. **Phase 1 (Tiers 0-2)**: Implement domain layer with core business logic and ports definitions. Create mock adapter for unit testing. (ADR-005 compliance)
2. **Phase 2 (Tiers 3-4)**: Implement API adapter for live data and historical DB adapter for offline mode. Integrate with existing API services.

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new architecture implementation)