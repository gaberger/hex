```markdown
# ADR-2409231230: Implement URL Shortener REST API in Go

## Status
proposed

## Date
2023-09-24

## Drivers
- User requirement for a URL shortener service
- Need for a simple, in-memory solution to prototype quickly
- Compliance with hexagonal architecture principles

## Context
The goal of this decision is to implement a URL shortener REST API in Go as a single-binary project utilizing in-memory storage. This feature aligns with the principles outlined in ADR-001 regarding hexagonal architecture as a foundational pattern. The REST API will serve as the primary interface for clients to create, retrieve, and manage shortened URLs. 

The use of in-memory storage will provide a speedy and lightweight solution suitable for first iterations, enabling rapid development and testing without the overhead of persistent storage systems. A clear separation among domain, ports, and adapters is paramount to maintain the integrity of the hexagonal architecture specified in existing ADRs.

## Decision
We will implement a URL shortener REST API in the `main.go` file of our Go project. This implementation will consist of the following components:

- **Domain Layer**: Define the core logic for URL shortening and data modeling.
- **Ports**: Specify interfaces that the application will expose for URL management.
- **Adapters/Primary**: Implement HTTP handlers to translate incoming requests into internal API calls.
- **Adapters/Secondary**: Handle in-memory storage for shortened URLs, ensuring they conform to the domain models.

This structured approach will ensure clear boundaries and maintainability in line with architectural best practices.

## Consequences

### Positive
- Rapid development and ease of testing through in-memory storage.
- Clear separation of concerns fostering maintainability and adaptability.
- Aligns well with existing architecture leveraging hexagonal principles.

### Negative
- In-memory storage will not persist data across application restarts, limiting usability for production.
- Increased complexity in switching to a persistent storage solution later.

### Neutral
- Performance limitations of in-memory storage, which may affect scalability under heavy load.

## Implementation

### Phases
1. **Phase 1**: Implement the core domain logic and in-memory storage adapter.
2. **Phase 2**: Develop the REST API endpoints and port definitions.

### Affected Layers
- [x] domain/
- [x] ports/
- [x] adapters/primary/
- [x] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None
```