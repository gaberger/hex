

# ADR-2303151430: Implement In-Memory Todo Storage Adapter

## Status
proposed

## Date
2023-03-15

## Drivers
- Requirement for minimal initial implementation
- Need for fast iteration during development
- No requirement for persistence beyond development phase
- Existing hex framework constraints requiring port/adapter separation

## Context
The project requires a REST API for todo management with three endpoints: POST /todos, GET /todos, and DELETE /todos/:id. The current hex architecture enforces strict layer boundaries where domain models must not depend on external systems, ports define interfaces without implementation details, and adapters implement ports without importing other adapters. The team needs a storage solution that:
1. Supports JSON serialization/deserialization
2. Allows fast iteration during development
3. Complies with hex architecture boundaries
4. Can be replaced later with persistent storage

Existing hex implementation shows successful use of ports/adapters pattern in other components (ADR-001, ADR-005). The current architecture score of 7800/100 indicates strong adherence to hex principles.

## Decision
We will implement a new `in_memory` adapter in the `adapters/secondary` layer that:
1. Implements the `StoragePort` interface defined in `ports/storage.rs`
2. Uses a `HashMap<u64, Todo>` for storage
3. Provides `create_todo`, `list_todos`, and `delete_todo` methods
4. Uses `serde` for JSON serialization/deserialization
5. Follows hex boundary rules by not importing other adapters

```rust
// adapters/secondary/in_memory/storage.rs
pub struct InMemoryStorage {
    storage: HashMap<u64, Todo>,
    next_id: u64,
}

impl StoragePort for InMemoryStorage {
    fn create_todo(&mut self, todo: Todo) -> Result<u64, Error> {
        let id = self.next_id;
        self.storage.insert(id, todo);
        self.next_id += 1;
        Ok(id)
    }

    fn list_todos(&self) -> Result<Vec<Todo>, Error> {
        Ok(self.storage.values().cloned().collect())
    }

    fn delete_todo(&mut self, id: u64) -> Result<(), Error> {
        self.storage.remove(&id).ok_or(Error::NotFound)?;
        Ok(())
    }
}
```

## Consequences

### Positive
- Eliminates need for external database dependencies during development
- Enables rapid iteration without persistence concerns
- Maintains strict hex architecture boundaries
- Provides clear path for future persistence implementation

### Negative
- Data lost on application restart
- Limited to single-node operation
- No persistence guarantees

### Neutral
- Implementation complexity remains minimal
- Maintains testability through dependency injection
- No impact on domain model design

## Implementation

### Phases
1. **Phase 1 (Domain & Ports):** Define `StoragePort` trait in `ports/storage.rs` (completed)
2. **Phase 2 (Adapter):** Implement `InMemoryStorage` in `adapters/secondary/in_memory/storage.rs`
3. **Phase 3 (Composition Root):** Configure `InMemoryStorage` in `composition_root.rs`

### Affected Layers
- [x] domain/
- [x] ports/
- [x] adapters/secondary/
- [ ] adapters/primary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (temporary solution, no data migration required)