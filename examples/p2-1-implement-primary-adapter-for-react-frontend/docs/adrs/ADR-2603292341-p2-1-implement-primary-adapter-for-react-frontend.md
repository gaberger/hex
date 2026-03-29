

#ADR-2306151430: Implement primary adapter for React frontend (REST client for F1 data)

## Status
proposed

## Date
2023-06-15

## Drivers
- User requirement for React frontend integration with F1 data
- Hexagonal architecture enforcement requiring primary adapter implementation
- Need for REST-based communication between frontend and domain

## Context
The React frontend requires a primary adapter to fetch F1 data from an external API. This adapter must adhere to hexagonal architecture principles by communicating exclusively through ports defined in the `ports/` layer. The existing `ports/` layer contains domain-agnostic interfaces for data access, but no concrete REST client implementation exists for the frontend. This decision must respect the hex boundary rules where primary adapters (adapters/primary) can only import domain models and ports, never other adapters or secondary layers.

## Decision
We will implement a REST client in the `adapters/primary/` layer using a lightweight library like Axios or Fetch. This client will implement the `ports/F1DataPort.ts` interface to provide F1 data access to the frontend. The client will be injected into use cases via dependency injection in the composition root, maintaining the hexagonal architecture's separation of concerns. The implementation will follow these constraints:
- Primary adapter imports only domain models and ports
- No circular dependencies between adapters
- Client configuration will be injected via environment variables

## Consequences

### Positive
- Clear separation of frontend concerns from domain logic
- Easier testing of frontend components in isolation
- Consistent API contract enforced by port interface
- Future-proof for potential backend changes

### Negative
- Additional setup required for REST client configuration
- Potential performance overhead compared to direct API calls
- Requires maintaining port interface for client implementation

### Neutral
- Choice of REST library (Axios vs Fetch) is implementation detail
- Client implementation may require error handling adjustments

## Implementation

### Phases
1. **Phase 1**: Create `adapters/primary/F1Client.ts` implementing `ports/F1DataPort.ts` interface with basic GET requests
2. **Phase 2**: Integrate client into frontend composition root and implement initial F1 data fetching

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None