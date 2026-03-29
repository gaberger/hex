

# ADR-230720T1430: Implement Todo REST API with Axum and Hexagonal Architecture

## Status
proposed

## Date
2023-07-20

## Drivers
- User requirement for REST API endpoints (POST/GET/DELETE /todos)
- Existing hex architecture constraints
- In-memory storage requirement
- JSON response format

## Context
The project requires implementing a simple todo REST API using Rust and Axum. The API must support CRUD operations for todo items with in-memory storage and JSON responses. The existing hexagonal architecture (ADR-001) enforces strict layer boundaries: domain layer imports only domain, ports import only domain, and adapters never import other adapters. The solution must respect these boundaries while providing a functional API.

## Decision
We will implement the todo API using the hexagonal architecture pattern with Axum. The domain layer will define the todo entity and business rules. The ports layer will define the repository interface. The adapters/secondary layer will implement in-memory storage. The composition root will wire these components together. All HTTP routing and JSON serialization will occur in the adapters/primary layer.

## Consequences

### Positive
- Maintains strict hex layer boundaries
- Enables future persistence adapter swapping
- Isolates business logic from infrastructure concerns
- Simplifies testing through dependency injection

### Negative
- Requires additional boilerplate for port/adapter setup
- In-memory storage lacks persistence
- JSON serialization adds overhead

### Neutral
- Axum integration requires learning curve
- No immediate performance benefits

## Implementation

### Phases
1. **Domain Layer** (Tier 0): Define `Todo` entity and business rules
2. **Ports Layer** (Tier 1): Implement `TodoRepository` interface
3. **Adapters/Secondary** (Tier 2): Implement in-memory storage
4. **Adapters/Primary** (Tier 3): Implement Axum routes and JSON handling

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/secondary/
- [ ] adapters/primary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new feature implementation)