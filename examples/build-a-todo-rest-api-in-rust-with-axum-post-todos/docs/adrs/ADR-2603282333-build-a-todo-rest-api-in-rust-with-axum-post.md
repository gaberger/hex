

# ADR-230615123456: Implement Todo REST API with Hexagonal Layers

## Status
proposed

## Date
2023-06-15

## Drivers
- User requirement for REST API endpoints (POST/GET/DELETE /todos)
- Existing hex architecture enforcement (ADR-001)
- In-memory storage requirement
- JSON response format

## Context
The project requires implementing a simple todo REST API using Rust and Axum. The API must support CRUD operations for todo items with in-memory storage and JSON responses. The existing hex architecture (ADR-001) mandates strict layer boundaries: domain layer (business logic), ports layer (interfaces), adapters layer (implementation), and use cases layer (application logic). The hex framework enforces that:
- Domain layer imports only domain
- Ports layer imports only domain
- Adapters layer imports only ports
- Use cases layer imports only ports
- Composition root ties everything together

## Decision
We will implement the todo API using the hexagonal architecture layers as follows:

1. **Domain Layer**: Define `Todo` struct and business rules
2. **Ports Layer**: Create `Storage` trait with `create`, `list`, and `delete` methods
3. **Adapters Layer**: Implement `MemoryStorage` struct that satisfies `Storage` trait
4. **Use Cases Layer**: Implement `CreateTodoUseCase`, `ListTodosUseCase`, and `DeleteTodoUseCase`
5. **Composition Root**: Wire everything together in `main.rs`

## Consequences

### Positive
- Clear separation of concerns between business logic and implementation
- Easy to swap storage implementations (e.g., switch to database)
- Testable domain logic without HTTP dependencies
- Consistent with existing hex architecture (ADR-001)

### Negative
- Increased boilerplate code for simple CRUD operations
- Initial setup complexity for new developers
- Performance overhead from interface abstraction

### Neutral
- In-memory storage limitation (data lost on restart)
- No persistence layer implemented

## Implementation

### Phases
1. **Phase 1**: Implement domain layer and ports layer (Tiers 0-2)
   - Define `Todo` struct in `domain/todo.rs`
   - Create `Storage` trait in `ports/storage.rs`

2. **Phase 2**: Implement primary adapters (Tiers 3-4)
   - Implement `MemoryStorage` in `adapters/primary/memory.rs`
   - Implement use cases in `usecases/todo.rs`

3. **Phase 3**: Implement composition root (Tier 5)
   - Wire everything together in `composition_root.rs`

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new feature implementation)