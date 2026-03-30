```markdown
# ADR-2310030956: Implement URL Shortener REST API in Go with In-Memory Storage

## Status
proposed

## Date
2023-10-03

## Drivers
- User need for a URL shortener feature
- Requirement for a single-binary Go project architecture

## Context
The project currently aims to implement a URL shortener as a REST API using the Go programming language with in-memory storage. This feature will allow for the creation, retrieval, and management of shortened URLs. Since the project follows a hexagonal architecture, it is crucial that we appropriately define the various elements of the architecture to maintain boundaries and ensure clean separation of concerns.

The implementation will take place in the `main.go` file, which will serve as the entry point for our application. Given that the feature specifications entail in-memory storage, it is essential to consider performance and scalability concerns as the number of shortened URLs grows, although in-memory storage can suffice for the initial version.

## Decision
We will implement the URL shortener REST API as a single binary Go project in `main.go` using the hexagonal architecture. This will involve defining the necessary domain models for the URLs, ports for interaction (both for creating and retrieving shortened URLs), and primary adapters to handle HTTP requests. The in-memory storage will be abstracted behind a secondary adapter to ensure the domain remains independent of the storage implementation.

Initial implementation phases will focus on ensuring a basic HTTP interface for URL creation and retrieval, with the domain, ports, and use cases strictly defined to prevent coupling with specific adapter implementations.

## Consequences

### Positive
- Clean separation of architecture layers enhances maintainability and testability.
- Single binary Go project simplifies deployment and execution.

### Negative
- In-memory storage limits the persistence of data; restarting the service will lose all URLs.
- Potential performance bottlenecks as the quantity of stored URLs increases over time.

### Neutral
- The solution enables rapid development and iteration of features without complex setup.

## Implementation

### Phases
1. **Phase 1:** Implement basic HTTP handlers to process URL creation and retrieval (maps to layers 3-5 of the architecture).
2. **Phase 2:** Develop in-memory storage adapter and connect it to the domain model (maps to layers 2-3).

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