# ADR-2404241436: Real-time Development Session Tracking via Push API

## Status
proposed

## Date
2024-04-24

## Drivers
- Need for real-time visibility of development sessions across multiple machines
- Requirement to track phase transitions and cost updates from hex dev pipeline
- Existing WebSocket infrastructure for live updates (ADR-007)
- Current SpacetimeDB persistence layer for workplan state (existing infrastructure)

## Context
Development sessions in hex currently operate in isolation per machine, with no cross-instance visibility. Teams working on the same project cannot see each other's active development sessions, leading to coordination challenges and duplicate work. The existing notification system (ADR-007) provides WebSocket capabilities, and SpacetimeDB is used for persistent workplan state storage. The hex dev pipeline already generates phase transitions and cost metrics internally but lacks a mechanism to share this state externally.

## Decision
We will implement real-time development session tracking through a push-based architecture. The hex dev pipeline will POST session updates (phase transitions and cost updates) to `/api/push` with `type: dev_session`. Nexus will handle this push type by writing to the SpacetimeDB `workplan-state` table and broadcasting updates via existing WebSocket subscriptions. The `hex dev list` command will read live session state from nexus instead of local storage, making sessions visible across all team machines.

Implementation will follow hexagonal architecture:
- **domain/**: Add `DevSession` aggregate with phase, cost, and machine identification
- **ports/**: Define `DevSessionRepository` port for persistence and `DevSessionNotifier` port for broadcasts
- **adapters/secondary/**: Implement SpacetimeDB repository adapter and WebSocket notifier adapter
- **adapters/primary/**: Extend existing push HTTP adapter to handle `dev_session` type
- **usecases/**: Add `UpdateDevSessionUseCase` and `ListDevSessionsUseCase`

## Consequences

### Positive
- Real-time visibility of all development sessions across team machines
- Consistent cost tracking and phase progression monitoring
- Leverages existing WebSocket infrastructure without new subscriptions
- Reuses SpacetimeDB persistence with minimal schema changes

### Negative
- Adds network dependency for `hex dev list` command (requires nexus connectivity)
- Increases load on SpacetimeDB with frequent session updates
- Additional payload validation required for push endpoint

### Neutral
- Session data remains ephemeral (cleared when sessions end)
- Backward compatibility maintained for local-only operation modes

## Implementation

### Phases
1. **Tier 2-3**: Domain model and port definitions in domain/ and ports/
2. **Tier 4**: Secondary adapters for SpacetimeDB and WebSocket in adapters/secondary/
3. **Tier 3**: Use cases in usecases/
4. **Tier 1**: Primary adapters extending push endpoint in adapters/primary/
5. **Tier 5**: CLI adapter updates in hex client (separate repository)

### Affected Layers
- [x] domain/ (DevSession aggregate)
- [x] ports/ (Repository and Notifier interfaces)
- [x] adapters/primary/ (Extended push HTTP handler)
- [x] adapters/secondary/ (SpacetimeDB, WebSocket adapters)
- [x] usecases/ (Update and List use cases)
- [x] composition-root (Dependency wiring)

### Migration Notes
- `hex dev list` will query nexus by default with local fallback
- Push endpoint accepts both existing and new payload formats
- WebSocket subscriptions remain unchanged (existing clients receive new event types)
- Session history older than 24 hours may be purged from SpacetimeDB