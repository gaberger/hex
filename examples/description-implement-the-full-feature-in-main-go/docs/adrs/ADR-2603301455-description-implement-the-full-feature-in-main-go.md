```markdown
# ADR-2310121500: Implement URL Shortener REST API as a Single-Binary Go Project

## Status
proposed

## Date
2023-10-12

## Drivers
- User need for a lightweight, in-memory URL shortener service.
- Desire for a single-binary deployment for ease of distribution and execution.

## Context
The evolution of our project requires a lightweight solution for a URL shortener that can easily be implemented and deployed. This aligns with our architecture principles and is driven primarily by a user need for functionality that is both efficient and straightforward. According to existing ADR-001, we have chosen hexagonal architecture as our foundational pattern, ensuring separations of concerns and adaptability to future changes.

The decision to adopt a REST API design allows for easier integration and usage by clients, while the in-memory storage meets initial performance and simplicity requirements, making it a suitable starting point for development. Given the constraints of a single-binary deployment as described in our requirements, the implementation will need to ensure minimal dependency footprint while being maintainable.

## Decision
We will implement a URL shortener REST API in Go, encapsulated within a single binary executable, located in `main.go`. The design will employ hexagonal architecture principles, with a focus on domain and ports layers first, followed by adapters. The API will interact with an in-memory storage layer to manage URLs efficiently.

This implementation will include defining the domain model for URL entities, establishing the necessary ports for interaction between the domain and adapters, and developing primary adapters to handle HTTP requests and responses.

## Consequences

### Positive
- Simplifies deployment as a single executable file, reducing operational overhead.
- In-memory storage provides fast access but will require persistence consideration in later iterations.

### Negative
- Limitations in scaling due to in-memory storage, necessitating future improvements for data persistence.
- Potential lack of persistence mechanisms may pose challenges for handling data beyond application restarts.

### Neutral
- The initial implementation focuses solely on core URL shortening functionalities, leaving room for future enhancements.

## Implementation

### Phases
1. **Phase 1** — Build the domain model and necessary ports, establish HTTP server in `main.go`.
2. **Phase 2** — Develop adapters for HTTP handling and implement in-memory storage for URL management.

### Affected Layers
- [x] domain/
- [x] ports/
- [x] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [x] composition-root

### Migration Notes
None
```