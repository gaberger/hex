# ADR-2412172334: Single-Binary URL Shortener with In-Memory Storage

## Status
proposed

## Date
2024-12-17

## Drivers
- User requirement to build a complete URL shortener REST API feature
- Need for simplicity through single-binary deployment model
- Development speed priority over persistence durability
- Educational/prototype use case requiring minimal external dependencies

## Context

The user has requested implementation of a URL shortener REST API as a single Go binary with in-memory storage. This represents a common microservice pattern that can benefit from hexagonal architecture even in its simplest form. While existing ADRs establish the hexagonal foundation (ADR-001) and multi-language support (ADR-003), this specific decision addresses how to structure a complete feature within the constraints of a single binary and volatile storage.

The choice of in-memory storage suggests this is either a prototype, educational example, or a service where data loss on restart is acceptable. The single-binary constraint means all hex layers must coexist in one executable, but the architectural boundaries must remain clear to demonstrate proper separation of concerns.

Traditional approaches might place everything in main.go or use a simple handler-based structure, but this would violate hexagonal principles and miss the opportunity to showcase clean architecture in a compact form.

## Decision

We will implement a complete URL shortener following hexagonal architecture with all layers present in a single Go binary. The implementation will demonstrate proper hex layer separation while maintaining deployment simplicity through in-memory storage and single-binary distribution.

The domain layer will contain URL shortening business logic and entities. Primary adapters will expose REST endpoints via HTTP handlers. Secondary adapters will implement in-memory storage using Go maps with mutex protection. Use cases will orchestrate between ports without knowledge of implementation details. The composition root in main.go will wire dependencies and start the HTTP server.

## Consequences

### Positive
- Demonstrates hexagonal architecture principles in a compact, understandable example
- Zero external dependencies for storage (no database setup required)
- Fast startup and simple deployment model
- Clear educational value showing hex layer boundaries
- Immediate testability of business logic through port interfaces

### Negative
- Data loss on application restart (volatile storage)
- No persistence across deployments or crashes
- Single point of failure with no redundancy options
- Memory usage grows unbounded without cleanup mechanisms
- Not suitable for production use cases requiring durability

### Neutral
- Architecture patterns remain identical to persistent storage implementations
- Migration to persistent storage requires only adapter layer changes

## Implementation

### Phases
1. **Domain & Ports Definition** — Define URL entities, shortening business rules, and port interfaces (repository, ID generator)
2. **Use Cases Implementation** — Create URL shortening and retrieval use cases that depend only on domain and ports
3. **Secondary Adapters** — Implement in-memory repository with thread-safe operations and ID generation
4. **Primary Adapters** — Build REST handlers for POST /shorten and GET /{short-code} endpoints
5. **Composition Root** — Wire all dependencies in main.go and configure HTTP server

### Affected Layers
- [x] domain/
- [x] ports/
- [x] adapters/primary/
- [x] adapters/secondary/
- [x] usecases/
- [x] composition-root

### Migration Notes
To upgrade from in-memory to persistent storage, only the secondary adapter implementation needs replacement. The repository port interface remains identical, preserving all business logic and use cases. Thread-safety considerations in the in-memory implementation will inform concurrent access patterns for database adapters.