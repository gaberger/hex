

# ADR-230720142300: F1 Web App Hexagonal Frontend Integration

## Status
proposed

## Date
2023-07-20

## Drivers
- Need to integrate React frontend with hexagonal backend
- Requirement for real-time driver standings updates
- Multiple data sources (live API, historical DB, third-party feeds)
- Maintain architecture score above 7800

## Context
The existing hexagonal architecture (ADR-001) defines strict layer boundaries where domain models never import infrastructure. The frontend currently lacks integration with the backend's ports/adapters. We need to:
1. Create a React frontend that respects hexagonal boundaries
2. Implement real-time updates for live standings
3. Handle multiple data sources without violating layer rules
4. Maintain separation between UI concerns and business logic

## Decision
We will implement a React frontend that communicates exclusively with the backend's ports/adapters layer. The frontend will:
1. Use WebSockets for real-time updates to the `driver-standings` port
2. Create a new `ui-adapter` layer containing React components
3. Implement a `composition-root` that connects React to the backend
4. Use dependency injection for all external dependencies (ADR-014)

## Consequences

### Positive
- Maintains architecture score by preserving layer boundaries
- Enables future frontend technology changes without backend impact
- Real-time updates improve user experience for live standings
- Clear separation between UI and business logic

### Negative
- Additional complexity in setting up WebSocket connections
- Potential performance overhead from serialization/deserialization
- Requires careful boundary definition between UI and domain layers

### Neutral
- No immediate impact on existing backend services
- Existing ADR-009 (Ruflo) may need adaptation for WebSockets

## Implementation

### Phases
1. **Phase 1 (Tiers 0-2):** Implement React composition-root and basic UI-adapter layer
2. **Phase 2 (Tiers 3-4):** Integrate WebSocket connections to driver-standings port
3. **Phase 3 (Tiers 5):** Add historical data fetching and constructor tables

### Affected Layers
- [ ] domain/ (unchanged)
- [ ] ports/ (new `ui-adapter` port)
- [ ] adapters/primary/ (unchanged)
- [ ] adapters/secondary/ (new WebSocket adapter)
- [ ] usecases/ (unchanged)
- [ ] composition-root (new React integration)

### Migration Notes
None required. Existing backend services remain unchanged.