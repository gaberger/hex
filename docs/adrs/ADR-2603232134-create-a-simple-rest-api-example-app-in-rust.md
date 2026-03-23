```markdown
# ADR-2603241130: Simple Todo REST API with Axum in Hex Structure

## Status
proposed

## Date
2024-03-26

## Drivers
- Need reference implementation for onboarding developers
- Demonstrate hexagonal architecture patterns with Axum
- Provide minimal viable product for Todo domain validation
- Establish baseline for future API extensions

## Context
We require a simple but architecturally sound implementation of a Todo REST API that:
1. Serves as foundation for more complex features
2. Validates our hex framework's Axum integration
3. Maintains strict layer separation per hexagonal principles
4. Implements standard RESTful practices for CRUD operations

The solution must balance simplicity with architectural rigor, avoiding premature optimization while maintaining testability and clear layer boundaries.

## Decision
We will implement a Todo REST API using Axum web framework with:
1. Health check endpoint at `/health`
2. RESTful Todo resource endpoints (`GET/POST/PUT/DELETE /todos`)
3. In-memory repository implementation
4. Clear separation between:
   - Domain: `Todo` struct with validation
   - Ports: `TodoRepository` trait
   - Adapters: Axum handlers (primary), in-memory repo (secondary)

Axum routes will delegate to use cases through a `ApiContext` wrapper maintaining hex tier boundaries. All business logic resides in domain layer with zero web framework dependencies.

## Consequences

### Positive
- Clear demonstration of hexagonal architecture patterns
- Decoupled infrastructure from business logic
- Easy replacement of Axum with another web framework
- In-memory repo enables testing without databases

### Negative
- Initial boilerplate for proper layer separation
- Manual mapping between HTTP models and domain entities
- No persistence out-of-the-box

### Neutral
- Axum handler signatures will mirror domain use cases
- Health check endpoint follows existing project conventions

## Implementation

### Phases
1. **Domain Core (Tier 0-1)**
   - Implement `Todo` domain model with validation
   - Define `TodoRepository` port trait

2. **Adapters & Composition (Tier 3-5)**
   - Axum route handlers with request/response DTOs
   - In-memory repository implementation
   - Wire components in composition root

### Affected Layers
- [x] domain/ (Todo model)
- [x] ports/ (TodoRepository)
- [x] adapters/primary/ (Axum handlers)
- [x] adapters/secondary/ (InMemoryTodoRepo)
- [ ] usecases/
- [x] composition-root (router setup)

### Migration Notes
None required - initial implementation. In-memory storage means no data persistence between runs by design.