```markdown
# ADR-2310271145: Build Real-Time Collaborative Task Board in Rust

## Status
proposed

## Date
2023-10-27

## Drivers
- User need for real-time collaboration and instant updates
- Requirement for a REST API for CRUD operations
- Performance concerns with in-memory state management for speed

## Context
The goal is to create a real-time collaborative task board that allows multiple users to interact simultaneously. The application is designed to operate in Rust, utilizing the axum framework for handling WebSocket connections and REST API requests. Clients must be able to see updates immediately as tasks are created, edited, or deleted, necessitating a live broadcast channel.

In addition to supporting real-time interactions, the application will feature a RESTful API to perform Create, Read, Update, and Delete (CRUD) operations on task entities. The choice of in-memory state is aimed at achieving high-performance operations and ease of state management during collaborative sessions.

## Decision
We will build the task board using a hexagonal architecture where the domain will encapsulate the core business logic for task management. The primary adapter will implement the REST API for CRUD operations and the WebSocket connection, while the secondary adapter will handle data persistence in memory. The live broadcast channel will be established within the primary adapter to ensure all connected clients receive updates seamlessly.

The development will occur in phases: first, we will establish the domain and primary adapters responsible for task management and WebSocket communication. Following this, we will enhance the system to include the REST API, ensuring that all components adhere to the principles of separation of concerns and maintainability.

## Consequences

### Positive
- Supports real-time collaboration, enhancing user experience.
- In-memory state management allows for quick read/write operations, improving performance.

### Negative
- In-memory state may lead to data loss if the application crashes or is restarted.
- Limited scalability; as the application grows, in-memory storage might not be sufficient.

### Neutral
- The reliance on WebSockets may introduce complexity in connection management and error handling.

## Implementation

### Phases
1. Phase 1 — Implement the domain model and primary adapter for task management and real-time updates via WebSockets (hex level 1 and 2).
2. Phase 2 — Implement the REST API for CRUD operations, dependent on the completion of phase 1 (hex level 2 and 3).

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