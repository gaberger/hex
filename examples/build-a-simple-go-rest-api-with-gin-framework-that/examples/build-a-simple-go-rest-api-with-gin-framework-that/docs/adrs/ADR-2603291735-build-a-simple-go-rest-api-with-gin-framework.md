# ADR-230720T1430: Gin-Based Todo REST API with Hexagonal Architecture

## Status
proposed

## Date
2023-07-20

## Drivers
- User requirement for CRUD endpoints
- Existing hexagonal architecture enforcement
- Gin framework compatibility

## Context
The project requires a new REST API for todo management using Go and Gin. This must integrate with existing hexagonal architecture layers while maintaining separation of concerns. The API will need domain models, port interfaces, and adapter implementations. Existing ADR-001 establishes hexagonal architecture as foundational, requiring strict adherence to layer boundaries.

## Decision
We will implement a Todo REST API using Gin framework following hexagonal architecture principles. The implementation will follow these phases:

1. **Domain Layer (Tier 0)**: Create `domain/todo.go` with Todo struct and business rules
2. **Ports Layer (Tier 1)**: Define `ports/todo_repository.go` interface
3. **Usecases Layer (Tier 2)**: Implement `usecases/todo_usecases.go` with CRUD operations
4. **Adapters Layer (Tier 3)**: Create `adapters/gin/todo_handler.go` Gin router handler
5. **Composition Root**: Wire dependencies in `main.go`

## Consequences

### Positive
- Clear separation of business logic from HTTP concerns
- Testable domain layer without Gin dependencies
- Easy adapter swapping for different HTTP frameworks
- Consistent architecture with existing project

### Negative
- Initial setup complexity for new developers
- Additional boilerplate for interface definitions
- Requires discipline to maintain layer boundaries

### Neutral
- Gin framework choice doesn't affect domain logic
- Existing ADR-001 compliance maintained

## Implementation

### Phases
1. **Phase 1 (Domain & Ports)**: Implement core business logic and interfaces
2. **Phase 2 (Usecases)**: Connect domain with port implementations
3. **Phase 3 (Adapters)**: Create Gin-specific handler
4. **Phase 4 (Composition Root)**: Integrate all components

### Affected Layers
- [x] domain/
- [x] ports/
- [x] usecases/
- [x] adapters/primary/
- [ ] adapters/secondary/
- [ ] composition-root

### Migration Notes
None - new implementation with no existing code to migrate