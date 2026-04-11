# Adversarial Type Safety & API Design Audit

**Date:** 2025-07-14
**Scope:** `src/core/domain/`, `src/core/ports/`, `src/index.ts`, `src/composition-root.ts`, and all adapter files
**tsconfig:** `strict: true`, `noImplicitReturns`, `noFallthroughCasesInSwitch` enabled. `exactOptionalPropertyTypes: false`.

---

## 1. Type Safety Holes

### 1.1 `any` Usage

**No explicit `any` type annotations found in production source.** The word "any" appears only in comments and string literals. This is excellent.

### 1.2 Type Assertions (`as`) -- 67 occurrences

#### CRITICAL: Unsafe casts from `Record<string, unknown>` in adapters

These assertions trust external data (CLI output, HTTP responses, JSON.parse) without runtime validation. A malformed response will not throw -- it will produce silently wrong data that propagates through the system.

| File | Line | Cast | Severity |
|------|------|------|----------|
| `src/adapters/secondary/ruflo-adapter.ts` | 92 | `(result.tasks ?? []) as SwarmTask[]` | **HIGH** -- trusts CLI stdout shape |
| `src/adapters/secondary/ruflo-adapter.ts` | 111 | `(result.agents ?? []) as SwarmAgent[]` | **HIGH** |
| `src/adapters/secondary/ruflo-adapter.ts` | 139 | `(result.results ?? []) as SwarmMemoryEntry[]` | **HIGH** |
| `src/adapters/secondary/ruflo-adapter.ts` | 196-201 | Six casts in `toSwarmStatus()` | **HIGH** -- any field could be wrong type |
| `src/adapters/secondary/llm-adapter.ts` | 127 | `json.content as Array<{ text: string }>` | **HIGH** -- trusts API response |
| `src/adapters/secondary/llm-adapter.ts` | 128 | `json.usage as { input_tokens: ... }` | **HIGH** -- null deref if missing |
| `src/adapters/secondary/llm-adapter.ts` | 135-136 | OpenAI choices/usage casts | **HIGH** |
| `src/adapters/secondary/registry-adapter.ts` | 86 | `JSON.parse(content) as LocalProjectIdentity` | **MEDIUM** -- trusts local file |
| `src/adapters/secondary/registry-adapter.ts` | 111 | `JSON.parse(content) as ProjectRegistry` | **MEDIUM** |
| `src/adapters/secondary/file-log-notifier.ts` | 79 | `notification.context as Record<string, unknown>` | **LOW** -- internal data |
| `src/adapters/secondary/file-log-notifier.ts` | 169 | `JSON.parse(line) as LogEntry` | **MEDIUM** -- trusts log file |
| `src/adapters/primary/mcp-adapter.ts` | 198-233 | ~20 casts of `call.arguments.*` | **HIGH** -- trusts MCP client input |
| `src/adapters/primary/mcp-adapter.ts` | 440-489 | ~15 casts of dashboard query responses | **MEDIUM** |
| `src/adapters/primary/cli-adapter.ts` | 1039-1042 | Casting YAML settings objects | **MEDIUM** |

**Verdict:** The adapters are the weakest link. Every boundary crossing (CLI stdout, HTTP JSON, MCP arguments, file reads) uses blind `as` casts. A single malformed response creates undefined behavior rather than a caught error.

#### Acceptable casts (justified)

| File | Line | Cast | Reason |
|------|------|------|--------|
| `src/core/usecases/arch-analyzer.ts` | 134-142, 297-302 | `fromLayer as DependencyDirection` | Guarded by `classifyLayer()` returning the union; the `'unknown'` case is filtered before these lines |
| `src/core/usecases/import-boundary-checker.ts` | 46-55, 114, 121 | Same pattern | Same guard |
| `src/adapters/secondary/build-adapter.ts` | 93 | `'error' as const` / `'warning' as const` | Narrowing a ternary -- harmless |
| `src/core/usecases/code-generator.ts` | 170, 178, 197 | `'error' as const` etc. | Same |
| `src/adapters/secondary/treesitter-adapter.ts` | 202 | `TS_NODE_KIND_MAP[...] as ExportEntry['kind'] \| undefined` | Correct -- the map may not contain the key |

### 1.3 Non-null Assertions (`!`) -- 6 occurrences

