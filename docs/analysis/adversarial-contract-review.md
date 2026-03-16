# Adversarial Contract Completeness Review

Reviewed: all port interfaces in `src/core/ports/` and all adapters in `src/adapters/`.

---

## 1. CRITICAL Findings

### 1.1 IArchAnalysisPort.buildDependencyGraph ignores its `rootPath` parameter

**File:** `src/core/ports/index.ts:257`, `src/core/usecases/arch-analyzer.ts:65`

The port declares `buildDependencyGraph(rootPath: string)` but the ArchAnalyzer implementation prefixes the parameter with `_rootPath` and calls `this.collectSummaries()`, which uses `this.fs.glob('**/*.ts')` -- always scanning from the FileSystemAdapter's configured root, not the provided `rootPath`.

All five methods on IArchAnalysisPort accept `rootPath` and all five ignore it.

- **Interpretation A:** `rootPath` scopes which subtree to analyze (e.g., `src/adapters/` only).
- **Interpretation B:** `rootPath` is vestigial; the adapter's root is the real scope.

**Severity:** CRITICAL -- any caller passing a subdirectory path gets silently incorrect results (full-project analysis instead of scoped).

**Resolution:** Either remove `rootPath` from the port signature (breaking change), or pass it through to `fs.glob` as a prefix. Document which one is correct.

---

### 1.2 RufloAdapter silently swallows all parse failures, returning fabricated defaults

**File:** `src/adapters/secondary/ruflo-adapter.ts:128-149`

`parseStatus`, `parseTasks`, `parseAgents`, and `memorySearch` all catch JSON parse errors and return hardcoded defaults (`{ status: 'idle', agentCount: 0 }`, empty arrays, etc.). `memoryRetrieve` catches all errors and returns `null`.

- **Interpretation A:** "No tasks returned" means the swarm has no tasks.
- **Interpretation B:** "No tasks returned" means the CLI binary is missing, the daemon is down, or the output format changed.

These are indistinguishable to the caller.

**Severity:** CRITICAL -- a broken ruflo installation silently looks like a healthy idle swarm. The `status()` call literally fabricates a `SwarmStatus` with `status: 'idle'` when the CLI errors out. Callers cannot distinguish "swarm is idle" from "swarm infrastructure is down."

**Resolution:** Introduce a typed error hierarchy (`SwarmConnectionError`, `SwarmParseError`) and let callers decide whether to degrade gracefully. At minimum, log the raw output before discarding it.

---

### 1.3 Dual AppContext interfaces -- CLI vs composition root

**File:** `src/adapters/primary/cli-adapter.ts:18-23`, `src/composition-root.ts:33-49`

The CLI adapter defines its own `AppContext` with 3 fields (`rootPath`, `archAnalyzer`, `ast`, `fs`). The composition root defines a different `AppContext` with 11 fields. Both are exported. Both use the same name.

- **Interpretation A:** CLI's AppContext is a deliberate subset (interface segregation).
- **Interpretation B:** They are meant to be the same type and drifted.

**Severity:** CRITICAL -- passing a composition-root `AppContext` to `runCLI` works by structural typing today, but the CLI adapter lacks access to `git`, `worktree`, `build`, `swarm`, `eventBus`, `notifier`, and `notificationOrchestrator`. Any future CLI command needing those ports will silently fail at runtime because the CLI's `AppContext` type doesn't include them.

**Resolution:** Make CLI's AppContext a `Pick<>` of the composition root's AppContext, or import the canonical type directly. One authoritative definition.

---

### 1.4 TreeSitter fallback stub violates IASTPort semantic contract

**File:** `src/composition-root.ts:79-89`

When TreeSitterAdapter fails to initialize, the fallback returns `{ exports: [], imports: [], lineCount: 0, tokenEstimate: 0 }` for every file. This is structurally valid but semantically false -- it claims every file has zero lines and zero exports.

- **Interpretation A:** An empty summary means "analysis unavailable."
- **Interpretation B:** An empty summary means "the file genuinely has no exports."

The ArchAnalyzer trusts these summaries. With the fallback, `findDeadExports` returns zero dead exports (nothing was exported), `buildDependencyGraph` returns zero edges (nothing was imported), and `analyzeArchitecture` reports `healthScore: 100`. A project with severe violations would score perfectly.

**Severity:** CRITICAL -- silent false-positive analysis results. The `analyze` CLI command would exit 0 (success) for a completely broken project.

