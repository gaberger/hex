

# ADR-230920T1430: Implement Todo REST API with Hexagonal Layers

## Status
proposed

## Date
2023-09-20

## Drivers
- User requirement for CRUD todo API endpoints
- Existing hexagonal architecture enforcement
- Bun runtime compatibility
- JSON response format consistency

## Context
The project requires implementing a todo REST API with Express and TypeScript. The existing hexagonal architecture (ADR-001) mandates strict layer boundaries: domain layer (business logic), ports (interfaces), and adapters (implementation). The API must use in-memory storage and return JSON responses. The bun runtime requires minimal dependencies and fast startup times. The architecture score of 7800/100 indicates strong adherence to hexagonal principles, but new implementations must maintain this integrity.

## Decision
We will implement the todo API using the hexagonal architecture tiers:
1. **Domain Layer**: Define `Todo` entity and business rules
2. **Ports Layer**: Create `TodoRepository` interface for storage operations
3. **Adapters Layer**: Implement Express routes using `TodoRepository` adapter
4. **Use Cases Layer**: Create `CreateTodoUseCase` and `DeleteTodoUseCase` classes

The implementation will follow these dependencies:
- Adapters import only ports and domain
- Ports import only domain
- Domain imports nothing
- Use cases import ports and domain

## Consequences

### Positive
- Clear separation of concerns between business logic and infrastructure
- Easy testing of domain logic without Express dependencies
- Consistent JSON response format across all endpoints
- Simplified future migration to other databases

### Negative
- Initial setup complexity for new developers
- Additional boilerplate for interface definitions
- Potential performance overhead from interface calls

### Neutral
- In-memory storage limitation for production use
- Express-specific error handling requirements

## Implementation

### Phases
1. **Phase 1**: Implement domain layer with `Todo` entity and validation rules
2. **Phase 2**: Create ports layer with `TodoRepository` interface
3. **Phase 3**: Implement adapters layer with Express routes
4. **Phase 4**: Create use cases layer with CRUD operations

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new feature implementation)