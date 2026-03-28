

# ADR-230720T1430: Implement In-Memory Todo Storage Adapter

## Status
proposed

## Date
2023-07-20

## Drivers
- Requirement for persistent todo storage
- Hexagonal architecture compliance
- Axum framework integration
- No external dependencies

## Context
The project requires a REST API for todo management with in-memory storage. The existing hexagonal architecture (ADR-001) mandates strict layer boundaries: domain layer (business logic), ports layer (interfaces), and adapters layer (implementations). Current storage solutions (ADR-015) are not applicable for in-memory requirements. The API must support POST /todos, GET /todos, and DELETE /todos/:id operations with JSON responses.

## Decision
We will implement a new `in_memory` adapter in the `adapters/storage` directory that:
1. Implements the `StoragePort` trait from `ports/storage.rs`
2. Uses a `HashMap<u64, Todo>` for storage
3. Provides `create_todo`, `list_todos`, and `delete_todo` methods
4. Adheres to hex boundary rules (ports imports domain, adapters imports ports)

```rust
// adapters/storage/in_memory.rs
pub struct InMemoryStorage {
    todos: HashMap<u64, Todo>,
    next_id: u64,
}

impl StoragePort for InMemoryStorage {
    fn create_todo(&mut self, todo: Todo) -> Result<u64, Error> {
        let id = self.next_id;
        self.todos.insert(id, todo);
        self.next_id += 1;
        Ok(id)
    }

    fn list_todos(&self) -> Vec<Todo> {
        self.todos.values().cloned().collect()
    }

    fn delete_todo(&mut self, id: u64) -> Result<(), Error> {
        self.todos.remove(&id).ok_or(Error::NotFound)?;
        Ok(())
    }
}
```

## Consequences

### Positive
- Eliminates external dependency requirements
- Enables fast development iteration
- Maintains testability through dependency injection
- Complies with existing hex architecture boundaries

### Negative
- Data lost on application restart
- Limited to single-process execution
- No persistence guarantees

### Neutral
- In-memory storage is suitable for development and testing
- Aligns with initial MVP requirements

## Implementation

### Phases
1. **Phase 1**: Implement domain models and ports (ADR-001 compliant)
2. **Phase 2**: Develop in-memory adapter implementation
3. **Phase 3**: Integrate adapter with Axum routes

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new implementation)