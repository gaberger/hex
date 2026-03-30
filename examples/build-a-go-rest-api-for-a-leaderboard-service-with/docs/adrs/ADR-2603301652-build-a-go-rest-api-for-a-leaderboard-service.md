```markdown
# ADR-2404261455: Leaderboard Service with Go API & TypeScript CLI

## Status
proposed

## Date
2024-04-26

## Drivers
- Need for a scalable and reliable leaderboard service.
- Requirement for a user-friendly CLI client to interact with the service.
- Desire to maintain consistency with existing hexagonal architecture.

## Context
The project requires a leaderboard service to track and rank user scores. This service will be accessed through a REST API built in Go, adhering to the project's existing hexagonal architecture (ADR-001). A TypeScript CLI client will provide a convenient interface for users to submit scores and view rankings. The choice of Go aligns with existing project languages (ADR-003), and the CLI client in TypeScript allows for cross-platform compatibility and maintainability.

## Decision
We will implement a leaderboard service with a Go REST API and a TypeScript CLI client, ensuring adherence to the hexagonal architecture principles. The Go service will expose endpoints for submitting scores and retrieving leaderboard rankings. The TypeScript CLI will provide commands for interacting with these endpoints. We will use a database adapter (likely PostgreSQL) to persist leaderboard data.

## Consequences

### Positive
- Provides a scalable and reliable leaderboard service.
- Offers a user-friendly CLI for interacting with the service.
- Aligns with the project's existing architecture and technology stack.
- Clear separation of concerns between the API and the CLI client.

### Negative
- Requires development and maintenance of both a Go service and a TypeScript CLI.
- Introduces potential challenges in maintaining consistency between the API and the CLI.
- Dependency on external database systems could impact service availability.

### Neutral
- Performance will vary depending on database choice and optimization.

## Implementation

### Phases
1. **Phase 1: Go REST API Implementation (Tiers 2-4).** Implement the core leaderboard logic in the domain layer, define ports for accessing leaderboard data and exposing API endpoints, and build adapters for database persistence and HTTP handling.
2. **Phase 2: TypeScript CLI Client Implementation (Tier 5).** Create the CLI client with commands for score submission and leaderboard retrieval, utilizing the API endpoints exposed by the Go service.
3. **Phase 3: Database Integration (Tier 3).** Implement the database adapter to connect the leaderboard service to a persistent database (e.g., PostgreSQL).

### Affected Layers
- [x] domain/
- [x] ports/
- [x] adapters/primary/
- [x] adapters/secondary/
- [x] usecases/
- [x] composition-root

### Migration Notes
None
```