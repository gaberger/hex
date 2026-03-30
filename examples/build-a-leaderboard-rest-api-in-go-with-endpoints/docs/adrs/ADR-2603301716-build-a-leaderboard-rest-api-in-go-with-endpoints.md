```markdown
# ADR-2310231230: Leaderboard REST API for Score Submission and Rankings

## Status
proposed

## Date
2023-10-23

## Drivers
- User need for a leaderboard system that allows for submitting scores and fetching rankings in real-time.

## Context
We are tasked with building a leaderboard REST API that will handle score submissions and return rankings. This API needs to be efficient and reliable, serving as an interface for clients to interact with the leaderboard data. Given the requirements, the API will need to accommodate simultaneous score submissions and retrieval requests, necessitating considerations for performance, data consistency, and scalability. Following the principles of hexagonal architecture, we aim to ensure that our application is adaptable to different use cases while keeping the core domain logic isolated.

Existing ADRs (like ADR-001: Hexagonal Architecture as Foundational Pattern) have established the foundation of our architecture, promoting a clear separation of concerns. The architecture should enable us to implement the necessary business logic within the domain layer while exposing the required interactions through ports and adapters.

## Decision
We will implement a leaderboard REST API in Go with two primary endpoints: one for submitting scores and another for fetching rankings. The implementation will be structured around hexagonal architecture, particularly focusing on the domain layer for core business logic, usecases for orchestrating input/output operations, and adapters for interfacing with external requests. 

The score submission endpoint will handle incoming POST requests to record scores, while the rankings endpoint will manage GET requests to retrieve the current leaderboard standings. We will ensure that the domain layer encapsulates the rules for score management and rankings calculation, while the ports will define the input and output contracts. Adapters will be implemented to convert incoming HTTP requests into domain commands and to format domain responses back into HTTP responses.

## Consequences

### Positive
- Clearly defined API endpoints for easy client integration.
- Scalability and maintainability through the hexagonal architecture approach.

### Negative
- Initial development overhead for creating and testing the necessary layers and interfaces.
- Potential complexity in coordinating behavior between different layers for first-time implementations.

### Neutral
- The decision to utilize Go as the foundation may impact performance, depending on the implementation of concurrency patterns.

## Implementation

### Phases
1. **Phase 1** — Implement the domain logic for score management and rankings calculation (Hex layer 2).
2. **Phase 2** — Develop the RESTful API endpoints with appropriate adapters for handling HTTP requests (Hex layer 3).

### Affected Layers
- [x] domain/
- [x] ports/
- [x] adapters/primary/
- [ ] adapters/secondary/
- [x] usecases/
- [x] composition-root

### Migration Notes
None
```