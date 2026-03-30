

# ADR-230315123456: Implement URL Shortener REST API in main.go

## Status
proposed

## Date
2023-03-15

## Drivers
- Feature requirement to build a URL shortener REST API in Go
- Single-binary project constraint
- Hexagonal architecture enforcement via hex framework

## Context
The project requires implementing a URL shortener REST API with in-memory storage as a single binary executable. Existing architecture (ADR-001) mandates hexagonal design with ports/adapters enforced by the hex framework. The solution must operate as a self-contained binary without external dependencies. In-memory storage implies no persistence layer, simplifying data management but requiring careful state handling.

## Decision
We will implement the URL shortener REST API entirely within `main.go` as the composition root, using the hex framework to wire domain logic, ports, and adapters. The in-memory storage adapter will be implemented as a simple map-based repository, adhering to hex boundary rules where domain models import only domain, ports import only domain, and adapters never import other adapters. The composition root will initialize the application with this adapter and expose HTTP handlers via the hex framework's built-in router.

## Consequences

### Positive
- Simplified deployment with single binary
- Strict adherence to hex architecture boundaries
- Clear separation of concerns between domain, ports, and adapters
- No external dependencies beyond standard library

### Negative
- In-memory storage lacks persistence (state lost on restart)
- Limited scalability due to in-memory constraints
- No built-in monitoring or health checks

### Neutral
- No immediate impact on existing hex framework integrations
- Maintains project's existing ADR-001 hexagonal architecture compliance

## Implementation

### Phases
1. **Phase 1 (Tiers 0-1)**: Implement domain model and ports (URLShortener interface)
2. **Phase 2 (Tiers 2-3)**: Implement in-memory storage adapter and composition root
3. **Phase 3 (Tiers 4-5)**: Add HTTP handlers and router integration

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new feature implementation)