| File | Line | Expression | Severity |
|------|------|------------|----------|
| `src/composition-root.ts` | 107 | `(anthropicKey ?? openaiKey)!` | **LOW** -- guarded by `if (anthropicKey \|\| openaiKey)` on line 105 |
| `src/adapters/secondary/treesitter-adapter.ts` | 179 | `root.child(i)!` | **MEDIUM** -- loop uses `root.childCount` but tree-sitter types are nullable |
| `src/adapters/secondary/treesitter-adapter.ts` | 187 | `exportClause.namedChild(j)!` | **MEDIUM** -- same pattern |
| `src/adapters/secondary/treesitter-adapter.ts` | 223 | `root.child(i)!` | **MEDIUM** |
| `src/adapters/secondary/treesitter-adapter.ts` | 262 | `node.namedChild(i)!` | **MEDIUM** |
| `src/adapters/primary/dashboard-hub.ts` | 139 | `this.projects.get(id)!` | **HIGH** -- only safe if caller checks `has(id)` first; no compile-time guarantee |

### 1.4 `@ts-ignore` / `@ts-expect-error`

**None found.** Clean.

### 1.5 Unsafe `as unknown as` Double-Casts

| File | Line | Expression | Severity |
|------|------|------------|----------|
| `src/adapters/primary/dashboard-adapter.ts` | 116-117 | `null as unknown as never` | **LOW** -- cache invalidation hack, contained |
| `src/core/usecases/notification-orchestrator.ts` | 301 | `progress as unknown as Record<string, unknown>` | **HIGH** -- breaks the type system to mutate a typed object by string key. This is a real bug risk: `stepsCompleted` is not part of `AgentProgress` but gets injected at runtime |

---

## 2. Port Interface Design Review

### 2.1 Missing Branded/Opaque Types

Every `string` parameter that represents a semantic identity is typed as bare `string`:

- **File paths**: `filePath: string` throughout `IFileSystemPort`, `IASTPort`, `ISummaryPort`, `IScaffoldPort`, `IArchAnalysisPort`. A `FilePath` branded type would prevent accidental swaps with other strings.
- **IDs**: `taskId: string`, `agentId: string`, `projectId: string` in `ISwarmPort` and `IRegistryPort`. These are semantically different but type-compatible. Passing a `taskId` where an `agentId` is expected compiles silently.
- **Branch names**: `branchName: string` in `IWorktreePort`. Same issue.
- **Git commit hashes**: `commitHash?: string` in `ISwarmPort.completeTask`. No length or format constraint.

**Recommendation:** Introduce at minimum:
```typescript
type FilePath = string & { readonly __brand: 'FilePath' };
type TaskId = string & { readonly __brand: 'TaskId' };
type AgentId = string & { readonly __brand: 'AgentId' };
```

### 2.2 Error Contracts

**No port interface declares what errors it can throw.** Every method returns `Promise<T>` with no indication of failure modes. Consumers must guess or read adapter source code.

Specific gaps:
- `IFileSystemPort.read()` -- does it throw on missing file or return empty string?
- `ILLMPort.prompt()` -- does it throw on rate limit? On invalid API key? On token budget exceeded?
- `ISwarmPort.init()` -- does it throw if daemon is not running?
- `IGitPort.commit()` -- does it throw if working tree is clean?

**Recommendation:** Use `Result<T, E>` return types or document error types in JSDoc. At minimum, use discriminated unions:
```typescript
type FileReadResult = { ok: true; content: string } | { ok: false; error: 'NOT_FOUND' | 'PERMISSION_DENIED' };
```

### 2.3 Implicit `any` from Untyped Dependencies

- `ISerializationPort.serialize<T>()` and `deserialize<T>()` -- the generic `T` is unconstrained. The caller picks `T` but there is no runtime validation that the deserialized value actually matches. This is `any` hiding behind a generic.
- `IWASMBridgePort.call<T>()`, `IFFIPort.call<T>()`, `IServiceMeshPort.call<T>()` -- same pattern. The return type `T` is a lie unless the adapter validates it.
- `ISchemaPort.validate<T>()` -- takes `value: T` but the generic provides no compile-time safety; the schema name is a runtime string.
- `NotificationAction.payload?: Record<string, unknown>` -- this is essentially `any` with extra steps.
- `INotificationEmitPort.registerChannel(channel, config?: Record<string, unknown>)` -- the `config` parameter is untyped.

### 2.4 Port Design Quality Summary

