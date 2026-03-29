

# ADR-230720142200: Gin-Based Todo REST API with Hexagonal Architecture

## Status
proposed

## Date
2023-07-20

## Drivers
- User need for CRUD operations on todo items
- Existing hex framework enforcement
- Gin framework selection for REST implementation
- Need to maintain hexagonal architecture boundaries

## Context
The project requires a simple REST API for todo management using Go and Gin. This must adhere to the existing hexagonal architecture enforced by the hex framework. The API will need CRUD endpoints for todo items, which will require:
1. Domain layer definition for todo business logic
2. Ports layer defining interfaces for todo operations
3. Adapters layer implementing Gin handlers
4. Use cases layer coordinating domain and adapters
5. Composition root wiring everything together

Existing ADR-001 establishes hexagonal architecture as foundational, requiring strict adherence to layer boundaries. The solution must avoid any dependencies between non-adjacent layers (e.g., adapters cannot import domain models directly).

## Decision
We will implement the todo API using a hexagonal architecture with Gin as the primary adapter. The implementation will follow these specific decisions:

1. **Domain Layer**: Create `domain/todo` package containing:
   - `Todo` struct with ID, title, completed fields
   - Business rules in `domain/todo/service.go`
   - Domain events for state changes

2. **Ports Layer**: Define interfaces in `ports/todo`:
   - `TodoRepository` interface for data operations
   - `TodoUseCase` interface for business logic

3. **Adapters Layer**: Implement Gin handlers in `adapters/primary/gin`:
   - `todoHandler.go` for HTTP endpoints
   - `todoRepository.go` for database interactions
   - Adapters will import only domain and ports packages

4. **Use Cases Layer**: Implement business logic in `usecases/todo`:
   - `CreateTodoUseCase`, `GetTodoUseCase`, etc.
   - Use cases will import only ports layer

5. **Composition Root**: Wire everything together in `composition-root/main.go`:
   - Create Gin router
   - Bind handlers to routes
   - Inject dependencies via constructor injection

## Consequences

### Positive
- Clear separation of concerns between business logic and infrastructure
- Easy testing of domain logic without Gin dependencies
- Simplified database migrations through repository abstraction
- Consistent API design following REST principles

### Negative
- Increased boilerplate code for interface definitions
- More complex setup for dependency injection
- Potential for over-engineering in simple CRUD operations
- Requires strict adherence to layer boundaries during development

### Neutral
- Implementation will follow existing ADR-001 hexagonal architecture patterns
- Gin framework choice doesn't affect domain logic
- No immediate performance impact from abstraction layers

## Implementation

### Phases
1. **Phase 1 (Domain & Ports)**: Define domain models and interfaces (2 weeks)
   - Create `domain/todo/todo.go`
   - Implement `ports/todo/todo_repository.go`
   - Create `ports/todo/todo_usecase.go`

2. **Phase 2 (Adapters & Use Cases)**: Implement Gin handlers and business logic (3 weeks)
   - Build `adapters/primary/gin/todo_handler.go`
   - Implement `usecases/todo/todo_usecase.go`
   - Wire dependencies in composition root

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/gin/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None - This is a greenfield implementation with no existing API to migrate.