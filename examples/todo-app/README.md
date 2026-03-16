# Todo App - hex Hexagonal Architecture Example

A fully working Todo application demonstrating hexagonal (ports & adapters) architecture patterns from the hex framework.

## Architecture

```
           CLI Adapter ──┐                    ┌── JSON Storage
                         ├── Ports ── UseCases ──┤
          HTTP Adapter ──┘    (interfaces)     └── (swap: SQLite, API, etc.)
          [primary]                                [secondary]
```

**Domain** has zero external dependencies. **Ports** define contracts. **Adapters** implement them. The **composition root** wires everything together.

## Quick Start

```bash
# Add a todo
bun run src/cli.ts add "Buy groceries" --priority high --tags shopping,food

# List all todos
bun run src/cli.ts list

# Filter by status/priority
bun run src/cli.ts list --status pending --priority high

# Complete a todo (use first 8 chars of ID)
bun run src/cli.ts complete abc12345

# Update a todo
bun run src/cli.ts update abc12345 --title "New title" --priority low

# Delete a todo
bun run src/cli.ts delete abc12345

# View stats
bun run src/cli.ts stats

# Start HTTP API server on :3456
bun run src/cli.ts serve
```

## REST API (serve mode)

```
GET    /api/todos              List all (?status=pending&priority=high)
GET    /api/todos/:id          Get one
POST   /api/todos              Create { title, priority?, tags? }
PATCH  /api/todos/:id          Update { title?, priority?, tags? }
POST   /api/todos/:id/complete Complete
DELETE /api/todos/:id          Delete
GET    /api/stats              Stats
```

## Run Tests

```bash
bun test
```

## Key Patterns Demonstrated

- **Value Objects**: TodoId, TodoTitle, TodoStatus, Priority with validation
- **Immutable Entities**: Todo methods return new instances
- **Domain Events**: TodoCreated, TodoCompleted, TodoUpdated, TodoDeleted
- **Aggregate Root**: TodoList manages a collection of Todo entities
- **Port Interfaces**: ITodoStoragePort (driven), ITodoQueryPort/ITodoCommandPort (driving)
- **CQRS-lite**: Separate query and command ports
- **Composition Root**: Single wiring point, only file that imports adapters
- **Adapter Swapping**: Replace JsonStorageAdapter with any ITodoStoragePort implementation
