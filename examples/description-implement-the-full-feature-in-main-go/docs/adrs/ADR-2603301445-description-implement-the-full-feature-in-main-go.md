```markdown
# ADR-2310251500: Implement URL Shortener REST API in Go

## Status
proposed

## Date
2023-10-25

## Drivers
- User need for a simple, efficient URL shortener service without external dependencies.

## Context
The project requires a straightforward URL shortening solution implemented in a single-binary Go application. Given the existing architectural guidelines, primarily focusing on hexagonal architecture, the solution will need to be structured in a way that adheres to the principles of clean separation of concerns among different layers of the application. The absence of persistent storage solutions dictates the necessity to employ in-memory data structures for storage. With the aim of delivering a scalable REST API that handles URL shortening requests, there’s a need to define clear interfaces that promote flexibility through adapters.

The use of Go, with its strengths in concurrency and performance, aligns well with the project needs. This feature must be implemented while ensuring that it adheres to the guidelines established in existing ADRs, particularly ADR-001 regarding hexagonal architecture and the separation of domain logic from infrastructure details. It is essential to maintain the simplicity and maintainability of the codebase, given that it will function as a single binary.

## Decision
We will implement a REST API for a URL shortener in a Go application structured according to the hexagonal architecture. The API will expose endpoints for creating short URLs, retrieving original URLs, and listing all stored URLs. In-memory storage will be utilized to handle the URLs efficiently during runtime, which aligns with the requirement for a single-binary deployment.

The implementation will consist of a domain layer responsible for the core business logic, a ports layer defining the interfaces for the API interactions, and an adapters layer implementing these interfaces for the HTTP transport. The in-memory storage will function as a secondary adapter, ensuring that domain logic remains agnostic of the underlying data storage mechanism.

## Consequences

### Positive
- Simplifies the deployment process to a single binary, reducing operational complexity.
- Leverages Go's performance for a responsive API that can handle multiple requests simultaneously.

### Negative
- In-memory storage might not be suitable for production use due to data loss on application restart.
- Limited scalability as data persists only in RAM, affecting persistence requirements in case of a stateful application.

### Neutral
- The implementation being solely in-memory may facilitate rapid development but raises considerations for future scalability.

## Implementation

### Phases
1. Implement the domain logic and define ports for the URL shortener service (hex layer 2).
2. Develop the HTTP handler adapters that implement the defined interfaces and utilize in-memory storage (hex layers 3-4).

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