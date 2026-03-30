```markdown
# ADR-2303191200: Implement URL Shortener REST API in Go

## Status
proposed

## Date
2023-03-19

## Drivers
- User need for a URL shortener REST API with simple implementation.
- Requirement for a single-binary Go project with in-memory storage for simplicity and performance.

## Context
The goal is to develop a URL shortener API with a focus on simplicity and efficiency, using Go as the primary programming language. The project aims to implement the complete feature set directly in `main.go`, following hexagonal architecture principles as established in ADR-001. Given that the API will rely on in-memory storage, we will prioritize speed and ease of deployment in our design decisions. 

This URL shortener must accommodate requests to shorten URLs, retrieve original URLs based on the shortened version, and maintain a simple RESTful interface. In-memory storage is chosen to simplify the implementation for an initial proof-of-concept, although this may limit persistence capabilities in the future.

## Decision
We will implement the URL shortener REST API in a single binary Go project, contained entirely within `main.go`. This implementation will reside within the adapters layer as it directly handles incoming HTTP requests and responses. The URL shortening logic will be encapsulated in domain entities and use cases, ensuring a clean separation of concerns and adherence to hexagonal architecture principles.

The project's structure will support requests to create shortened URLs and retrieve them, with the functionality being built in phases: first establishing the HTTP interface, followed by the integration of domain logic and use case management.

## Consequences

### Positive
- Simplifies the development process by focusing all code in a single binary.
- Fast speed and low overhead using in-memory storage for URL management.
- Complies with hexagonal architecture principles, ensuring modularity for future expansions.

### Negative
- Limited durability and persistence of data due to in-memory storage.
- Potential challenges in scaling to handle a larger number of requests without a more robust storage solution.

### Neutral
- The single binary approach may limit separation of concerns, but it provides an opportunity for rapid prototyping.

## Implementation

### Phases
1. Implement the HTTP server interface and basic request handlers for short URL creation and retrieval.
2. Integrate domain logic for managing URL shortening and expand the functionality as necessary.

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