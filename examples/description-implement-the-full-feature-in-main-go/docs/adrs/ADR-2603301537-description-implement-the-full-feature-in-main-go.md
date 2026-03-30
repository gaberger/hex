```markdown
# ADR-2309281520: Implement URL Shortener REST API with In-Memory Storage

## Status
proposed

## Date
2023-09-28

## Drivers
- User need for a functional URL shortener as a REST API.
- Simplification of development through a single-binary Go project.

## Context
The project requires the implementation of a URL shortener REST API using Go with in-memory storage. This decision will leverage the flexibility of hexagonal architecture as outlined in ADR-001, ensuring clean separation of concerns between the business logic, ports, and adapters. The need for a single-binary application facilitates easier deployment and testing, aligning with the simplicity goals of this feature.

The use of in-memory storage will expedite development and simplify data management within the early stages. However, it also brings considerations for data persistence, which may require future enhancements. This initial decision focuses on delivering the minimal viable product (MVP) while adhering to hexagonal architecture principles, as introduced in previous ADRs.

## Decision
We will implement a URL shortener REST API in the main.go file using a single-binary Go project with in-memory storage for URL mapping. The implementation will involve the following layers: 
- **Domain Layer**: Define the core business logic and data structures related to URLs.
- **Ports Layer**: Define interfaces for the service and repository that will allow interaction between the domain and adapters.
- **Adapters Layer**: Implement the primary adapter (HTTP handler) to expose RESTful endpoints and a secondary adapter for in-memory repository.

The architecture will maintain separation between these layers, promoting ease of testing and potential future replaceability with more robust storage solutions.

## Consequences

### Positive
- Rapid development and iteration capability due to simplicity of in-memory storage.
- Clear structure and maintainability through hexagonal architecture.

### Negative
- Lack of persistence, leading to data loss on application restart unless future enhancements are made for persistent storage.
- Limited scalability and reliability of in-memory storage for production scenarios.

### Neutral
- The single-binary nature of the Go project lends itself to easier distribution but may lead to challenges in modularity as the project scales.

## Implementation

### Phases
1. **Phase 1** — Develop the core business logic and in-memory repository for URL mapping (Domain and Ports layers).
2. **Phase 2** — Implement the REST API endpoints using an HTTP router and wire them to the domain logic (Adapters layer).

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