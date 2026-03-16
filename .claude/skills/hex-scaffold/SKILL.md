---
name: hex-scaffold
description: Scaffold a new hexagonal architecture project. Use when the user asks to "create hex project", "scaffold hexagonal", "new ports and adapters project", "hex-intf init", or "init hexagonal project".
---

# Hex Scaffold — Create a Hexagonal Architecture Project

CRITICAL: Do NOT enter plan mode (EnterPlanMode). Proceed directly with execution.

If the `--yes` or `-y` flag was passed (check `ctx.autoConfirm`), skip Phase 1 entirely and use these defaults:
- Interface: CLI
- Storage: JSON file
- Entities: inferred from project name or user description
- Style: Minimal MVP
Then go straight to Phase 2: Scaffold.

Otherwise, use AskUserQuestion for an interactive wizard experience.

## Phase 1: Interactive Discovery Wizard (skipped with --yes)

Use the AskUserQuestion tool to gather requirements interactively. Ask in batches of 3-4 questions max.

### Wizard Step 1 (3 questions)

```tool
AskUserQuestion({
  questions: [
    {
      question: "How will users interact with this application?",
      header: "Interface",
      multiSelect: true,
      options: [
        { label: "CLI (Recommended)", description: "Terminal commands like `myapp add \"task\"`. Fastest to scaffold, works everywhere." },
        { label: "Web UI", description: "Browser-based interface with local server. Great for visual apps." },
        { label: "REST API", description: "JSON API for other services. Good for backends and microservices." },
        { label: "Library", description: "Imported as npm package. No runtime interface, just typed exports." }
      ]
    },
    {
      question: "Where should this app store its data?",
      header: "Storage",
      multiSelect: false,
      options: [
        { label: "JSON file (Recommended)", description: "Simple file persistence. Perfect for CLIs and small tools." },
        { label: "SQLite", description: "Embedded database. Good for queries and local apps." },
        { label: "In-memory only", description: "No persistence. Good for stream processors and libraries." },
        { label: "External API", description: "Delegates storage to another service via HTTP/gRPC." }
      ]
    },
    {
      question: "What are the main domain entities? (the core nouns)",
      header: "Entities",
      multiSelect: false,
      options: [
        { label: "Todo, TodoList", description: "Task management domain" },
        { label: "User, Session", description: "Authentication domain" },
        { label: "Event, Handler", description: "Event processing domain" },
        { label: "Custom", description: "I'll describe my own entities" }
      ]
    }
  ]
})
```

### Wizard Step 2 (1-2 questions, based on Step 1 answers)

Use AskUserQuestion with preview to show the proposed architecture:

```tool
AskUserQuestion({
  questions: [
    {
      question: "Here's the architecture plan. Which style fits your project?",
      header: "Style",
      multiSelect: false,
      options: [
        {
          label: "Minimal MVP",
          description: "Just the core CRUD operations. Ship fast, add later.",
          preview: "src/\n  core/\n    domain/entities.ts     # 1 entity\n    ports/index.ts         # 3 interfaces\n    usecases/service.ts    # CRUD logic\n  adapters/\n    primary/cli.ts         # 5 commands\n    secondary/storage.ts   # JSON file\n  composition-root.ts\ntests/\n  unit/service.test.ts\n\n~8 files, ~600 lines"
        },
        {
          label: "Full Featured",
          description: "CRUD + filtering + stats + validation + events.",
          preview: "src/\n  core/\n    domain/\n      value-objects.ts    # IDs, statuses, types\n      entities.ts         # Entities + events\n    ports/index.ts         # 5 interfaces (CQRS)\n    usecases/service.ts    # Full orchestration\n  adapters/\n    primary/cli.ts         # 8+ commands\n    secondary/storage.ts   # Atomic writes\n  composition-root.ts\ntests/\n  unit/\n    entities.test.ts\n    service.test.ts\n\n~12 files, ~1200 lines"
        },
        {
          label: "Production",
          description: "Full featured + error handling + logging + health checks.",
          preview: "src/\n  core/\n    domain/\n      value-objects.ts\n      entities.ts\n      errors.ts          # Domain error types\n    ports/\n      commands.ts         # Write operations\n      queries.ts          # Read operations\n      storage.ts          # Persistence\n    usecases/service.ts\n  adapters/\n    primary/\n      cli.ts\n      http.ts            # REST + health\n    secondary/\n      storage.ts\n      logger.ts\n  composition-root.ts\ntests/\n  unit/ (3 files)\n  integration/ (1 file)\n\n~16 files, ~1800 lines"
        }
      ]
    }
  ]
})
```

## Phase 2: Scaffold Based on Answers

After the wizard completes, generate ONLY the files that match the user's choices.

### Map answers to adapters:

| User chose | Primary adapter to create |
|-----------|--------------------------|
| CLI | `src/adapters/primary/cli-adapter.ts` with arg parsing |
| Web UI | `src/adapters/primary/http-adapter.ts` serving HTML |
| REST API | `src/adapters/primary/http-adapter.ts` JSON-only |
| Library | Just `src/index.ts` with type exports |

| User chose | Secondary adapter to create |
|-----------|---------------------------|
| JSON file | `src/adapters/secondary/json-storage.ts` |
| SQLite | `src/adapters/secondary/sqlite-storage.ts` |
| In-memory | `src/adapters/secondary/memory-storage.ts` |
| External API | `src/adapters/secondary/api-storage.ts` |

### Generate in order:

1. `mkdir -p` for directory structure
2. Domain layer (value-objects.ts, entities.ts) — zero external imports
3. Ports (index.ts) — imports only domain types
4. Use cases (service.ts) — imports domain + ports
5. Selected primary adapter(s) — imports ports only
6. Selected secondary adapter — imports ports only
7. composition-root.ts — the ONLY cross-boundary file
8. Entry point (cli.ts or index.ts)
9. Tests (London-school mocks)
10. package.json, tsconfig.json, README.md

### Verify:

Run `bun test` and verify the app works end-to-end via the chosen primary adapter.

## Rules

- Domain imports NOTHING external
- Ports import only from domain
- Adapters import only from ports
- composition-root.ts is the ONLY cross-boundary file
- All imports use `.js` extensions
- Every file under 150 lines
- The app MUST actually work after scaffolding
- NEVER scaffold adapters the user didn't ask for
