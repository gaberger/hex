

# ADR-230907T1430: Implement Todo List REST API with Axum

## Status
proposed

## Date
2023-09-07

## Drivers
- User requirement for a todo list REST API
- Axum as preferred HTTP framework
- Hexagonal architecture enforcement
- Existing hex framework adoption

## Context
The project requires implementing a todo list REST API using Rust and Axum. This must adhere to the established hexagonal architecture (ADR-001) with clear separation between domain logic and infrastructure concerns. The API will need to support CRUD operations for todo items while maintaining the project's multi-language support (ADR-003) and Git worktree isolation (ADR-004) patterns. The solution must integrate with existing hex framework components while avoiding dependencies between non-adjacent layers.

## Decision
We will implement the todo list API using Axum with strict adherence to hexagonal architecture boundaries. The domain layer will contain pure business logic, ports will define interfaces for external interactions, and adapters will implement HTTP endpoints. The usecases layer will coordinate between domain models and ports. Implementation will follow dependency order: domain → ports → usecases → adapters.

## Consequences

### Positive
- Clear separation of concerns between business logic and HTTP handling
- Maintainability through dependency inversion
- Easy testing of domain logic without HTTP dependencies
- Compatibility with existing hex framework components

### Negative
- Increased boilerplate for interface definitions
- Potential performance overhead from adapter layer
- Requires careful boundary management between layers

### Neutral
- No immediate impact on existing system performance metrics
- No changes to current testing infrastructure

## Implementation

### Phases
1. **Phase 1**: Implement domain model (todo item structure, business rules) in `domain/`
2. **Phase 2**: Define ports (interfaces for todo operations) in `ports/`
3. **Phase 3**: Implement usecases (business logic orchestration) in `usecases/`
4. **Phase 4**: Create Axum adapters (HTTP endpoints) in `adapters/primary/`

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None - This is a greenfield implementation with no existing API to migrate.