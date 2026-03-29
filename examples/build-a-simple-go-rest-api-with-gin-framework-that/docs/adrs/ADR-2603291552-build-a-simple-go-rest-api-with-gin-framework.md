

# ADR-230315T1200: Gin REST API with Hexagonal CRUD for Todos

## Status
proposed

## Date
2023-03-15

## Drivers
- User requirement for a simple Go REST API with CRUD endpoints for todos
- Existing Hexagonal Architecture foundation (ADR-001)
- Need for testable, maintainable implementation

## Context
The project requires a new REST API using Gin framework to manage todo items. This must align with existing Hexagonal Architecture principles while providing a clear separation of concerns. The API will need to handle HTTP requests, business logic, and persistence in a way that maintains the architecture's integrity. Key constraints include:
- Must not violate Hexagonal boundary rules (domain imports only domain, ports import only domain, adapters never import other adapters)
- Must integrate with existing project structure and ADR-001 Hexagonal foundation
- Requires testable implementation using dependency injection (ADR-014)

## Decision
We will implement a Hexagonal Architecture pattern for the todo API with Gin framework. The decision involves:
1. Creating a `domain` package containing the core todo business logic and entities
2. Developing `ports` packages defining interfaces for HTTP (ports/primary) and persistence (ports/secondary)
3. Implementing `adapters` packages that implement the port interfaces using Gin and database drivers
4. Using Gin's router to create HTTP endpoints that delegate to use cases
5. Structuring the composition root to wire dependencies according to Hexagonal rules

## Consequences

### Positive
- Clear separation of concerns between HTTP handling and business logic
- Easier testing of business logic without HTTP dependencies
- Simplified persistence changes (e.g., switching databases)
- Maintained compliance with Hexagonal Architecture principles

### Negative
- Initial setup complexity for new developers
- Additional boilerplate code for interface definitions
- Requires careful dependency management to avoid boundary violations

### Neutral
- No immediate performance impact
- No changes to existing ADR-001 Hexagonal foundation

## Implementation

### Phases
1. **Phase 1 (Domain & Use Cases)**: Implement core todo business logic and use cases in `domain` and `usecases` packages
2. **Phase 2 (Ports)**: Define HTTP and persistence interfaces in `ports/primary` and `ports/secondary`
3. **Phase 3 (Adapters)**: Implement Gin adapters in `adapters/primary` and database adapters in `adapters/secondary`
4. **Phase 4 (Composition Root)**: Wire dependencies in main.go using Gin router

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new implementation)