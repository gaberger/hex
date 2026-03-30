```markdown
# ADR-2304261200: Implement URL Shortener REST API in Go

## Status
proposed

## Date
2023-04-26

## Drivers
- User need for a simple and performant URL shortener that operates as a single binary application.

## Context
The project aims to build a friendly URL shortener REST API in Go, leveraging in-memory storage for simplicity and performance. Implementing this feature will serve various use cases, including quick URL redirection and logging for analytics. The decision to store data in memory is driven by the need for low latency and ease of deployment, as the URL shortener can be run as a single-binary Go application specified in `main.go`. This is in alignment with existing architectural patterns defined in previous ADRs—particularly ADR-001 regarding hexagonal architecture as the foundational pattern.

By adhering to the hexagonal architecture, we can develop the API such that it remains modular and testable, allowing for potential future extensions—such as transitioning to a persistent storage solution—without major refactoring. The anticipated complexity of managing state in memory, alongside handling concurrent requests, must also be taken into consideration.

## Decision
We will implement a URL shortener REST API in Go, designed as a single binary application, with in-memory storage for URL mappings. The implementation will primarily affect the following hexagonal architecture layers: 
- **Domain Layer**: Definition of core entities like `URL` and domain logic for shortening and expanding URLs.
- **Ports**: Interfaces for incoming requests and command responses.
- **Adapters**: 
  - **Primary Adapter**: An HTTP server handler that processes incoming REST API calls.
  - **Secondary Adapter**: In-memory storage solution that serves as a backend for storing URL mappings.

The feature will be developed in the `main.go` file to keep the entire application self-contained and straightforward to execute.

## Consequences

### Positive
- Quick development and deployment due to single binary architecture and in-memory storage.
- Increased performance with lower latency for URL lookups and creations.

### Negative
- In-memory storage means data will be lost if the application is restarted, implying no persistence.
- Scalability issues may arise if the application is required to handle large datasets or concurrent requests in the future.

### Neutral
- The choice of using REST API leverages widely adopted practices that are familiar to developers and users.

## Implementation

### Phases
1. **Phase 1**: Build the core functionality in `main.go`, including the REST API endpoints for shortening and retrieving URLs, as well as the in-memory storage mechanism. This corresponds to Tiers 1 and 2 (HTTP and domain logic).
2. **Phase 2**: Implement unit tests and integration tests to ensure the functionality works as intended, followed by addressing any performance considerations that emerge. This corresponds to Tier 3 (application logic).

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