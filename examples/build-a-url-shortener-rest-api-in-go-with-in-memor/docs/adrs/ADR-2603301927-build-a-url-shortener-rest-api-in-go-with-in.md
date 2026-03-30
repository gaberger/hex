```markdown
# ADR-2311131230: Build URL Shortener REST API in Go with In-Memory Storage

## Status
proposed

## Date
2023-11-13

## Drivers
- User need for a simple URL shortening service.
- Quick implementation with in-memory storage to facilitate rapid development and testing.

## Context
The project requires a straightforward URL shortener REST API that can efficiently manage URL mappings and serve shortened links to users. Given the immediate requirements, utilizing in-memory storage will allow for swift implementation and testing, catering to the initial user demand without the complexity of persistent storage. This approach aligns with existing ADRs in the project that emphasize an in-memory architecture, such as ADR-2603301813 and ADR-2603301820 for REST APIs.

However, using in-memory storage also introduces limitations, such as data loss on application restarts and no easy scaling options. Therefore, it's crucial to outline these limitations clearly and plan the architecture to allow for potential future extensions, such as persisting URLs in a database.

## Decision
We will build a URL shortener REST API using Go, featuring in-memory storage to manage the mappings of shortened URLs to original URLs. The implementation will adhere to the hexagonal architecture principle by defining a clear domain model for URL mapping, using ports for interaction and adapters to expose the REST API. 

The API will support endpoints for creating, retrieving, and deleting shortened URLs, with use cases defining the business logic required for these operations. In this way, we will ensure that the architecture promotes separation of concerns between the different layers, allowing for future adaptability, such as integrating persistent storage or additional features.

## Consequences

### Positive
- Rapid development with in-memory storage simplifies the initial implementation.
- Clear separation of concerns supports maintainability and scalability for future changes.

### Negative
- In-memory data storage means potential data loss on application restarts.
- Limited scalability without the ability to persist data in a database.

### Neutral
- Implementation in Go provides strong performance characteristics benefiting the API responsiveness.

## Implementation

### Phases
1. **Phase 1** — Develop the core domain model and basic CRUD operations for the URL mappings, utilizing in-memory storage (mapping URLs).
2. **Phase 2** — Implement REST API endpoints as primary adapters interfacing with the core use cases and domain model.

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