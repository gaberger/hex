

# ADR-2406231500: Key-Value Store REST API with Axum

## Status
proposed

## Date
2024-06-23

## Drivers
- Requirement to implement a REST API for key-value storage
- Need to maintain hexagonal architecture boundaries
- Utilization of Axum as primary HTTP framework

## Context
The project requires a REST API for key-value storage operations (GET/PUT/DELETE). This must integrate with existing hexagonal architecture layers while using Axum as the HTTP adapter. The solution must respect domain layer isolation, port definitions, and adapter boundaries. Existing hex framework enforces strict layer dependencies (domain → ports → adapters) with no cross-layer imports.

## Decision
We will implement a REST API using Axum as the primary HTTP adapter. The domain layer will define ports for key-value operations (Get, Set, Delete) with pure Rust interfaces. The Axum adapter will implement these ports using Axum's request/response handling. The use cases layer will orchestrate domain logic around key-value operations. This maintains strict hexagonal boundaries where:
- Domain layer imports only ports
- Ports import only domain
- Axum adapter imports only ports
- Use cases import both domain and ports

## Consequences

### Positive
- Clear separation of concerns between HTTP handling and business logic
- Testable domain layer independent of Axum
- Axum-specific concerns (routing, middleware) contained in adapter layer
- Easy to replace HTTP framework in future without domain changes

### Negative
- Additional abstraction layer may increase cognitive overhead
- Axum-specific error handling must be mapped to domain errors
- Initial setup requires defining port interfaces and adapter implementations

### Neutral
- Performance impact of HTTP adapter layer is negligible for key-value operations
- Memory usage remains within expected bounds

## Implementation

### Phases
1. **Phase 1**: Define domain ports and use cases for key-value operations
2. **Phase 2**: Implement Axum adapter layer binding ports to HTTP endpoints
3. **Phase 3**: Integrate with existing hex framework composition root

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new feature implementation)