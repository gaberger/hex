---
name: hex-scaffold
description: Scaffold a new hexagonal architecture project. Use when the user asks to "create hex project", "scaffold hexagonal", "new ports and adapters project", "hex-intf init", or "init hexagonal project".
---

# Hex Scaffold — Create a Hexagonal Architecture Project

CRITICAL: Do NOT generate code immediately. This skill is a guided conversation that gathers requirements BEFORE scaffolding.

## Phase 1: Discovery (MUST complete before any code)

Ask the user these questions. Wait for answers before proceeding.

### Question 1: What does this application do?
"What's the core purpose of this application? Describe in 1-2 sentences what it does."

Examples of good answers:
- "A todo list manager"
- "A webhook relay that forwards events between services"
- "A CLI tool that analyzes git commit patterns"

### Question 2: How will users interact with it?
"How should users interact with this? Pick one or more:"

| Option | Description |
|--------|-------------|
| **CLI** | Terminal commands (e.g., `myapp add "task"`, `myapp list`) |
| **Web UI** | Browser-based interface with a local server |
| **REST API** | JSON API consumed by other services or frontends |
| **Library** | Imported as a package by other TypeScript/Node projects |
| **MCP Tool** | Claude Code tool integration via Model Context Protocol |

This determines which **primary adapters** to scaffold.

### Question 3: Where does data live?
"How should this app store its data?"

| Option | Description |
|--------|-------------|
| **JSON file** | Simple file-based persistence (good for CLIs, small apps) |
| **SQLite** | Embedded database (good for local apps with queries) |
| **PostgreSQL/MySQL** | External database (good for APIs, multi-user) |
| **External API** | Delegates storage to another service |
| **In-memory only** | No persistence (good for libraries, stream processors) |

This determines which **secondary adapters** to scaffold.

### Question 4: What's the domain model?
"What are the main entities? List 2-5 nouns that represent core concepts."

Example for a todo app: "Todo, TodoList, Tag"
Example for a webhook relay: "Webhook, Endpoint, Delivery, RetryPolicy"

### Question 5: Project name
"What should the project directory be called?"

## Phase 2: Design Summary

Before generating any code, present a design summary and get confirmation:

```
Project: {name}
Purpose: {description}

Primary Adapters (how users interact):
  - {adapter1}: {what it does}
  - {adapter2}: {what it does}

Secondary Adapters (external systems):
  - {storage}: {what it does}

Domain Entities:
  - {Entity1}: {brief description}
  - {Entity2}: {brief description}

Port Interfaces:
  Input (driving):
    - I{Entity}CommandPort: create, update, delete
    - I{Entity}QueryPort: getById, list, filter, stats
  Output (driven):
    - I{Entity}StoragePort: load, save

Files to create: ~{count}
```

Ask: "Does this look right? Any changes before I scaffold?"

## Phase 3: Scaffold (only after Phase 2 approval)

### Step 1: Create directory structure

```bash
mkdir -p {name}/src/core/{domain,ports,usecases}
mkdir -p {name}/src/adapters/{primary,secondary}
mkdir -p {name}/tests/unit
```

### Step 2: Domain layer (zero external imports)

Create `src/core/domain/value-objects.ts`:
- Type aliases for IDs (branded strings)
- Status enums relevant to the domain
- Small value types

Create `src/core/domain/entities.ts`:
- Domain event discriminated union
- Entity classes with immutable state transitions
- Aggregate root if applicable

### Step 3: Port interfaces

Create `src/core/ports/index.ts`:
- Command port (mutations) — derived from Question 2
- Query port (reads) — derived from Question 2
- Storage port (persistence) — derived from Question 3
- Use domain value objects for all types

### Step 4: Use cases

Create `src/core/usecases/{entity}-service.ts`:
- Implements command + query ports
- Composes storage port via constructor injection
- Validates inputs at this boundary
- Emits domain events

### Step 5: Adapters (ONLY the ones the user chose)

For each primary adapter from Question 2:
- CLI: `src/adapters/primary/cli-adapter.ts` with arg parsing
- Web: `src/adapters/primary/http-adapter.ts` with node:http
- API: same as Web but JSON-only (no HTML)
- Library: just export ports from `src/index.ts`
- MCP: `src/adapters/primary/mcp-adapter.ts`

For each secondary adapter from Question 3:
- JSON: `src/adapters/secondary/json-storage.ts`
- SQLite: `src/adapters/secondary/sqlite-storage.ts`
- External API: `src/adapters/secondary/api-storage.ts`
- In-memory: `src/adapters/secondary/memory-storage.ts`

### Step 6: Wiring

Create `src/composition-root.ts`:
- Only file that imports adapters
- Constructor-injects storage into use case
- Exports factory function

Create `src/cli.ts` (entry point).

### Step 7: Tests

Create London-school mock tests for:
- Domain entities (immutability, validation, events)
- Use case service (mock storage, test CRUD)

### Step 8: Config

Create:
- `package.json` with scripts: start, test, dev
- `tsconfig.json` with strict mode
- `README.md` with usage examples specific to chosen adapters

### Step 9: Verify

Run `bun test` and fix any failures. Run the primary adapter to verify it works:
- CLI: `bun run src/cli.ts help`
- Web: Start server, verify / responds
- API: Start server, verify healthcheck

## Rules

- Domain imports NOTHING external
- Ports import only from domain
- Adapters import only from ports
- composition-root.ts is the ONLY cross-boundary file
- All imports use `.js` extensions
- Every file under 150 lines
- The app MUST actually work end-to-end after scaffolding
