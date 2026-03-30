```markdown
# ADR-2310241200: Build Leaderboard REST API in Go with In-Memory Storage

## Status
proposed

## Date
2023-10-24

## Drivers
- User need for a competitive gaming feature that tracks scores and rankings.
- Performance requirements for fast access to leaderboard data.

## Context
The objective is to create a leaderboard REST API that allows for submitting player scores, retrieving top-N rankings, accessing player statistics, and viewing score history. Given that the application is intended for high-performance scenarios with low latency, we are opting for in-memory storage to facilitate quick read and write operations. This choice is driven by the need for responsiveness in gameplay experiences, which requires near-instantaneous access to updated leaderboard data.

Hexagonal architecture principles will guide the implementation, where we will clearly define our domain logic, ports, and adapters. Since this is a new feature, there are no related architecture decisions recorded in existing ADRs except ADR-2603301813, which outlines the general structure for API endpoints. We will ensure that our implementation adheres to these architectural guidelines and maintains a clean separation of concerns.

## Decision
We will build a leaderboard REST API in Go, utilizing hexagonal architecture. The API will include endpoints for submitting scores, retrieving top-N rankings, accessing player stats, and viewing score history. The implementation will focus initially on a core set of endpoints (submitting scores and accessing rankings), which will establish the domain and port layers. The in-memory storage will be implemented in the adapter layer as a secondary adapter, allowing other data storage solutions to be more easily integrated later.

This decision impacts the domain layer where we will define the core models such as Player and Score, as well as the use cases related to leaderboard functionalities. It also affects the ports layer by defining the necessary interfaces for submitting scores and retrieving data.

## Consequences

### Positive
- Fast performance due to in-memory storage, facilitating a responsive user experience.
- Clear separation of concerns in the architecture, making the codebase maintainable and scalable.

### Negative
- The in-memory storage solution is not suitable for persistence; should the application restart, all scores will be lost.
- Scaling beyond a single instance may require additional effort to handle data consistency across distributed systems.

### Neutral
- While in-memory storage provides advantages in speed, it will necessitate consideration for persistent storage options in future iterations.

## Implementation

### Phases
1. **Phase 1** — Build the core functionality: endpoints for submitting scores and retrieving top-N rankings (Hex layer 1 - Domain, 2 - Ports, 3 - Adapters/Primary).
2. **Phase 2** — Add endpoints for player stats and score history (Hex layer 3 - Adapters/Primary, 4 - Adapters/Secondary).

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