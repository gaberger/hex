```markdown
# ADR-2310231200: Implement URL Shortener REST API in Go

## Status
proposed

## Date
2023-10-23

## Drivers
- User need for a functional URL shortener service.
- Requirement for a straightforward, single-binary deployment.

## Context
The goal is to implement a URL shortener REST API in Go, utilizing in-memory storage for simplicity and quick access. This REST API will serve clients by accepting URL submission requests and generating short links as responses. The decision is being made to keep the implementation within a single binary executable, which aligns with the project's goal of simplicity and ease of deployment as outlined in previous ADRs (e.g., ADR-001, where hexagonal architecture is established as the foundational pattern).

Using in-memory storage permits rapid development and testing phases, while initial performance concerns will guide future enhancements. The chosen design must comply with hexagonal architecture principles, ensuring clear separation of concerns across layers and promoting maintainability.

## Decision
We will implement the URL shortener REST API as a single-binary Go application using in-memory storage for the URL mappings. The core components will be developed across the appropriate hexagonal architecture layers: 

- The **domain layer** will encapsulate the business logic for managing URLs.
- The **usecases layer** will define the workflow of shortening and retrieving URLs.
- The **adapters/primary layer** will handle the HTTP requests and translate them into use case calls.

The implementation will start from `main.go`, orchestrating all necessary components and functions.

## Consequences

### Positive
- Rapid initial development and deployment due to in-memory storage.
- Simplifies the architecture by maintaining a single binary application.

### Negative
- In-memory storage will limit persistence and durability; data will not be retained between application restarts.
- Potential for increased complexity in future enhancements, as stronger storage solutions may need to be added later.

### Neutral
- Simplified API surface during initial development that may evolve with user feedback and use.

## Implementation

### Phases
1. Phase 1: Develop the core functionalities, including URL shortening and redirection, within `main.go`.
2. Phase 2: Add API endpoints and testing ensuring all functionality works correctly and can be accessed via HTTP requests.

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