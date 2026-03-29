

# ADR-230315T1200: Implement Todo REST API with Hexagonal Architecture

## Status
proposed

## Date
2023-03-15

## Drivers
- User requirement for CRUD operations on todo items
- Existing hexagonal architecture enforcement (ADR-001)
- Need for JSON API with Axum
- In-memory storage requirement

## Context
The project requires a REST API for todo management with three endpoints: POST /todos (create), GET /todos (list), and DELETE /todos/:id (remove). Implementation must follow hexagonal architecture principles enforced by the hex framework. The solution must use in-memory storage for simplicity while maintaining clear separation of concerns. The existing architecture has established domain, ports, and adapters layers, with ADR-001 defining the foundational hexagonal pattern.

## Decision
We will implement the todo API using hexagonal architecture layers:
1. **Domain Layer**: Define `Todo` struct and business rules
2. **Ports Layer**: Create `TodoRepository` trait with CRUD operations
3. **Adapters Layer**: Implement in-memory storage adapter for `TodoRepository`
4. **Composition Root**: Configure Axum routes to use the repository

The implementation will follow these dependencies:
- Domain imports only domain
- Ports import only domain
- Adapters import only ports
- Composition root imports all layers

## Consequences

### Positive
- Clear separation of business logic from infrastructure
- Easy to replace in-memory storage with database later
- Testable domain layer without external dependencies
- Consistent with existing hex architecture

### Negative
- Additional abstraction layer may increase complexity
- In-memory storage has no persistence (data lost on restart)
- Requires more boilerplate code than direct implementation

### Neutral
- Uses existing Axum framework integration
- Maintains JSON response format consistency
- No immediate performance impact

## Implementation

### Phases
1. **Phase 1**: Implement domain model and ports (Tiers 0-2)
   - Create `domain/todo.rs` with `Todo` struct
   - Define `ports/todo_repository.rs` trait
   - Write unit tests for domain logic

2. **Phase 2**: Implement in-memory adapter (Tier 3)
   - Create `adapters/secondary/in_memory_repository.rs`
   - Implement CRUD operations using Vec
   - Configure Axum routes in composition root

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new feature implementation)