

# ADR-230920T1430: Implement GetLiveStandingsUseCase with WebSocket Adapter

## Status
proposed

## Date
2023-09-20

## Drivers
- Requirement for real-time sports standings updates
- Existing WebSocket infrastructure in secondary adapters
- Hexagonal architecture compliance (ports/adapters separation)

## Context
The system requires live sports standings updates, which must be delivered in real-time. Current implementations use polling mechanisms that introduce latency. The WebSocketLiveStandingsAdapter (ADR-007) exists in the secondary adapters layer and provides a standardized interface for real-time data delivery. This decision aligns with the hexagonal architecture's ports/adapters pattern (ADR-001) by separating the domain logic from implementation details. The adapter layer already handles WebSocket connections, authentication, and message parsing, which reduces development effort and maintains consistency with existing real-time systems.

## Decision
We will implement the GetLiveStandingsUseCase in the usecases layer to consume data from the WebSocketLiveStandingsAdapter in the secondary adapters layer. This involves:
1. Creating a new port interface in `ports/live-standings.ts`
2. Implementing the WebSocket adapter in `adapters/secondary/live-standings/websocket.ts`
3. Integrating the adapter with the usecase via dependency injection in the composition root

## Consequences

### Positive
- Real-time data delivery with reduced latency
- Reuses existing WebSocket infrastructure
- Maintains hexagonal architecture boundaries

### Negative
- Adds dependency on WebSocket reliability
- Requires additional error handling for network interruptions
- May introduce complexity in testing (mocking WebSocket connections)

### Neutral
- No immediate impact on domain logic
- Existing polling mechanism remains available as fallback

## Implementation

### Phases
1. **Phase 1 (Tiers 2-3)**: Implement port interface and WebSocket adapter
2. **Phase 2 (Tiers 4-5)**: Integrate adapter with GetLiveStandingsUseCase

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new implementation with no existing counterpart)