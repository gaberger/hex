

# ADR-231005: Build TodoList REST API with Axum

## Status
proposed

## Date
2023-10-05

## Drivers
- User requirement for a todo list REST API using Rust and Axum
- Existing hex framework enforcement requiring hexagonal architecture compliance
- Need to demonstrate Axum integration within established port/adapter boundaries

## Context
The project requires implementing a todo list API using Rust and Axum while maintaining strict adherence to the hexagonal architecture enforced by the hex framework. This decision must respect existing ADR-001 (Hexagonal Architecture foundation) and ADR-003 (multi-language support) while introducing a new primary adapter (HTTP) for the todo list domain. The solution must avoid violating hex boundary rules (e.g., domain layer imports only domain, ports import only domain, adapters never import other adapters). The implementation must follow the established ADR-014 dependency injection pattern for test isolation and ADR-015 SQLite persistence for state management.

## Decision
We will implement the todo list API using Axum as the primary HTTP adapter, with the following hex layer implementation:

1. **Domain Layer**: Create `domain/todo` module containing business logic (e.g., `Todo`, `TodoRepository`, `TodoService`). This layer will define interfaces for persistence and application logic without implementation details.

2. **Ports Layer**: Implement `ports/todo` module defining interfaces (traits) for the domain layer to interact with external systems. This includes `TodoRepository` trait for persistence operations.

3. **Adapters Layer**: Create `adapters/primary/todo` module implementing the `TodoRepository` trait using SQLite (ADR-015) and Axum handlers. This layer will handle HTTP requests/responses and database interactions.

4. **Composition Root**: Implement `composition-root.rs` to wire dependencies (ADR-014) and configure Axum application with todo list routes.

The implementation order will follow dependency hierarchy: domain → ports → adapters → composition root.

## Consequences

### Positive
- Maintains strict hexagonal architecture compliance
- Enables easy swapping of persistence/adapter implementations
- Facilitates testing via dependency injection (ADR-014)
- Leverages existing SQLite persistence pattern (ADR-015)

### Negative
- Additional abstraction layer may increase initial complexity
- Requires careful boundary definition between layers
- Axum-specific implementation details confined to adapter layer

### Neutral
- No immediate performance impact
- No changes to existing multi-language support (ADR-003)
- No new dependencies beyond Axum and SQLite

## Implementation

### Phases
1. **Phase 1**: Implement domain and ports layers for todo functionality (tdd with unit tests)
2. **Phase 2**: Implement primary adapter layer with Axum handlers and SQLite integration
3. **Phase 3**: Implement composition root to wire dependencies and configure Axum

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new implementation)