**Resolution:** Either make `IASTPort.extractSummary` return `ASTSummary | null` to signal unavailability, or throw a typed `ASTUnavailableError` that callers must handle. The fallback stub should not exist.

---

## 2. HIGH Findings

### 2.1 IGitPort.commit return type is `Promise<string>` -- what string?

**File:** `src/core/ports/index.ts:201`, `src/adapters/secondary/git-adapter.ts:27-30`

The port says `commit(message: string): Promise<string>`. The GitAdapter returns `rev-parse --short HEAD` (a short hash). But the port does not specify this.

- **Interpretation A:** Returns the full commit SHA.
- **Interpretation B:** Returns the short hash.
- **Interpretation C:** Returns the commit message echo.

**Severity:** HIGH -- `RufloAdapter.completeTask` passes this value as `commitHash` in `commit ${commitHash}`. If another IGitPort adapter returned the full 40-char hash, the ruflo task result format would change.

**Resolution:** Define a branded type `CommitHash` or at minimum document "returns the abbreviated commit hash (7+ hex chars)."

---

### 2.2 IFileSystemPort.write -- no contract on parent directory creation

**File:** `src/core/ports/index.ts:209`, `src/adapters/secondary/filesystem-adapter.ts:23-26`

The port says `write(filePath: string, content: string): Promise<void>`. The FileSystemAdapter auto-creates parent directories with `mkdir(dirname(abs), { recursive: true })`. The port does not promise this.

- **Interpretation A:** `write` creates intermediate directories as needed.
- **Interpretation B:** `write` throws if the parent directory does not exist.

**Severity:** HIGH -- an adapter author implementing IFileSystemPort for, say, an S3 backend would reasonably not auto-create "directories." Callers who depend on the auto-mkdir behavior would break.

**Resolution:** Either add `mkdir(path: string): Promise<void>` to the port and require callers to create directories first, or document that `write` MUST create parent directories.

---

### 2.3 IFileSystemPort.resolve -- absolute vs relative path ambiguity

**File:** `src/core/ports/index.ts:207-212`, `src/adapters/secondary/filesystem-adapter.ts:47-49`

All IFileSystemPort methods accept `filePath: string`. The FileSystemAdapter resolves paths with `join(this.root, filePath)`. If `filePath` is absolute (e.g., `/etc/passwd`), `join` returns the absolute path, **bypassing the root**.

- **Interpretation A:** Paths are always relative to the project root.
- **Interpretation B:** Absolute paths are allowed and used as-is.

**Severity:** HIGH -- this is a path traversal vulnerability. A malicious or buggy caller can read/write any file on the filesystem by passing an absolute path.

**Resolution:** Validate that `resolve(filePath)` starts with `this.root`. Use `path.resolve(this.root, filePath)` and then assert the result is within `this.root`. This is a security boundary.

---

### 2.4 IWorktreePort.create -- branch name validation absent

**File:** `src/core/ports/index.ts:194`, `src/adapters/secondary/worktree-adapter.ts:27-30`

`create(branchName: string)` passes `branchName` directly to `git worktree add ... -b branchName`. No validation on:
- Spaces in branch names
- Slashes (is `feature/foo` allowed?)
- Names starting with `-` (interpreted as git flags)
- Empty string
- Names with `..` (ref traversal)

The WorktreeAdapter also uses `branchName` in a filesystem path: `hex-${branchName}`. A branch name like `../../etc` would create a worktree outside the intended directory.

- **Interpretation A:** Branch names must be valid git ref names.
- **Interpretation B:** The port accepts any string and the adapter validates.

**Severity:** HIGH -- directory traversal via branch name, and potential git flag injection via names starting with `-`.

**Resolution:** Add a `BranchName` branded type with a factory function that validates against git ref naming rules (`git check-ref-format`). Use `--` before branch names in git commands to prevent flag injection.

---

### 2.5 TokenBudget invariant not enforced

**File:** `src/core/ports/index.ts:36-40`

```typescript
interface TokenBudget {
  maxTokens: number;
  reservedForResponse: number;
  available: number;
}
```

Must `available === maxTokens - reservedForResponse`? Can `available` be negative? Can `reservedForResponse > maxTokens`?

- **Interpretation A:** `available` is a computed value and must equal `maxTokens - reservedForResponse`.
- **Interpretation B:** `available` is independently set (e.g., accounting for existing context).

