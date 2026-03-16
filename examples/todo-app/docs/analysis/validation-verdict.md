# Validation Verdict: WARN

## Overall Score: 78/100

### Category Breakdown

| Category | Score | Weight | Weighted |
|----------|-------|--------|----------|
| Behavioral Specs | 82 | 40% | 32.8 |
| Property Tests | 40 | 20% | 8.0 |
| Smoke Tests | 100 | 25% | 25.0 |
| Sign Convention Audit | 80 | 15% | 12.0 |
| **Total** | | **100%** | **77.8** |

---

### Behavioral Specs

| # | Behavior | Test Exists | Passes | Notes |
|---|----------|-------------|--------|-------|
| 1 | Create todo with title, priority, tags | YES | PASS | `todo-service.test.ts` create suite + `entities.test.ts` |
| 2 | Complete todo (status=completed, sets completedAt) | YES | PASS | `todo-service.test.ts` complete suite + `entities.test.ts` complete() |
| 3 | Update todo (title, priority, tags) | YES | PASS | `todo-service.test.ts` update suite (title + priority tested; tags NOT tested at service level) |
| 4 | Delete todo | YES | PASS | `todo-service.test.ts` delete suite |
| 5 | List all todos | YES | PASS | `todo-service.test.ts` getAll suite |
| 6 | Filter todos by status | YES | PASS | `todo-service.test.ts` filter suite + `entities.test.ts` TodoList.filter |
| 7 | Filter todos by priority | YES | PASS | `todo-service.test.ts` filter suite + `entities.test.ts` TodoList.filter |
| 8 | Get stats (total, pending, completed, rate) | YES | PASS | `todo-service.test.ts` stats suite + `entities.test.ts` TodoList stats |
| 9 | Health check returns status=ok | YES | PASS | `http-adapter.test.ts` GET /api/health |
| 10 | Validation error on empty title | YES | PASS | `todo-service.test.ts` + `entities.test.ts` + `http-adapter.test.ts` (400 on missing title) |
| 11 | NotFound error on unknown ID | YES | PASS | `todo-service.test.ts` (complete, update, delete) + `http-adapter.test.ts` (404) |
| 12 | Conflict error on completing already-completed | PARTIAL | PASS | Entity-level test in `entities.test.ts` ("throws when completing already completed todo"). No service-level or HTTP-level test for 409 response. |
| 13 | HTTP API serves Web UI static files | NO | - | No test for static file serving. `HttpAdapter.serveStatic()` is untested. |
| 14 | JSON storage persists and loads data | NO | - | No integration test for `JsonStorageAdapter`. Only tested indirectly via `MockStorage` in unit tests. |

**Score: 82/100** -- 12 of 14 behaviors covered; 2 untested (static files, real JSON persistence), 1 partially tested (conflict at HTTP level).

---

### Property Tests

| Property | Test Exists | Notes |
|----------|-------------|-------|
| Idempotency (create then load returns same data) | NO | No property test. `TodoList.fromData()` test in entities.test.ts is a single example, not property-based. |
| Round-trip (serialize/deserialize TodoData) | NO | `toData()` tested with single example. No `fc.property()` or equivalent. |
| Invariant: completed todo always has completedAt | NO | Checked in one test case but not as a property over random inputs. |
| Invariant: empty title always rejected | NO | Checked for `""` and `"   "` but not as a property over arbitrary whitespace strings. |

**Score: 40/100** -- No property-based tests exist. Single-example tests cover the behaviors but lack generative/fuzzing coverage. The `fromData()` round-trip test provides minimal confidence. Score reflects that example-based tests partially cover these properties but no `fast-check` or equivalent framework is used.

---

### Smoke Tests

| Check | Result | Notes |
|-------|--------|-------|
| `bun test` passes | PASS | 52 tests, 0 failures, 109 expect() calls |
| HTTP server starts | PASS | `http-adapter.test.ts` starts server on port 13456 and runs 9 integration tests |
| Happy path (create -> list -> complete -> delete) | PASS | Integration tests cover full CRUD lifecycle via HTTP |

**Score: 100/100** -- All 52 tests pass. Integration tests prove the HTTP server starts and the happy path works end-to-end.

---

### Sign Convention Audit

#### Error Handling
| Check | Result | Notes |
|-------|--------|-------|
| Typed DomainErrors used consistently | WARN | 3 instances of raw `new Error()` found: (1) `entities.ts:104` -- `throw new Error('Tag cannot be empty')` should be `ValidationError`, (2) `json-storage.ts:23` -- `throw new Error('Invalid todos.json')` should be a domain or infrastructure error, (3) `http-adapter.ts:30` -- `new Error('Body too large')` is acceptable as infrastructure-level |

