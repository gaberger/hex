```markdown
# ADR-2310181530: Implement URL Shortener REST API as Full Feature in main.go

## Status
proposed

## Date
2023-10-18

## Drivers
- User need for a simple and efficient URL shortener service
- Desire to encapsulate all functionality within a single binary Go application

## Context
The project aims to develop a URL shortener REST API implemented in Go, utilizing an in-memory storage solution for simplicity and performance. This feature addresses a common requirement in modern web applications—providing shortened URLs while maintaining ease of use and integration. The choice of a single-binary Go project is driven by its advantages in deployment and operational simplicity. Given the architecture established by previous ADRs, particularly ADR-001, which enforces the hexagonal architecture pattern, it is imperative to ensure that each layer (domain, ports, adapters) is respected and organized correctly.

Integrating this feature requires attention to the REST API interface design, data handling, and the in-memory storage mechanism. We must ensure that layer dependencies are followed while producing a clean, maintainable code base that allows for future enhancements or changes in storage options if needed.

## Decision
We will implement the full URL shortener feature within the `main.go` file of the Go project as outlined in the user requirements. This will include the domain model representing URLs, a use case for transforming long URLs to shortened forms, and the primary adapter facilitating the REST API interactions. The temporary in-memory storage will be built within the adapters layer to handle URL data efficiently. 

This implementation will primarily affect the domain, ports, adapters/primary, and usecases layers. The purpose of the adapters is to expose the API without leaking business logic, while the usecases will encapsulate the core functionality required for URL shortening.

## Consequences

### Positive
- Simplifies deployment and integration by delivering a single binary application.
- Eliminates the need for external storage dependencies by using in-memory storage for quick development and testing.

### Negative
- In-memory storage sacrifices durability and persistence; data will be lost upon application restarts.
- Future scalability may require re-architecting for persistent storage solutions, adding complexity.

### Neutral
- The use of a single main.go may limit the initial modularity of the codebase but is manageable for a prototype implementation.

## Implementation

### Phases
1. **Phase 1** — Develop the core domain model and the primary REST API endpoints in `main.go`.
2. **Phase 2** — Implement in-memory storage mechanisms and complete the use cases for URL handling.

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