

# ADR-240530H1234: Gin REST API with Hexagonal CRUD for Todos

## Status
proposed

## Date
2024-05-30

## Drivers
- User need for CRUD operations on todo resources
- Existing Hexagonal architecture enforcement (ADR-001)
- Gin framework as primary HTTP adapter (ADR-009 superseded)

## Context
The project requires a new REST API for todo management using Gin. This must adhere to Hexagonal architecture principles with clear separation between domain logic and infrastructure. The API will need HTTP endpoints for CRUD operations, requiring a structured implementation that respects existing ADR-001 boundaries. The solution must avoid direct dependencies between Gin handlers and domain models, instead using ports/adapters for communication.

## Decision
We will implement the todo API using Gin as the primary HTTP adapter, with the following Hexagonal structure:
1. **Domain Layer**: Define `Todo` struct and business rules
2. **Ports Layer**: Create repository interfaces (`TodoRepository`)
3. **Adapters Layer**: Implement Gin controllers and SQLite repository
4. **Use Cases Layer**: Create `CreateTodoUseCase`, `GetTodosUseCase`, etc.
5. **Composition Root**: Wire dependencies in main.go

The Gin handlers will only depend on the ports layer, while the domain layer remains isolated. The SQLite adapter will implement the repository interface, and Gin controllers will use the use cases layer.

## Consequences

### Positive
- Clear separation of concerns between HTTP and business logic
- Testable domain layer without Gin dependencies
- Easy adapter swapping (e.g., switch to PostgreSQL)
- Compliance with Hexagonal architecture boundaries

### Negative
- Increased boilerplate code for interface implementations
- More complex dependency injection setup
- Potential performance overhead from interface calls

### Neutral
- No immediate impact on existing system components
- Maintains ADR-001 architecture consistency

## Implementation

### Phases
1. **Phase 1**: Domain layer implementation (Todo struct, validation rules)
2. **Phase 2**: Ports layer creation (repository interfaces)
3. **Phase 3**: Adapters layer implementation (Gin controllers, SQLite repo)
4. **Phase 4**: Composition root wiring (main.go)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new implementation)