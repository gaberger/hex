# ADR-2603270830: Hexagonal Todo REST API with Axum

## Status
proposed

## Date
2025-03-27

## Drivers
- User request for a simple, complete Rust REST API example following hexagonal architecture
- Need to demonstrate practical hex framework usage with real-world patterns
- Requirement for easily extensible example that maintains clean architecture boundaries
- Educational value for developers learning hexagonal architecture in Rust

## Context
The hex framework enforces strict architectural boundaries but lacks a comprehensive, end-to-end example showing how to implement a complete REST API. Developers need to see how to connect a web framework (Axum) to the hexagonal layers while maintaining the dependency rule (domain → ports → adapters). The example must demonstrate health checks and full CRUD operations for a todo list entity, providing a minimal but complete implementation that can serve as a template for real applications.

Existing project ADRs establish key patterns (self-sufficient agents, dependency injection, operational resilience) but don't address the common scenario of building REST APIs. The hex framework's tier-based approach (domain tier 0, ports tier 1, adapters tiers 2-3, usecases tier 4) provides a clear structure but needs concrete demonstration with HTTP endpoints.

## Decision
We will create a hexagonal todo REST API example application using Axum with the following structure:

1. **Domain Layer (Tier 0)**: Define `Todo` aggregate root with business rules
2. **Ports Layer (Tier 1)**: Create `TodoRepository` trait (secondary port) and API request/response DTOs
3. **Adapters Layer (Tiers 2-3)**:
   - Primary: Axum REST handlers mapping HTTP to usecase calls
   - Secondary: In-memory repository implementing `TodoRepository`
4. **Usecases Layer (Tier 4)**: CRUD operations encapsulating business logic
5. **Composition Root**: Wire all components for dependency injection

The architecture will strictly enforce hex boundaries: domain knows nothing external, ports define interfaces, adapters implement them, usecases orchestrate domain through ports, and composition root assembles everything.

## Consequences

### Positive
- Clear demonstration of hex framework's value for real-world applications
- Provides template for other REST APIs using Axum or alternative web frameworks
- Maintains testability through dependency injection and trait boundaries
- Shows practical separation between HTTP concerns and business logic

### Negative
- Additional boilerplate compared to traditional monolithic Axum apps
- Learning curve for developers unfamiliar with hexagonal architecture patterns
- Requires explicit wiring that simple examples might omit

### Neutral
- Example will use in-memory storage for simplicity, not production persistence
- Demonstrates hex boundary enforcement but not all framework features
- Focuses on structural patterns rather than advanced business logic

## Implementation

### Phases
1. **Tier 0 (Domain)**: Define `Todo` entity with validation logic and domain errors
2. **Tier 1 (Ports)**: Create repository trait and DTOs for API communication
3. **Tier 2-3 (Adapters)**: Implement in-memory repository and Axum route handlers
4. **Tier 4 (Usecases)**: Add CRUD operations (create, read, update, delete, list)
5. **Wire-up**: Configure composition root with health endpoint and todo routes

### Affected Layers
- [x] domain/ (Todo entity, domain errors)
- [x] ports/ (TodoRepository trait, API DTOs)
- [x] adapters/primary/ (Axum handlers, health endpoint)
- [x] adapters/secondary/ (InMemoryTodoRepository)
- [x] usecases/ (CRUD operations)
- [x] composition-root (app configuration)

### Migration Notes
None - this is a new example application with no migration path from existing code.