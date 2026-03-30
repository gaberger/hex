```markdown
# ADR-2310311230: Implement URL Shortener REST API in Go

## Status
proposed

## Date
2023-10-31

## Drivers
- User need for a lightweight, single-binary URL shortener tool in Go.

## Context
The project requires the implementation of a URL shortener REST API, which is expected to handle creating and retrieving shortened URLs. Given the nature of the feature, it should be designed as a single-binary Go application, leveraging an in-memory storage solution for simplicity in deployment and testing. This implementation aligns with the overall architectural direction of using a hexagonal architecture (as established in ADR-001), ensuring separation of concerns and independence between the core business logic and the external interfaces.

In-memory storage is suitable for the initial development phase, permitting rapid iteration and simplifying the setup, although it does introduce trade-offs regarding data persistence. As there are no existing ADRs addressing in-memory storage for REST APIs specifically, this decision stands independently while adhering to the architectural guidelines of the project.

## Decision
We will implement the URL shortener REST API in `main.go` of the single-binary Go project, utilizing an in-memory storage system to manage shortened URLs. This implementation will adhere to hexagonal architecture principles, where the core domain logic will be separated from both the REST API interfaces (`adapters/primary`) and data management interfaces (`adapters/secondary`). We will structure the project into domain models and service layers, isolating core business logic from external concerns while enabling easier testing and future scalability.

## Consequences

### Positive
- Rapid development and deployment due to the simplicity of a single-binary application.
- Improved testability and maintenance by following hexagonal architecture, which promotes clean separation between layers.

### Negative
- In-memory storage lacks durability; any application restart or crash could lead to data loss.
- Potentially increased complexity in future scalability if transitioning to persistent storage in response to growing demands.

### Neutral
- The choice of an in-memory solution is beneficial for initial prototypes, with flexibility to adapt to persistent storage solutions later, if necessary.

## Implementation

### Phases
1. **Phase 1** — Implement core logic for URL shortening and retrieval within the domain layer and primary adapter for the REST API.
2. **Phase 2** — Integrate in-memory storage in the secondary adapter, ensuring linkage and data handling for the previously established routes.

### Affected Layers
- [X] domain/
- [X] ports/
- [X] adapters/primary/
- [X] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None
```