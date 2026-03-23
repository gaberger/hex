# ADR-2603232200: Simple Todo REST API with Axum

## Status
proposed

## Date
2023-03-26

## Drivers
- Need a clear, working example demonstrating hexagonal architecture implementation in Rust
- Requirement for a minimal but complete REST API with standard endpoints (health check + CRUD)
- Axum's growing popularity as a Rust web framework makes it a practical choice for examples
- Example must be immediately runnable for educational purposes while maintaining architectural purity

## Context
We need to create a simple example application that demonstrates hexagonal architecture using the hex framework. While the architecture itself is well-defined through existing ADRs, we lack a concise, runnable example that new developers can study to understand how all the pieces fit together.

The application requires a health check endpoint (standard requirement for observability and orchestration) and CRUD operations for a todo list (a canonical example that covers most REST API patterns). Axum provides a modern, ergonomic web framework that aligns with Rust's async ecosystem and has strong type safety features that complement hexagonal architecture's clean boundaries.

This decision primarily relates to ADR-2603232110 (Hexagonal Todo REST API with Axum), but provides a more focused, minimal implementation that can serve as a starting point for larger applications.

## Decision
We will create a minimal hexagonal REST API using Axum that implements health check and todo CRUD operations. The example will strictly follow hex framework conventions with these layers:

1. **domain/**: Contains `Todo` entity with fields (id, title, completed, created_at) and domain validation logic
2. **ports/**: Defines primary port `TodoPort` for use cases and secondary port `TodoRepository` for data persistence
3. **usecases/**: Implements CRUD operations (Create Todo, List Todos, Update Todo, Delete Todo) and health check use case
4. **adapters/primary/**: Contains Axum HTTP handlers that implement the adapter pattern for web requests
5. **adapters/secondary/**: In-memory repository implementation using `std::sync::Mutex<HashMap>` for simplicity
6. **composition-root/**: Wires all dependencies together and starts the Axum server

We will use Tower middleware for request logging and error handling, and include Swagger UI documentation via `utoipa` to demonstrate API documentation in a hexagonal context.

## Consequences

### Positive
- Provides immediately runnable example for developers learning hexagonal architecture
- Demonstrates how to integrate modern Rust web frameworks with hex framework boundaries
- Shows complete dependency flow from HTTP request → adapter → use case → domain → repository
- Minimal dependencies keep the example focused on architecture rather than features

### Negative
- In-memory repository is not production-suitable (lacks persistence)
- Simplified error handling may not cover all edge cases
- Single-server setup doesn't demonstrate distributed system concerns

### Neutral
- Example can be extended later with database adapters, authentication, etc.
- Uses sync Mutex for simplicity but demonstrates pattern for async adapters
- Provides baseline for comparing with other web frameworks (warp, actix-web)

## Implementation

### Phases
1. **Phase 1 (Tier 0-2)**: Setup hex project structure, define `Todo` domain entity and validation, create repository interface
2. **Phase 2 (Tier 3-4)**: Implement use cases, create Axum handlers with route definitions, implement in-memory repository
3. **Phase 3 (Tier 5)**: Compose application in composition root with dependency injection, add middleware, launch server

### Affected Layers
- [x] domain/
- [x] ports/
- [x] adapters/primary/
- [x] adapters/secondary/
- [x] usecases/
- [x] composition-root

### Migration Notes
None — This is a new example application without existing implementation to migrate.