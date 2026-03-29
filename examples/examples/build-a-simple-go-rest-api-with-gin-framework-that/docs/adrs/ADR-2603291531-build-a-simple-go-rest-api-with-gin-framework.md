

# ADR-230315123456: Gin REST API with Hexagonal Architecture for Todo CRUD

## Status
proposed

## Date
2023-03-15

## Drivers
- User requirement for CRUD endpoints
- Existing hexagonal architecture enforcement
- Gin framework compatibility

## Context
The project requires a new REST API for todo management using Gin. The existing hexagonal architecture (ADR-001) mandates strict layer boundaries: domain models must not depend on external libraries, ports define interfaces for use cases, and adapters implement these interfaces. The Gin framework will serve as the primary adapter layer. This decision must respect the existing architecture constraints while implementing the required functionality.

## Decision
We will implement a Todo CRUD API using Gin as the primary adapter layer. The architecture will follow hexagonal principles with:
1. **Domain Layer**: `domain/todo.go` defining `Todo` struct and business rules
2. **Use Cases**: `usecases/todo_usecase.go` implementing CRUD operations
3. **Ports**: `ports/todo_port.go` defining interface for TodoRepository
4. **Adapters**: `adapters/gin/todo_handler.go` implementing Gin handlers
5. **Composition Root**: `composition_root.go` wiring dependencies

The Gin handlers will depend only on the TodoPort interface, which will be implemented by a repository adapter (SQLite in ADR-015). This maintains the dependency inversion principle and allows for easy swapping of persistence mechanisms.

## Consequences

### Positive
- Clear separation of concerns between business logic and web layer
- Testability through dependency injection
- Easy replacement of Gin with other frameworks
- Compliance with existing architecture standards

### Negative
- Additional boilerplate for interface definitions
- Learning curve for new developers
- Potential performance overhead from interface calls

### Neutral
- No immediate impact on existing functionality
- Maintains project's architectural consistency

## Implementation

### Phases
1. **Phase 1**: Implement domain model and use cases (Tiers 0-2)
2. **Phase 2**: Create port interface and repository adapter (Tiers 3-4)
3. **Phase 3**: Implement Gin handlers and composition root (Tiers 4-5)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new implementation)