```markdown
# ADR-2309251030: Implement URL Shortener REST API in Go with In-Memory Storage

## Status
proposed

## Date
2023-09-25

## Drivers
- User need for a functional URL shortener service in a single-binary Go project.

## Context
The need arises to build a URL shortener REST API using Go that can efficiently manage URL mappings. This service will operate with in-memory storage, enabling quick access and manipulation of URL data. The chosen approach aligns with the principles of hexagonal architecture, allowing for clear separation between the application's core logic and external interfaces.

Implementing the URL shortener within a single-binary solution simplifies deployment and distribution, catering to user requirements for ease of use and maintenance. This decision supports the overall project architecture as defined in ADR-001, ensuring that the application will maintain its structural integrity while meeting functional requirements.

## Decision
We will implement the full URL shortener feature in `main.go`, utilizing Go’s capabilities for creating a REST API. The implementation will encompass the core domain logic, including URL shortening and retrieval, while storing data in-memory for rapid access. The following hexagonal architecture layers will be involved: 

- **Domain Layer:** Defines the core business logic related to URLs.
- **Ports:** Interfaces that the API will expose for URL management.
- **Adapters (Primary):** Responsible for handling HTTP requests and mapping them to domain operations.

The first phase will focus on establishing the domain and ports, ensuring that core functionality is encapsulated and accessible. The second phase will integrate the HTTP adapters to facilitate user interactions.

## Consequences

### Positive
- Rapid development and deployment of a minimal yet functional URL shortener.
- In-memory storage allows for high performance and low latency in URL handling.

### Negative
- In-memory storage limits persistence; data will be lost on service restart.
- May require additional implementation in the future to handle scalability and durability.

### Neutral
- Development solely in `main.go` may lead to larger files as functionality expands, though it aligns with a quick prototyping approach.

## Implementation

### Phases
1. Establish domain and ports — define URL structure and API interfaces.
2. Implement HTTP adapters to connect user requests to domain functionalities.

### Affected Layers
- [x] domain/
- [x] ports/
- [x] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None
```