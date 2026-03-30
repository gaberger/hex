```markdown
# ADR-2310161015: Implement URL Shortener REST API in Go

## Status
proposed

## Date
2023-10-16

## Drivers
- User need for a URL shortener feature
- Need for quick prototyping using in-memory storage

## Context
The project requires the implementation of a URL shortener REST API using Go as part of the overall architecture defined in previous ADRs, particularly following the hexagonal architecture pattern outlined in ADR-001. The decision to use a single-binary Go project aligns with the need for simplicity and ease of deployment, adhering to the constraints of developing and deploying a small application quickly.

In-memory storage was chosen as a tradeoff to expedite development and testing, allowing us to focus on functionality without the overhead of persistent storage solutions. The limitations of in-memory storage in terms of data persistence and scalability are noted but acceptable given the current scope of this feature. This approach serves well for initial phases of the product, ensuring that we can quickly assess the effectiveness of the URL shortening feature.

## Decision
We will implement a URL shortener REST API in a single-binary Go project, leveraging in-memory storage for this feature. This implementation will be structured within the hexagonal architecture, where the core business logic will reside in the domain layer, the REST API will be defined in the primary adapter layer, and use cases will orchestrate the interactions between the domain and the adapters. 

The initial phase will involve creating a domain model that represents URLs, their shortened forms, and the associated logic for handling URL shortening and retrieval. Subsequent phases will focus on defining the port interfaces and implementing the REST endpoints, followed by integrating these endpoints with the in-memory storage solution.

## Consequences

### Positive
- Rapid development cycle due to single-binary deployment and in-memory storage
- Clear separation of concerns through hexagonal architecture facilitates maintainability

### Negative
- In-memory storage implies data will be lost upon application restart
- Limited scalability for heavy usage scenarios

### Neutral
- The implementation may serve as a prototype to validate the core functionality before considering persistent storage solutions.

## Implementation

### Phases
1. Phase 1 — Develop the domain model and the essential logic for URL shortening
2. Phase 2 — Define the REST API in the primary adapter layer and connect it to the in-memory storage

### Affected Layers
- [x] domain/
- [x] ports/
- [x] adapters/primary/
- [ ] adapters/secondary/
- [x] usecases/
- [ ] composition-root

### Migration Notes
None
```