```markdown
# ADR-2311081530: Implementing a URL Shortener REST API with In-Memory Storage

## Status
proposed

## Date
2023-11-08

## Drivers
- User need for a simple and efficient URL shortening service.
- Quick feedback loop for users without persistent storage complexities.
- Desiring a high-performance solution leveraging Go.

## Context
The project requires a URL shortener service that allows users to create short links from long URLs quickly and efficiently. Given the focus on rapid development and deployment, an in-memory storage mechanism is proposed. This decision aligns with the existing architectural principles outlined in ADR-001, which speaks to maintaining a clear separation between layers through hexagonal architecture.

Building this API will require understanding how the domain model will represent URLs, how ports and adapters will facilitate HTTP requests and responses, and how use cases will orchestrate the interaction between these components. The selected technology (Go) will enable efficient performance and concurrency, which is essential given the expected usage patterns.

## Decision
We will implement a URL shortening REST API using Go that relies on in-memory storage for URL mappings. The domain layer will define the entities and logic pertaining to URLs and their shortened counterparts. The ports layer will define interfaces for the services that will handle incoming API requests (like creating and fetching URLs). The adapters layer will include HTTP handlers for handling RESTful requests and responses. This architecture will ensure adherence to hexagonal principles while meeting the project's goals for rapid deployment.

## Consequences

### Positive
- Fast response times due to in-memory storage, suitable for initial prototyping and development.
- Simplified architecture as there's no requirement for persistent storage at this stage.
- Easy to modify or replace the in-memory storage implementation in the future for scaling needs.

### Negative
- Data will be lost upon application shutdown or restart, which may not be suitable for production use without further enhancements.
- Scaling might require refactoring the storage layer if the application transitions to a more robust architecture later.

### Neutral
- The in-memory approach allows for simple testing and validation but may necessitate additional work to introduce persistence later.

## Implementation

### Phases
1. Develop the domain model and the in-memory storage implementation (Hex Layer 1 - Domain).
2. Implement the ports and adapters to expose the API (Hex Layers 2-5 - Ports/Adapters).

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