```markdown
# ADR-2310201230: Real-Time Collaborative Task Board with WebSockets

## Status
proposed

## Date
2023-10-20

## Drivers
- User need for real-time collaboration features.
- Requirement for a responsive and interactive user experience.
- Need for a lightweight in-memory state management solution.

## Context
The goal is to build a real-time collaborative task board that allows users to manage tasks efficiently. Given the requirement for instant updates for all connected clients, WebSockets via the axum framework will enable efficient communication between the server and clients. The system will also include a REST API for standard Create, Read, Update, and Delete (CRUD) operations on tasks. 

Utilizing in-memory state will streamline data access and facilitate low-latency interactions, aligning with the real-time aspect of the application. The architecture will follow hexagonal principles as established in ADR-001, ensuring modularity and separation of concerns.

## Decision
We will implement a real-time collaborative task board using axum's WebSocket capabilities. The architecture will comprise a domain layer for task management logic, a ports layer defining the interfaces for REST API and WebSocket connections, and adapters for both primary (HTTP, WebSocket) and secondary (in-memory state) interactions. The use of in-memory state for task storage will optimize performance for real-time updates, while the REST API will ensure traditional CRUD functionality.

The live broadcast channel will be established via the WebSocket implementation to enable all connected clients to receive updates instantly. This implementation directly corresponds to the use cases of the task board application.

## Consequences

### Positive
- Users receive real-time updates, enhancing collaboration and user engagement.
- In-memory storage allows for faster data interactions and minimal latency.
- Clear separation of concerns enhances maintainability and testability.

### Negative
- In-memory state may complicate persistence beyond application lifecycle, necessitating additional mechanisms for data storage.
- Potential scalability issues with WebSocket connections if the number of clients grows significantly.

### Neutral
- While opting for in-memory storage simplifies the architecture, it requires careful consideration of state management, especially for persistence across application restarts.

## Implementation

### Phases
1. Implement the domain layer for task management logic and the REST API for CRUD operations (Hex layers 1-3).
2. Establish WebSocket handling to manage real-time client communication and live updates (Hex layers 2-4).

### Affected Layers
- [x] domain/
- [x] ports/
- [x] adapters/primary/
- [x] adapters/secondary/
- [x] usecases/
- [ ] composition-root

### Migration Notes
None
```