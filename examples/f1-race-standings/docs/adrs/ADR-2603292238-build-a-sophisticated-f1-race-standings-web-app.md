

# ADR-240623142300: WebSocket Adapter for Live F1 Data

## Status
proposed

## Date
2024-06-23

## Drivers
- User requirement for live driver standings updates
- Need for real-time constructor championship tracking
- Historical season data retrieval from external APIs
- Maintain separation of concerns per hexagonal architecture

## Context
The application requires real-time updates for live standings while maintaining historical data access. Current architecture uses SQLite for persistence (ADR-015) but lacks a mechanism for live data synchronization. The hexagonal architecture enforces strict layer boundaries: domain layer (business logic) must not depend on external systems, while ports define interfaces and adapters implement them. Existing adapters include REST clients for historical data but no real-time transport mechanism.

## Decision
We will implement a WebSocket adapter layer to handle live F1 data synchronization. This will:
1. Create a `LiveStandingsPort` interface in the domain layer with methods like `subscribeToLiveStandings()`
2. Develop a `WebSocketLiveAdapter` in the adapters/secondary layer that implements this port
3. Integrate with F1's official API (via `f1-live-api` package) for real-time data
4. Maintain SQLite persistence for historical data while using WebSocket for live updates

## Consequences

### Positive
- Real-time driver standings updates without polling
- Reduced API load compared to frequent REST requests
- Clear separation between live data transport and historical storage
- Future-proof for adding additional live data streams

### Negative
- Added complexity in error handling for WebSocket connections
- Potential latency between WebSocket updates and SQLite persistence
- Requires additional monitoring for WebSocket connection health
- Initial implementation time for WebSocket adapter logic

### Neutral
- No direct impact on existing REST-based historical data access
- Maintains current SQLite persistence strategy

## Implementation

### Phases
1. **Phase 1 (Domain Layer)**: Define `LiveStandingsPort` interface with `subscribe()` method
2. **Phase 2 (Adapters)**: Implement `WebSocketLiveAdapter` with connection logic
3. **Phase 3 (Composition Root)**: Integrate adapter with React components
4. **Phase 4 (Testing)**: Add WebSocket integration tests

### Affected Layers
- [ ] domain/LiveStandingsPort.ts
- [ ] adapters/secondary/WebSocketLiveAdapter.ts
- [ ] composition-root/CompositionRoot.ts

### Migration Notes
None - WebSocket adapter will be implemented as a new layer without affecting existing REST-based historical data access