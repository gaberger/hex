

# ADR-230720T1430: Implement Simple Todo REST API with Hexagonal Architecture

## Status
proposed

## Date
2023-07-20

## Drivers
- Requirement for a minimal REST API implementation
- Need to demonstrate hexagonal architecture patterns
- In-memory storage requirement for development environment
- Express framework as primary adapter

## Context
The project requires a simple todo REST API with three endpoints: POST /todos, GET /todos, and DELETE /todos/:id. The solution must use TypeScript and Express with in-memory storage. The existing hexagonal architecture (ADR-001) mandates strict layer boundaries: domain layer (business logic), ports (interfaces), and adapters (implementation). The solution must avoid dependencies between non-adjacent layers (e.g., domain cannot import adapters).

## Decision
We will implement the todo API using the hexagonal architecture pattern with the following structure:

1. **Domain Layer**: Create `Todo` entity and business rules
2. **Ports Layer**: Define `TodoRepository` interface
3. **Adapters Layer**: Implement Express routes using in-memory storage
4. **Composition Root**: Configure Express app with routes

```typescript
// domain/todo.ts
export interface Todo {
  id: string;
  title: string;
  completed: boolean;
}

// ports/todo-repository.ts
export interface TodoRepository {
  create(todo: Todo): Promise<string>;
  findAll(): Promise<Todo[]>;
  delete(id: string): Promise<void>;
}

// adapters/primary/todo-adapter.ts
import express from 'express';
import { TodoRepository } from '../ports/todo-repository';

export function createTodoRouter(repository: TodoRepository) {
  const router = express.Router();
  
  router.post('/', async (req, res) => {
    const todo = req.body as Todo;
    const id = await repository.create(todo);
    res.status(201).json({ id });
  });

  router.get('/', async (req, res) => {
    const todos = await repository.findAll();
    res.json(todos);
  });

  router.delete('/:id', async (req, res) => {
    await repository.delete(req.params.id);
    res.status(204).send();
  });

  return router;
}
```

## Consequences

### Positive
- Clear separation of concerns between business logic and implementation
- Easy testing of domain logic without Express dependencies
- Simple in-memory storage implementation
- Direct demonstration of hexagonal architecture boundaries

### Negative
- No persistence layer (data lost on server restart)
- Limited scalability for production use
- Basic error handling needs enhancement
- No validation beyond basic JSON schema

### Neutral
- Demonstrates core hexagonal architecture principles
- Minimal implementation complexity
- Uses standard Express patterns within adapter layer

## Implementation

### Phases
1. **Phase 1**: Implement domain layer and ports layer (Tiers 0-2)
2. **Phase 2**: Implement adapter layer with Express routes (Tier 3)
3. **Phase 3**: Implement composition root (Tier 4)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new implementation)