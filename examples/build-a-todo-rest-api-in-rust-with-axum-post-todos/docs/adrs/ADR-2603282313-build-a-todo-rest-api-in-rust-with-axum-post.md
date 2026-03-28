

# ADR-230720T1430: Implement In-Memory Todo Storage Adapter

## Status
proposed

## Date
2023-07-20

## Drivers
- Need for persistent storage in todo API
- Requirement to maintain hexagonal architecture boundaries
- No external dependencies allowed in production

## Context
The todo API requires persistent storage for todo items. Current architecture enforces strict hexagonal boundaries where domain models must not depend on external systems. Existing storage solutions (ADR-015 SQLite) are not suitable for in-memory requirements. The solution must:
1. Maintain separation between domain logic and storage
2. Allow easy replacement with database adapter later
3. Support JSON serialization/deserialization
4. Handle concurrent requests safely

## Decision
We will implement a new `in-memory` storage adapter that:
1. Implements the `StoragePort` interface from ADR-001
2. Uses `serde` for JSON serialization
3. Maintains thread-safe operations via `Arc<Mutex<...>>`
4. Follows hex layer boundaries:
   - Domain layer: `domain/todo.rs` defines `Todo` struct
   - Ports layer: `ports/storage.rs` defines `StoragePort` trait
   - Adapters layer: `adapters/storage/in_memory.rs` implements storage

## Consequences

### Positive
- Zero external dependencies
- Simple implementation for MVP
- Easy to test in isolation
- No database setup required

### Negative
- Data lost on server restart
- Limited to single-node deployments
- No persistence across restarts

### Neutral
- In-memory storage is faster than disk I/O
- No database connection pooling needed

## Implementation

### Phases
1. **Phase 1 (Domain & Ports)**: Implement `domain/todo.rs` and `ports/storage.rs` (Tiers 0-2)
2. **Phase 2 (Adapter)**: Implement `adapters/storage/in_memory.rs` (Tier 3)
3. **Phase 3 (Integration)**: Wire adapter into Axum routes (Tier 4)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None - this is a new implementation with no existing data to migrate