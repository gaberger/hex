

#ADR-230817001: Implement Todo REST API with Hexagonal Axum Adapter

## Status
proposed

## Date
2023-08-17

## Drivers
- Requirement to build a REST API for todo management
- Existing hex framework enforcement
- Axum as primary HTTP adapter

## Context
The project requires a REST API for todo list management using Rust and Axum. This must adhere to the existing hexagonal architecture enforced by the hex framework. The API will need to handle CRUD operations for todo items while maintaining separation between business logic and infrastructure concerns. Existing ADRs establish the hexagonal pattern (ADR-001) and Axum as the primary web adapter (ADR-003), but no specific implementation exists for todo functionality.

## Decision
We will implement a todo REST API using Axum as the primary adapter, following the hexagonal architecture pattern. The implementation will consist of:
1. **Domain Layer**: Define `Todo` struct and business logic (ADR-001 compliant)
2. **Ports Layer**: Create `TodoRepository` trait defining CRUD operations
3. **Adapters/Secondary**: Implement in-memory storage adapter for development
4. **Adapters/Primary**: Build Axum routes and handlers using the repository port
5. **Composition Root**: Configure Axum application with todo routes

## Consequences

### Positive
- Clear separation between business logic and HTTP concerns
- Testable domain logic without infrastructure dependencies
- Easy adapter swapping for different persistence mechanisms
- Consistent pattern with existing hex framework implementation

### Negative
- Additional abstraction layer may increase initial complexity
- Requires careful dependency management between layers
- Testing requires mock repository implementation
- Axum-specific error handling needs integration

### Neutral
- No immediate performance impact
- No database integration required for initial implementation

## Implementation

### Phases
1. **Phase 1**: Implement domain layer and repository port (Tier 0)
2. **Phase 2**: Build Axum adapter and integration tests (Tier 1)
3. **Phase 3**: Add persistence adapter (Tier 2)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new implementation)