**Severity:** HIGH -- ILLMPort.prompt receives this budget. If `available` is inconsistent with `maxTokens - reservedForResponse`, the LLM adapter cannot know which value to trust.

**Resolution:** Either make `TokenBudget` a class with a constructor that enforces the invariant and computes `available`, or remove `available` and let callers compute it.

---

### 2.6 EventFilter.minSeverity references severity levels that DomainEvents lack

**File:** `src/core/ports/event-bus.ts:28`, `src/core/domain/entities.ts:19-37`

`EventFilter` has `minSeverity?: 'info' | 'warning' | 'error'` but `DomainEvent` is a discriminated union with no `severity` field. Events like `CodeGenerated` or `LintPassed` have no inherent severity.

- **Interpretation A:** The adapter maps event types to severity levels internally.
- **Interpretation B:** `minSeverity` is unimplementable and dead code.

The NULL_EVENT_BUS in composition-root.ts implements `getHistory()` to return `[]`, so this filter is never evaluated today. But any real IEventBusPort implementation would have to invent a severity mapping.

**Severity:** HIGH -- the filter contract is unimplementable without an undocumented mapping table.

**Resolution:** Either add a `severity` field to `DomainEvent`, define a canonical `EVENT_SEVERITY_MAP`, or remove `minSeverity` from `EventFilter`.

---

### 2.7 IEventBusPort.publish is async, handlers can be sync or async -- no delivery guarantees

**File:** `src/core/ports/event-bus.ts:40-66`

`publish(event)` returns `Promise<void>`. Handlers are typed `(event) => void | Promise<void>`. The contract does not specify:
- Does `publish` wait for all handlers to complete before resolving?
- If a handler throws, does `publish` reject? Are other handlers still called?
- Is delivery ordered?

The EventBusNotifier catches handler errors silently (line 59). Other implementations might not.

- **Interpretation A:** `publish` resolves after all handlers complete. Handler errors are swallowed.
- **Interpretation B:** `publish` is fire-and-forget. Handler errors propagate.

**Severity:** HIGH -- an adapter that awaits async handlers would block the publish call. An adapter that doesn't would lose errors. Two correct-looking implementations would behave incompatibly.

**Resolution:** Document delivery semantics explicitly: "publish MUST call all handlers. publish MUST NOT reject due to handler errors. publish MAY return before async handlers complete." Or choose at-least-once with error isolation.

---

### 2.8 WorktreeAdapter.merge operates on the main repo, not the worktree

**File:** `src/adapters/secondary/worktree-adapter.ts:33-46`

`merge(worktree, target)` runs `git checkout target` and then `git merge worktree.branch` in `this.repoPath` (the main repo). This switches the main repo's HEAD to `target`, which would disrupt any concurrent work in the main worktree.

- **Interpretation A:** Merge happens in an isolated context.
- **Interpretation B:** Merge happens in the main repo (current behavior).

**Severity:** HIGH -- in a multi-agent swarm where multiple worktrees exist, calling `merge` from any agent would `checkout` the main repo's HEAD, potentially corrupting another agent's in-progress work.

**Resolution:** Merge should either operate within the worktree directory itself, or use `git merge --no-checkout` / bare repo operations.

---

## 3. MEDIUM Findings

### 3.1 ISwarmPort vs ISwarmOrchestrationPort -- overlapping responsibilities, unclear boundary

**File:** `src/core/ports/swarm.ts:67-113`

`ISwarmPort.spawnAgent` spawns a single agent. `ISwarmOrchestrationPort.orchestrate` plans and executes using agents. When should a caller use `spawnAgent` directly vs `orchestrate`?

- Can `orchestrate` be called multiple times?
- Does `orchestrate` use `ISwarmPort` internally, or is it independent?
- `getProgress()` returns `SwarmStatus & { tasks; agents }` -- is this the same data as `ISwarmPort.status()` plus lists?

**Severity:** MEDIUM -- unclear composition relationship. An implementer of `ISwarmOrchestrationPort` doesn't know if they should delegate to `ISwarmPort`.

**Resolution:** Document that `ISwarmOrchestrationPort` is a higher-level port that composes `ISwarmPort`. Or merge them.

---

### 3.2 QualityScore.score formula is undocumented and would diverge across implementations

**File:** `src/core/domain/entities.ts:51-57`

```typescript
const lintPenalty = this.lintErrorCount * 10 + this.lintWarningCount * 2;
const testScore = this.testsPassed / Math.max(1, this.testsPassed + this.testsFailed);
const efficiency = Math.min(1, this.tokenEfficiency * 5);
return Math.max(0, Math.min(100, testScore * 60 + efficiency * 20 + Math.max(0, 20 - lintPenalty)));
```

