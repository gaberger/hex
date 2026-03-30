```markdown
# ADR-2310201020: Implement URL Shortener REST API in Go

## Status
proposed

## Date
2023-10-23

## Drivers
- User requirement for a URL shortener service that is simple, efficient, and utilizes in-memory storage for quick access.

## Context
We are tasked with building a REST API for a URL shortener service using Go, which is a single-binary project. Due to the nature of this service, it will need to handle URL shortening and retrieval functionalities. Utilizing in-memory storage is essential for performance, given that URL shorteners require fast read and write operations.

Implementing this feature within the bounds of **hexagonal architecture** (adr-001) demands careful attention to dependency direction, ensuring that our domain logic remains independent of the frameworks and infrastructure. The proposed solution will articulate how to structure this service using domain, ports, and adapters in alignment with our established architecture principles.

## Decision
We will implement the URL shortener REST API in `main.go`, using the hexagonal architecture pattern. This will involve creating a domain model that defines the core business logic for URL shortening. The adapters will handle the REST API interface, and in-memory storage will serve as the data repository.

The implementation will consist of defining the necessary ports and adapters to separate the application logic from the external connections, facilitating easier testing and maintenance. The REST API will reside in the primary adapter layer, while in-memory storage operations will be executed through the secondary adapter layer, ensuring adherence to our architectural goals.

## Consequences

### Positive
- The application will be straightforward to deploy as a single binary, improving our deployment simplicity.
- Utilizing in-memory storage will maximize performance due to reduced latency in read/write operations.

### Negative
- In-memory storage may limit persistence and scalability, as data will not be retained across application restarts.
- This initial implementation may require future adjustments to accommodate persistent storage solutions if scalability becomes an issue.

### Neutral
- The decision allows for quick prototyping and testing but may need refinements if user demand increases.

## Implementation

### Phases
1. Phase 1: Build the domain model and basic URL shortening logic, including unit tests (hex tier 3).
2. Phase 2: Develop the REST API interface and connect it with the in-memory storage for data retrieval and storage (hex tier 4).

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