#### Port Compliance
| Adapter | Implements Port | Notes |
|---------|----------------|-------|
| `JsonStorageAdapter` | `ITodoStoragePort` | PASS -- explicit `implements ITodoStoragePort` |
| `ConsoleLoggerAdapter` | `ILoggerPort` | PASS -- explicit `implements ILoggerPort` |
| `HttpAdapter` | (driving adapter) | PASS -- accepts `ITodoQueryPort` + `ITodoCommandPort` via constructor |
| `CliAdapter` | (driving adapter) | PASS -- accepts `ITodoQueryPort` + `ITodoCommandPort` via constructor |

#### Import Rules (Hexagonal Boundaries)
| Rule | Result | Notes |
|------|--------|-------|
| Domain imports nothing external | PASS | `entities.ts` imports only from `./value-objects.js` and `./errors.js`. `value-objects.ts` imports only `node:crypto` (stdlib) and `./errors.js`. |
| Ports import only domain | PASS | `ports/index.ts` imports from `../domain/entities.js` and `../domain/value-objects.js`. `ports/logger.ts` has zero imports. |
| Usecases import only domain + ports | PASS | `todo-service.ts` imports from `../domain/entities.js`, `../domain/errors.js`, `../domain/value-objects.js`, `../ports/index.js`, `../ports/logger.js`. |
| Primary adapters import only ports | FAIL | `http-adapter.ts` imports `DomainError, NotFoundError, ValidationError, ConflictError` directly from `../../core/domain/errors.js`. Should import via a port or re-export from ports. `cli-adapter.ts` imports `isValidPriority, isValidStatus, shortId` from `../../core/domain/value-objects.js` and `TodoData` from `../../core/domain/entities.js`. |
| Secondary adapters import only ports | WARN | `json-storage.ts` imports `TodoData` from `../../core/domain/entities.js` directly (type-only, but still a boundary crossing). `console-logger.ts` imports only from ports -- PASS. |
| Adapters never import other adapters | PASS | No cross-adapter imports found. |
| composition-root.ts is the only file importing adapters | PASS | Only `composition-root.ts` and `cli.ts` (entry point) import from adapters. |

#### .js Extensions on Relative Imports
| Check | Result |
|-------|--------|
| All relative imports use `.js` extension | PASS -- verified across all source files |

#### Naming Conventions
| Check | Result |
|-------|--------|
| Consistent file naming (kebab-case) | PASS |
| Consistent class naming (PascalCase) | PASS |
| Port interfaces prefixed with `I` | PASS |

**Score: 80/100** -- Primary adapters violate hex boundary rules by importing directly from domain layer. One raw `new Error()` should be a `ValidationError`. Import extensions and naming are clean.

---

### Fix Instructions

#### Priority 1: Hex Boundary Violations (adapters importing domain)

1. **`src/adapters/primary/http-adapter.ts` line 6**: Remove direct import of `DomainError`, `NotFoundError`, `ValidationError`, `ConflictError` from domain. Instead, re-export these from `src/core/ports/index.ts` or create a port-level error mapping interface (e.g., `IErrorClassifierPort`).

2. **`src/adapters/primary/cli-adapter.ts` lines 2-4**: Remove direct imports from `../../core/domain/entities.js` and `../../core/domain/value-objects.js`. Re-export needed types (`TodoData`, `isValidPriority`, `isValidStatus`, `shortId`) from ports.

3. **`src/adapters/secondary/json-storage.ts` line 4**: Import `TodoData` type from ports instead of domain. Re-export it from `src/core/ports/index.ts`.

#### Priority 2: Inconsistent Error Types

4. **`src/core/domain/entities.ts` line 104**: Change `throw new Error('Tag cannot be empty')` to `throw new ValidationError('Tag cannot be empty')`. Import `ValidationError` is already present.

#### Priority 3: Missing Tests

5. **Add integration test for `JsonStorageAdapter`**: Test actual file read/write with a temp directory. Verify round-trip persistence.

6. **Add HTTP test for conflict (409)**: Create a todo, complete it, then POST complete again -- assert 409 status and `CONFLICT` code.

7. **Add test for static file serving**: Create a temp public dir with an `index.html`, verify GET `/` returns 200 with correct content-type.

#### Priority 4: Property Tests

8. **Add property-based tests** using `fast-check` or `bun`'s built-in property testing:
   - `fc.property(fc.string(), title => { if (title.trim().length === 0) expect(() => Todo.create(title)).toThrow(); })`
   - Round-trip: `fc.property(arbTodoData, data => { expect(new Todo(data).toData()).toEqual(data); })`
   - Completed invariant: any completed `Todo` must have `completedAt > 0`

---

*Generated by hex-validate judge on 2025-07-09*
*52 tests passed, 0 failed across 4 test files*
*Source files audited: 11 (src/) + 4 (tests/)*
