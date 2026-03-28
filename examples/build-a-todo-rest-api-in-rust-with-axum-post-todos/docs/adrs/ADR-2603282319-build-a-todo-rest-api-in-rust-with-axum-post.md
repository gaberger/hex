

# ADR-230720T1430: Implement Hexagonal REST API for Todo Service

## Status
proposed

## Date
2023-07-20

## Drivers
- User requirement for REST API endpoints (POST/GET/DELETE /todos)
- Existing hexagonal architecture enforcement (ADR-001)
- Need for testability via dependency injection (ADR-014)
- In-memory storage requirement (ADR-015)

## Context
The project requires a REST API for todo management with Axum. The solution must:
1. Implement three endpoints: POST /todos, GET /todos, DELETE /todos/:id
2. Use in-memory storage (ADR-015)
3. Adhere to hexagonal architecture boundaries
4. Support JSON responses
5. Maintain testability through dependency injection

Existing constraints:
- Hexagonal architecture enforced (ADR-001)
- Repository pattern required for storage (ADR-015)
- Primary/secondary adapter separation mandatory
- Domain layer must remain pure (ADR-001)

## Decision
We will implement a hexagonal REST API for todo management using Axum. The solution will follow these layers:

1. **Domain Layer** (tier 0)
   - Define `Todo` model and business rules
   - Create `TodoRepository` port interface

2. **Ports Layer** (tier 1)
   - Implement `TodoRepository` port with in-memory storage
   - Create `TodoUseCase` interface for business logic

3. **Adapters Layer** (tier 2)
   - Primary HTTP adapter: `TodoHandler` implementing `TodoUseCase`
   - Secondary storage adapter: `InMemoryTodoRepository`

4. **Composition Root** (tier 3)
   - Wire dependencies: `TodoHandler` → `InMemoryTodoRepository`

## Consequences

### Positive
- Clear separation of concerns improves maintainability
- Testability via dependency injection (ADR-014)
- Easy to replace storage implementation later
- Domain logic remains pure and testable

### Negative
- Increased boilerplate code for small project
- Initial setup complexity for new developers
- Performance overhead from HTTP request/response mapping

### Neutral
- In-memory storage limitation for production use
- JSON serialization overhead

## Implementation

### Phases
1. **Phase 1 (Domain & Ports)** - Implement `domain/todo.rs` and `ports/todo_repository.rs`
2. **Phase 2 (Adapters)** - Implement `adapters/http/todo_handler.rs` and `adapters/storage/in_memory.rs`
3. **Phase 3 (Composition)** - Implement `composition_root.rs` to wire components

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new implementation)