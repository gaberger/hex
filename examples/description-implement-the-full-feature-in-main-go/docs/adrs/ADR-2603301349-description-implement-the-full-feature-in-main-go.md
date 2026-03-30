```markdown
# ADR-2309281000: Implement URL Shortener REST API in Go with In-Memory Storage

## Status
proposed

## Date
2023-09-28

## Drivers
- User need for a URL shortener feature with a RESTful interface.
- Desire for a simple and efficient implementation using in-memory storage.

## Context
The current project is aiming to build a URL shortener service that will allow users to create short links which redirect to long URLs. The decision to implement this feature as a REST API aligns with the growing need for accessible web services that provide essential functionalities via HTTP. Storing data in-memory will ensure fast access and simplicity, while the single-binary Go project structure simplifies deployment and reduces operational overhead.

This decision builds on the existing hexagonal architecture (ADR-001), which mandates strict boundaries between layers. The feature will employ the use cases defined in the domain layer to ensure business logic encapsulation, while the adapters will facilitate interaction with external clients. An in-memory storage mechanism has been selected for this implementation to streamline the development process and support rapid testing without persistent storage complexities.

## Decision
We will implement a URL shortener REST API in Go, utilizing in-memory storage for URL mapping. The API will be designed within the adapters/primary layer, specifically exposing endpoints for creating and retrieving short links. The business logic will reside within the domain and usecases layers, ensuring that URL shortening and retrieval adhere to the defined domain models. 

The architecture encapsulation will focus on maintaining clear separation of concerns, with the primary adapter acting as the interface for HTTP requests and responses while leveraging domain services to handle the core functionality. The decision also includes transitioning all configuration and routing through the composition-root layer to maintain modularity.

## Consequences

### Positive
- Fast development and testing cycle due to in-memory data management.
- Clear separation of concerns enhancing maintainability and scalability.

### Negative
- The in-memory storage means data will be lost on application restarts, necessitating future work for persistent storage.
- Limitations in scalability and concurrency management due to single-binary constraints.

### Neutral
- The REST API structure can be adapted for future enhancements such as persistent storage solutions without significant refactoring of the core architecture.

## Implementation

### Phases
1. **Phase 1:** Build the basic REST API endpoints (create, retrieve short link) within adapters/primary.
2. **Phase 2:** Implement in-memory storage logic and integrate it with the domain layer for URL mapping.

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