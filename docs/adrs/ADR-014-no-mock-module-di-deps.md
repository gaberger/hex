# ADR-014: Ban mock.module() — Use Dependency Injection for Test Isolation

**Status:** Accepted
## Date

2026-03-17

## Context

Bun's `mock.module()` permanently replaces a Node.js module for the entire process lifetime. There is no `mock.restore()` or per-file scoping. When Bun runs test files in parallel (the default), a `mock.module('node:http', ...)` call in one test file replaces `node:http` for every other test file in the same worker process.

This caused 17+ test failures that were invisible when running individual files but appeared in the full suite. The failures manifested as:
- Dashboard integration tests receiving mocked HTTP responses instead of real ones
- Secrets adapter tests getting wrong constructor types because `node:fs` was globally replaced
- Hub command sender tests timing out because their real HTTP server was intercepted by another file's mock

Debugging was expensive because:
- Tests passed individually (`bun test file.test.ts`) but failed together (`bun test`)
- Error messages pointed to the victim test, not the polluter
- The contamination was non-deterministic depending on Bun's file execution order

Affected files before the fix:
- `dashboard-adapter.test.ts` — mocked `node:http`, `ws`, `node:fs`, `node:os`, `node:path`
- `hub-command-sender.test.ts` — mocked `node:http`
- `coordination-adapter.test.ts` — mocked `node:http`, `node:fs`, `node:child_process`, `node:util`
- `hub-launcher.test.ts` — mocked `node:fs`, `node:child_process`

## Decision

**Never use `mock.module()` in hex tests.** Instead, use constructor-injected dependencies (the "Deps" pattern).

### The Deps Pattern

Each adapter that uses external modules accepts an optional `deps` parameter in its constructor:

```typescript
export interface DashboardAdapterDeps {
  httpRequest?: typeof request;
  createWebSocket?: (url: string) => WebSocket;
  authToken?: string;
  watchDir?: typeof watch;
  pathResolve?: typeof resolve;
}

export class DashboardAdapter implements IHubCommandReceiverPort {
  private readonly _httpRequest: typeof request;

  constructor(
    private readonly ctx: AppContext,
    private readonly hubPort: number = HUB_PORT,
    deps?: DashboardAdapterDeps,   // optional — defaults to production modules
  ) {
    this._httpRequest = deps?.httpRequest ?? request;
    // ...
  }
}
```

Tests inject fakes through this interface:

```typescript
function makeDeps(): DashboardAdapterDeps {
  return {
    httpRequest: createFakeHttpRequest() as any,
    createWebSocket: createFakeWebSocket as any,
    authToken: 'test-token',
    watchDir: createFakeWatch() as any,
  };
}

const adapter = new DashboardAdapter(ctx, 9999, makeDeps());
```

### Rules

1. **No `mock.module()` calls in any test file** — enforced by grep in pre-commit hook
2. **Adapters that use `node:http`, `node:fs`, `node:child_process`, or `ws` must accept a `deps` parameter**
3. **The `deps` parameter must be optional** — production code passes nothing and gets real modules
4. **Test helpers use `makeDeps()` pattern** — centralizes fake creation, easy to customize per test

## Consequences

### Positive
- Tests are fully isolated — no cross-file contamination regardless of execution order
- Adapters are more testable and follow hex architecture's dependency inversion principle
- Tests run faster (no module replacement overhead, no import-after-mock dance)
- The pattern is self-documenting — `DashboardAdapterDeps` interface shows exactly what the adapter depends on

### Negative
- Slightly more boilerplate in adapter constructors (~5 lines per adapter)
- Type assertions (`as any`) needed when injecting fake HTTP functions that don't fully match node:http types
- Existing adapters needed one-time refactoring (4 adapters, 4 test files)

### Adapters refactored
- `DashboardAdapter` → `DashboardAdapterDeps`
- `CoordinationAdapter` → `CoordinationAdapterDeps`
- `HubLauncher` → `HubLauncherDeps`
- `HubCommandSenderAdapter` — uses real HTTP server with port 0 (random) instead of mocks

## Alternatives Considered

1. **Bun test isolation flag** — Bun has no per-file worker isolation option as of v1.3
2. **Jest-style auto-reset** — Bun's `mock.module` has no reset API
3. **Separate test configs** — Would require multiple `bun test` invocations, slowing CI
4. **Real servers for everything** — Works for some cases (hub-command-sender) but impractical for adapters with many external deps