This is a concrete class, not an interface, so there is only one implementation today. However:
- The formula weights (60/20/20) are magic numbers.
- `tokenEfficiency * 5` means a ratio of 0.2 (20%) yields a perfect efficiency score. Why 20%?
- The NotificationOrchestrator tracks quality scores as `100` for passed tests and `Math.max(0, Math.round((1 - failures * 10) * 100))` for failures -- a completely different scale than `QualityScore.score()`.

**Severity:** MEDIUM -- two different quality scoring systems coexist. `QualityScore.score()` produces 0-100 with a weighted formula. The orchestrator produces 0-100 with `1 - failures * 10`. They would disagree on the same input.

**Resolution:** Use `QualityScore` consistently in the orchestrator. Document the formula.

---

### 3.3 DomainEvent is a closed union -- adding events requires modifying entities.ts

**File:** `src/core/domain/entities.ts:19-37`

`DomainEvent` is a discriminated union of 15 specific event types. Adding a new event type (e.g., `SecurityScanCompleted`) requires modifying this type, which breaks the open/closed principle and potentially all existing subscribers.

`IEventBusPort.subscribe` is generic over `DomainEvent['type']`, so TypeScript would flag any subscriber referencing a removed event. But adapters that use string-based filtering (e.g., event bus notifier's wildcard patterns) would not get type safety.

**Severity:** MEDIUM -- adding cross-language events, security events, or custom adapter events requires touching the domain core.

**Resolution:** Consider a registry pattern or make DomainEvent extensible via module augmentation.

---

### 3.4 IServiceMeshPort.subscribe unsubscribe function -- sync or async?

**File:** `src/core/ports/cross-lang.ts:213`

`subscribe<T>(...): Promise<() => void>` -- the returned cleanup function returns `void`, not `Promise<void>`. For a gRPC stream or NATS subscription, cleanup likely involves async operations (closing connections, sending unsubscribe messages).

- **Interpretation A:** Unsubscribe is synchronous (just removes local handler).
- **Interpretation B:** Unsubscribe needs to be async (network cleanup).

**Severity:** MEDIUM -- if the real cleanup is async, callers have no way to await completion, leading to resource leaks or race conditions.

**Resolution:** Change return type to `Promise<() => Promise<void>>` or `Promise<Disposable>`.

---

### 3.5 WorkplanStep.dependencies are string IDs with no referential integrity

**File:** `src/core/ports/index.ts:64-70`

`dependencies: string[]` contains step IDs. `TaskGraph.getReady()` checks if dependencies exist in the map but treats "exists" as "completed" (with a comment: "In real impl, check completion status"). `topologicalSort` silently skips missing dependencies.

- **Interpretation A:** All dependency IDs MUST exist in the same Workplan.
- **Interpretation B:** Dependencies can reference external/previous workplan steps.

**Severity:** MEDIUM -- a typo in a dependency ID would cause `getReady()` to return the step as ready (since the dependency "exists" check passes vacuously when the dep is missing and `getReady` only checks that `dep !== undefined`... wait, actually it checks `this.steps.get(depId)` returns non-undefined, so a missing dep would return `undefined` and the step would NOT be ready). However, `topologicalSort` would skip the missing dep via `if (!step) return`, causing the dependent step to appear in the sort order as if it had no dependency.

**Resolution:** Validate referential integrity in `Workplan` construction. Throw on dangling dependency IDs.

---

### 3.6 IBuildPort.compile/lint/test accept Project but BuildAdapter ignores Project.language

**File:** `src/core/ports/index.ts:187-191`, `src/adapters/secondary/build-adapter.ts:42-75`

The `Project` has a `language` field, but `BuildAdapter` always runs `tsc`, `eslint`, and `bun test` regardless of `project.language`. If `project.language` is `'go'` or `'rust'`, the adapter would fail trying to run TypeScript tools on non-TypeScript code.

- **Interpretation A:** Each language needs its own IBuildPort adapter.
- **Interpretation B:** A single adapter should dispatch based on `project.language`.

**Severity:** MEDIUM -- the port signature suggests language-agnostic build support. The adapter only supports TypeScript. No error or guard for non-TS projects.

**Resolution:** Either guard on `project.language` and throw `UnsupportedLanguageError`, or document that this adapter is TypeScript-only.

---

### 3.7 IWASMBridgePort.call uses SerializedPayload[] for args -- WASM functions take primitives

**File:** `src/core/ports/cross-lang.ts:158`

`call<T>(moduleName, functionName, args: SerializedPayload[])` -- WASM exported functions accept i32/i64/f32/f64 primitives and memory pointers. Passing `SerializedPayload[]` (which contains `Uint8Array` data) implies the adapter must handle memory allocation, write the payload into WASM linear memory, and pass pointers. This is a complex protocol that the port interface completely hides.

- **Interpretation A:** The adapter handles all memory management transparently.
- **Interpretation B:** The caller must understand WASM memory layout.

**Severity:** MEDIUM -- the port promises simplicity that no real WASM bridge can deliver without a runtime-specific binding layer (wasm-bindgen, wasm-pack, etc.).

**Resolution:** Document the expected serialization protocol. Consider splitting into `callPrimitive` (direct WASM args) and `callSerialized` (complex types via shared memory).

---

### 3.8 FileSystemAdapter.glob uses Bun.Glob -- not portable

**File:** `src/adapters/secondary/filesystem-adapter.ts:39`

`new Bun.Glob(pattern)` is Bun-specific API. The port `IFileSystemPort.glob` does not specify runtime requirements. Running on Node.js would throw `ReferenceError: Bun is not defined`.

**Severity:** MEDIUM -- the adapter is secretly Bun-only while the port suggests runtime independence.

**Resolution:** Use `node:fs` glob (Node 22+), or a cross-runtime library like `fast-glob`. Document the runtime requirement if Bun-only is intentional.

---

### 3.9 ArchAnalyzer.analyzeArchitecture calls collectSummaries 4 times

**File:** `src/core/usecases/arch-analyzer.ts:187-195`

`analyzeArchitecture` calls `collectSummaries()` once directly (line 188), then calls `buildDependencyGraph`, `findDeadExports`, `validateHexBoundaries`, and `detectCircularDeps` via `Promise.all` -- each of which calls `collectSummaries()` again internally. That is 5 total calls to glob + parse every TS file.

**Severity:** MEDIUM -- O(5n) file reads where O(n) would suffice. For large projects this is a significant performance issue.

**Resolution:** Extract a shared `analyze(summaries)` path that accepts pre-collected summaries.

---

## 4. LOW Findings

### 4.1 IASTPort.extractSummary -- no contract for unsupported languages

The port accepts `filePath: string` and the adapter infers language from extension. If the file is `.py` or `.java`, `detectLanguage` defaults to `'typescript'` and parsing fails silently (returns base summary with no exports).

**Resolution:** Return an error or `null` for unsupported languages.

### 4.2 IEventBusPort event ordering not specified

No guarantee that subscribers receive events in publish order, or that events from concurrent publishers are serialized.

**Resolution:** Document ordering guarantees (or lack thereof).

### 4.3 Notification.id generation -- collision risk

Multiple places generate IDs: `crypto.randomUUID()` in adapters, `notif-${counter}` in orchestrator. The counter-based IDs could collide across orchestrator instances.

**Resolution:** Standardize on UUID everywhere.

### 4.4 INotificationEmitPort.requestDecision -- blocking call with no cancellation

All three implementations (Terminal, Webhook, FileLog) auto-resolve with the default option. The EventBusNotifier blocks for up to `deadline` ms. There is no way to cancel a pending decision from outside.

**Resolution:** Add an `AbortSignal` parameter or return a cancellable handle.

### 4.5 Layer classifier does not handle `composition-root.ts`

`classifyLayer('src/composition-root.ts')` returns `'unknown'` because it matches no pattern. The composition root legitimately imports from both ports and adapters. If it were classified, it would show violations.

**Resolution:** Add a `'composition-root'` layer that is allowed to import from everything.

---

## Summary

| Severity | Count | Key Theme |
|----------|-------|-----------|
| CRITICAL | 4 | Silent false results, dual types, unscoped analysis |
| HIGH | 8 | Missing validation, path traversal, undefined semantics |
| MEDIUM | 9 | Unclear boundaries, portability, performance |
| LOW | 5 | Minor gaps in error handling and documentation |

The most impactful pattern is **silent degradation**: the codebase consistently prefers returning empty/default values over signaling errors (RufloAdapter parse failures, TreeSitter fallback, language detection default). This makes the system appear functional when its infrastructure is broken.