| Port | Invariant Expression | Error Contract | Branded IDs | Score |
|------|---------------------|----------------|-------------|-------|
| `IASTPort` | Good (level union) | None | No FilePath | 5/10 |
| `ILLMPort` | Good (TokenBudget) | None | N/A | 5/10 |
| `IBuildPort` | Good | None | N/A | 6/10 |
| `IFileSystemPort` | Minimal | None | No FilePath | 3/10 |
| `IGitPort` | Minimal | None | No BranchName | 3/10 |
| `IWorktreePort` | Good (WorktreePath) | None | N/A | 6/10 |
| `ISwarmPort` | Good (union types) | None | No TaskId/AgentId | 5/10 |
| `IEventBusPort` | Excellent (generics) | None | N/A | 7/10 |
| `INotificationEmitPort` | Good (Omit<>) | None | N/A | 6/10 |
| `INotificationQueryPort` | Good | None | N/A | 6/10 |
| `IRegistryPort` | Good | None | No ProjectId | 5/10 |
| `IArchAnalysisPort` | Good | None | No FilePath | 5/10 |
| `IValidationPort` | Excellent | None | N/A | 7/10 |
| `IScaffoldPort` | Excellent | None | No FilePath | 7/10 |
| `ISerializationPort` | Weak (unconstrained T) | None | N/A | 3/10 |
| `IWASMBridgePort` | Weak (unconstrained T) | None | N/A | 3/10 |
| `IFFIPort` | Weak (unconstrained T) | None | N/A | 3/10 |
| `IServiceMeshPort` | Weak (unconstrained T) | None | N/A | 3/10 |
| `ISchemaPort` | Weak (unconstrained T) | None | N/A | 3/10 |

---

## 3. Value Object Robustness

### 3.1 Immutability

**All value objects in `value-objects.ts` are plain interfaces** -- they have no class wrapper, no `readonly` modifier on fields, and no constructor validation. Any consumer can mutate them after creation:

```typescript
const summary: ASTSummary = getSummary();
summary.exports.push({ name: 'injected', kind: 'function' }); // No error
summary.lineCount = -1; // No error
summary.level = 'L99' as any; // No error (but would require `any`)
```

**Fields that should be `readonly`:**
- Every field on every interface in `value-objects.ts` (all 40+ interfaces)
- `ASTSummary.exports` and `ASTSummary.imports` arrays -- should be `readonly ExportEntry[]`
- `Workplan.steps` -- should be `readonly WorkplanStep[]`

### 3.2 Construction Validation

**No value object validates on construction.** They are bare interfaces, so there is no construction-time check for:
- `TokenBudget.available` being non-negative or <= `maxTokens`
- `TokenBudget.available` equaling `maxTokens - reservedForResponse`
- `LintError.line` and `LintError.column` being positive integers
- `ASTSummary.level` actually being one of the four valid values when created at runtime
- `ProjectRegistration.port` being in a valid range (1024-65535)
- `QualityScore` fields being non-negative

### 3.3 Domain Entity Classes

`QualityScore`, `FeedbackLoop`, and `TaskGraph` are classes, which is better:

- **QualityScore**: Uses `readonly` constructor parameters -- good. But no validation: `lintErrorCount` can be negative, `tokenEfficiency` can be NaN. The `score` getter clamps to 0-100 but does not protect against NaN inputs.
- **FeedbackLoop**: `iterations` array is `private` -- good encapsulation. `maxIterations` is `readonly` -- good. But `maxIterations` accepts 0 or negative values, which would make `canRetry` always false (arguably correct but confusing).
- **TaskGraph**: Mutable by design (steps added incrementally). `topologicalSort()` does not detect cycles -- it silently produces a partial ordering if cycles exist. The `getReady()` method comment says "In real impl, check completion status" -- this is an incomplete implementation.

### 3.4 Domain Error Types

`DomainError`, `ValidationError`, `InvariantViolation`, `BoundaryViolation` are well-structured with readonly fields and proper inheritance. This is one of the strongest parts of the type design.

However, `DomainError.code` is `string` rather than a discriminated union, so exhaustive matching on error codes is not possible.

---

## 4. Public API Surface (`src/index.ts`)

### 4.1 What Is Exported

- 10 port interfaces (type-only) -- correct
- 10 value types (type-only) -- correct
- 3 domain entity classes (`QualityScore`, `FeedbackLoop`, `TaskGraph`) -- these are value exports, correctly included
- `createAppContext` factory function -- correct
- `AppContext` type -- correct

