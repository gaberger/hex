

# ADR-230920T1430: REST API Implementation with Axum

## Status
proposed

## Date
2023-09-20

## Drivers
- User requirement for a todo list REST API
- Hexagonal architecture enforcement via hex framework
- Axum as primary HTTP adapter requirement
- Existing ADR-001 (Hexagonal Architecture) compliance

## Context
The project requires implementing a REST API for todo list management using Rust and Axum. This must adhere to the hexagonal architecture enforced by the hex framework, which enforces strict layer boundaries: domain layer (business logic), ports (interfaces), primary adapters (external interactions), and secondary adapters (infrastructure). The API must expose CRUD operations for todo items while maintaining separation of concerns and testability. Existing infrastructure includes SQLite persistence (ADR-015), Git worktrees (ADR-004), and multi-language support (ADR-003).

## Decision
We will implement the todo list REST API using Axum as the primary HTTP adapter. The domain layer will contain the core business logic for todo items, while ports will define interfaces for data access. The Axum adapter will implement these ports to handle HTTP requests and responses. Secondary adapters will handle SQLite persistence through the ports. Use cases will orchestrate domain logic with ports and adapters. The composition root will wire these components together.

## Consequences

### Positive
- Clear separation of concerns between HTTP handling and business logic
- Testability through port interfaces
- Compliance with existing hexagonal architecture
- Direct integration with SQLite persistence via secondary adapters

### Negative
- Increased complexity in wiring components
- Potential for port interface bloat
- Learning curve for new developers on Axum integration

### Neutral
- No immediate performance impact
- No changes to existing CI/CD pipeline

## Implementation

### Phases
1. **Phase 1**: Implement domain layer (todo item model, business rules) and ports (todo repository interface)
2. **Phase 2**: Implement Axum primary adapter (request handling, response serialization) and secondary SQLite adapter (persistence implementation)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new implementation)