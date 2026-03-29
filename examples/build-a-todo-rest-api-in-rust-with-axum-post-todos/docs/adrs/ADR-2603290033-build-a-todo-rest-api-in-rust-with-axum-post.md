

#ADR-230315123456: In-Memory Todo API with Hexagonal Architecture

## Status
proposed

## Date
2023-03-15

## Drivers
- User requirement for a minimal REST API with CRUD operations
- Existing hexagonal architecture constraints
- No external dependencies required for MVP

## Context
The project requires a simple todo API with POST/GET/DELETE endpoints. The existing hexagonal architecture (ADR-001) mandates strict layer boundaries: domain layer contains business logic, ports define interfaces, and adapters implement storage. In-memory storage is acceptable for MVP but must not violate layer boundaries. The solution must integrate with existing Axum framework and composition root patterns.

## Decision
We will implement the todo API using hexagonal architecture tiers:
1. **Domain Layer**: Create `todo` entity and `Todo` struct with business rules
2. **Ports Layer**: Define `TodoRepository` trait with `create`, `list`, and `delete` methods
3. **Adapters Layer**: Implement `InMemoryTodoRepository` struct that stores todos in a `Vec`
4. **Composition Root**: Wire `InMemoryTodoRepository` into Axum routes via `compose` function

## Consequences

### Positive
- Maintains architectural integrity by avoiding external dependencies
- Enables future persistence migration without domain changes
- Clear separation of concerns between business logic and storage

### Negative
- Data loss on application restart (in-memory limitation)
- No persistence guarantees for production use
- Requires additional error handling for empty state scenarios

### Neutral
- Implementation complexity remains minimal due to simple requirements

## Implementation

### Phases
1. **Phase 1**: Implement domain layer and ports layer (tiers 0-1)
2. **Phase 2**: Implement in-memory adapter and composition root (tiers 2-3)

### Affected Layers
- [x] domain/
- [x] ports/
- [x] adapters/secondary/
- [x] usecases/
- [ ] composition-root

### Migration Notes
None (new feature with no existing implementation)