### 4.2 Issues

1. **`AppContext` is re-exported from `composition-root.ts`** (line 54) rather than from `core/ports/app-context.ts`. The canonical definition is in the port file, but the public API imports from the composition root. If the composition root ever adds fields not in the port definition, they leak.

2. **Missing exports for downstream use:**
   - `DomainEvent` type is not exported. Consumers who subscribe to `IEventBusPort` cannot type their handlers.
   - `DomainError`, `ValidationError`, `InvariantViolation`, `BoundaryViolation` -- the error hierarchy is not exported. Consumers cannot catch specific domain errors.
   - `Notification`, `DecisionRequest`, `DecisionResponse` -- partially exported (Notification is), but `DecisionRequest` and `DecisionResponse` are not, making `INotificationQueryPort.getPendingDecisions()` and `respondToDecision()` unusable from the public API.
   - `SwarmAgent`, `SwarmMemoryEntry`, `AgentRole` -- not exported, but `SwarmTask` is. Inconsistent.

3. **Leaking internal types:** `LLMAdapterConfig` is exported from `composition-root.ts` import chain but not from `index.ts`. This is correct -- it should stay internal.

4. **No `Readonly` wrapper on re-exported interfaces.** Consumers can mutate returned objects.

---

## 5. Exhaustiveness Analysis

### 5.1 Switch Statements on Discriminated Unions

#### `src/core/usecases/notification-orchestrator.ts:172` -- `switch (event.type)`

Handles: `CodeGenerated`, `LintPassed`, `LintFailed`, `TestsPassed`, `TestsFailed`, `BuildSucceeded`, `BuildFailed`, `WorkplanCreated`, `StepCompleted`, `SwarmSpawned`.

**Missing:** `DecisionRequested`, `DecisionResolved`, `AgentStalled`, `QualityRegressed`, `PhaseCompleted`.

Has `default` clause (line 261) that returns a trace notification -- so it degrades gracefully. But adding a new event type will silently go to `default` with no compiler warning. **No `never` exhaustiveness check.**

#### `src/core/usecases/notification-orchestrator.ts:282` -- `switch (event.type)`

Handles: `CodeGenerated`, `LintPassed`, `LintFailed`, `TestsPassed`, `TestsFailed`, `BuildSucceeded`, `BuildFailed`, `StepCompleted`.

**Missing:** 9 event types. No default clause, but the function does not return, so TypeScript does not enforce exhaustiveness.

#### `src/core/usecases/notification-orchestrator.ts:321` -- `switch (tracked.progress.status)`

Handles all 5 AgentProgress status values (`done`, `failed`, `queued`, `running`, `blocked`). **Exhaustive.** Good.

#### `src/core/usecases/status-formatter.ts:325` -- `switch (agent.status)`

Same 5 values. **Exhaustive.** Good.

#### `src/adapters/primary/mcp-adapter.ts:195` -- `switch (call.name)`

Handles all registered tool names plus `default`. Not a discriminated union -- `call.name` is `string`. The default returns an error. Acceptable.

#### `src/adapters/primary/mcp-adapter.ts:426,445` -- `switch (query)`

Handles `health`, `tokens`, `swarm`, `graph` plus `default`. Not a discriminated union. Acceptable.

#### `src/adapters/primary/cli-adapter.ts:279` -- `switch (args.command)`

Has a default clause. Not a discriminated union (argv string). Acceptable.

### 5.2 `DependencyDirection` Union Coverage

The `DependencyDirection` type is:
```typescript
'domain' | 'ports' | 'usecases' | 'adapters/primary' | 'adapters/secondary' | 'infrastructure'
```

`ALLOWED_IMPORTS` in `layer-classifier.ts` covers all 6 values as keys -- **exhaustive**. Good.

`classifyLayer()` returns `DependencyDirection | 'unknown'` -- the `'unknown'` case is handled by callers before casting. However, if a 7th layer is added to `DependencyDirection`, `classifyLayer()` will return `'unknown'` for it with no compiler error.

### 5.3 `Language` Union Coverage

```typescript
type Language = 'typescript' | 'go' | 'rust';
```

