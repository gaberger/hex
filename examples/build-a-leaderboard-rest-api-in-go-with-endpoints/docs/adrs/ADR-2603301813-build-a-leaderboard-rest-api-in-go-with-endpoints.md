```markdown
# ADR-2310241200: Build Leaderboard REST API in Go with In-Memory Storage

## Status
proposed

## Date
2023-10-24

## Drivers
- User need for a performant and efficient leaderboard feature.
- The requirement for straightforward implementation with minimal complexity.

## Context
The project requires a leaderboard REST API that allows users to submit scores, retrieve rankings, view player statistics, and check score history. In-memory storage is favored due to its speed and simplicity, especially during the initial development phase. This decision stems from the need to quickly prototype and iterate the leaderboard functionalities without the complexity of persistent data storage.

The API will align with hexagonal architecture principles, ensuring separation of concerns through distinct layers. The domain layer will encapsulate the leaderboard logic, while the ports will define the necessary interfaces for interacting with the domain. Adapters will handle the REST communication and any required input/output transformations.

With this context in mind, we need to design the API endpoints while ensuring the architecture supports future scalability and potential persistence changes.

## Decision
We will implement a leaderboard REST API in Go with the following endpoints: 
1. **Submit Score** - to allow users to submit scores.
2. **Retrieve Top-N Rankings** - to provide a ranked list of players based on their scores.
3. **Player Stats** - to retrieve statistics for individual players.
4. **Score History** - to view the historical scores for a player.

All data will be stored in memory initially to enable high performance and rapid development. The implementation will consist of the following affected layers:
- **Domain**: Contains the leaderboard business logic (e.g., score submission, ranking calculation).
- **Ports**: Define the interfaces needed for the communication from the API to the domain.
- **Adapters/Primary**: Implement the HTTP handlers for REST API interactions.
- **Adapters/Secondary**: Currently none, but could be added later for persistence or external integrations if needed.
- **Usecases**: Manage the interaction between the port interfaces and the domain logic.

We will also keep the architecture flexible for future changes that may involve switching to a persistent storage solution.

## Consequences

### Positive
- Allows for rapid prototyping and development of the leaderboard features.
- High performance due to in-memory data handling, providing a responsive user experience.

### Negative
- In-memory storage limits scalability and persistence; data will be lost on service restart.
- Potential technical debt if persistence mechanisms are not planned from the outset.

### Neutral
- The initial simplicity may lead to an over-reliance on in-memory storage, which could complicate future transitions to more persistent solutions.

## Implementation

### Phases
1. **Phase 1**: Develop the core functionality and endpoints for submitting scores and retrieving top-N rankings.
2. **Phase 2**: Implement player stats and score history features based on the first phase, ensuring all components use the established domain and ports.

### Affected Layers
- [x] domain/
- [x] ports/
- [x] adapters/primary/
- [ ] adapters/secondary/
- [x] usecases/
- [ ] composition-root

### Migration Notes
None.
```