Adding a new language (e.g., `'python'`) would silently break:
- `TreeSitterAdapter` grammar loading (no exhaustiveness check)
- `BuildAdapter` compile/lint/test dispatch
- `LLMAdapter` is language-agnostic (safe)
- `DEFAULT_MODELS` in llm-adapter (safe -- keyed by provider, not language)

---

## 6. Null Safety

### 6.1 `AppContext` Nullable Fields

`AppContext` has 5 nullable fields: `llm`, `codeGenerator`, `workplanExecutor`, `notificationOrchestrator`, `eventBus`. TypeScript enforces null checks on these. The CLI adapter correctly checks for null before using `codeGenerator` and `workplanExecutor`. **Good.**

### 6.2 Potential Null Dereferences

| Location | Risk |
|----------|------|
| `llm-adapter.ts:130` | `content.map(c => c.text)` -- if `json.content` is null/undefined, the `as` cast makes it `null` typed as `Array<>`, causing a runtime crash on `.map()` |
| `llm-adapter.ts:138` | `choices[0]?.message.content` -- safe due to optional chaining, but `usage` on line 136 has no null guard and will crash if missing |
| `ruflo-adapter.ts:127` | `result.value ?? null` -- safe |
| `dashboard-hub.ts:139` | `this.projects.get(id)!` -- will throw if project was unregistered between `has()` check and `get()` call (race condition) |
| `notification-orchestrator.ts:301-302` | After double-cast, `p['stepsCompleted']` access is unguarded -- if the property does not exist, `as number` on `undefined` yields `NaN`, then `NaN + 1 = NaN` is stored |

### 6.3 `exactOptionalPropertyTypes: false`

This tsconfig setting means `undefined` can be assigned to optional properties. With it enabled, `{ field?: string }` would reject `{ field: undefined }`. Enabling this would catch ~5-10 additional bugs where `undefined` is explicitly passed instead of omitting the property.

---

## 7. Summary of Findings

### Severity Distribution

| Severity | Count | Category |
|----------|-------|----------|
| **CRITICAL** | 3 | Unsafe `as` casts on external data (LLM, ruflo, MCP adapters) |
| **HIGH** | 5 | Missing runtime validation at system boundaries, `as unknown as` double-cast, non-null assertion in dashboard-hub |
| **MEDIUM** | 8 | Missing branded types, tree-sitter non-null assertions, YAML/JSON parse casts |
| **LOW** | 6 | Cache invalidation hacks, internal data casts, justified non-null assertions |
| **INFO** | 4 | Missing public API exports, no `readonly` on interfaces, incomplete TaskGraph |

### Top 5 Recommendations (by impact-to-effort ratio)

1. **Add runtime validation at adapter boundaries.** Create a `validate<T>(schema, data): T` helper using a lightweight schema library (e.g., Zod, Valibot, or hand-rolled type guards). Apply to: `ruflo-adapter.ts`, `llm-adapter.ts`, `registry-adapter.ts`, `mcp-adapter.ts`. This prevents the ~20 highest-severity `as` casts from being silent data corruption vectors.

2. **Add `readonly` to all value object interface fields.** This is a mechanical change (`readonly filePath: string` etc.) with zero runtime cost. Prevents accidental mutation of returned data.

3. **Export missing types from `src/index.ts`.** Add: `DomainEvent`, `DomainError`, `ValidationError`, `InvariantViolation`, `BoundaryViolation`, `DecisionRequest`, `DecisionResponse`, `SwarmAgent`, `AgentRole`. Without these, the public API is incomplete.

4. **Add exhaustiveness checks to event type switches.** In `notification-orchestrator.ts`, add a `default: { const _exhaustive: never = event; }` guard. This ensures new `DomainEvent` variants get handled at compile time.

5. **Fix the `as unknown as Record<string, unknown>` double-cast** in `notification-orchestrator.ts:301`. Add `stepsCompleted` as an optional field to `AgentProgress` instead of hacking it in via type erasure.

### What the Codebase Does Well

- Zero `any` annotations in production code
- Zero `@ts-ignore` / `@ts-expect-error` directives
- `strict: true` with additional strictness flags enabled
- Clean port/adapter separation with types flowing in the correct direction
- Well-designed discriminated union for `DomainEvent`
- Proper `readonly` on class fields in domain entities
- Null-aware `AppContext` design (nullable LLM, codeGenerator, etc.)
- Domain error hierarchy with typed codes
- Re-export strategy keeps value objects owned by domain, consumed via ports
