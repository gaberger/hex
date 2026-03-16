# Hexagonal Boundary Purity Review — hex

**Reviewer**: Adversarial Architecture Reviewer
**Date**: 2025-03-15
**Scope**: All source files in `src/`

---

## CRITICAL Findings

### C1. Domain Entities Import from Ports — Dependency Inversion Violation
**Severity**: CRITICAL
**File**: `src/core/domain/entities.ts:8-15`
**Evidence**:
```typescript
import type { Language, CodeUnit, LintError, BuildResult, TestResult, WorkplanStep } from '../ports/index.js';
```
**Analysis**: The file's own docstring claims "Pure domain objects with no external dependencies." This is false. Domain entities depend on port-layer value objects (`CodeUnit`, `LintError`, `BuildResult`, `TestResult`, `WorkplanStep`). In strict hexagonal architecture, the domain is the innermost ring and must have ZERO outward dependencies — not even to ports. Value objects used by domain entities should be defined IN the domain layer, and ports should import FROM the domain, not the reverse. The `layer-classifier.ts` rule table confirms this is self-contradictory: it allows `domain -> ports` AND `ports -> domain`, creating a bidirectional dependency between the two innermost layers.

**Impact**: The entire dependency direction model is compromised. The domain cannot be extracted or tested without bringing the ports layer along.

**Recommendation**: Move shared value objects (`Language`, `CodeUnit`, `LintError`, `BuildResult`, `TestResult`, `WorkplanStep`) into `src/core/domain/value-objects.ts`. Have both ports and entities import from domain. Alternatively, acknowledge that ports + domain form a single "core" module and document this deviation.

---

### C2. Event Bus Port Creates a Circular Dependency: ports <-> domain
**Severity**: CRITICAL
**File**: `src/core/ports/event-bus.ts:14`
**Evidence**:
```typescript
import type { DomainEvent } from '../domain/entities.js';
```
**Analysis**: `event-bus.ts` (ports layer) imports `DomainEvent` from `entities.ts` (domain layer), while `entities.ts` imports value objects from `ports/index.ts`. This creates a circular dependency cycle: `domain -> ports -> domain`. The layer-classifier's own rules say `ports` may import from `domain` (line 22) and `domain` may import from `ports` (line 21), which means the rule engine explicitly permits this cycle — a flaw in the rule table itself.

**Impact**: The self-analysis tool (`ArchAnalyzer`) would report this project as healthy while harboring a foundational cycle. The dog is not eating its own dogfood correctly.

---

### C3. Composition Root Exposes Concrete Type in AppContext
**Severity**: CRITICAL
**File**: `src/composition-root.ts:39`
**Evidence**:
```typescript
notificationOrchestrator: NotificationOrchestrator;
```
**Analysis**: `AppContext` exposes the concrete class `NotificationOrchestrator` instead of the port interface `INotificationQueryPort`. Every consumer of `AppContext` now has a compile-time dependency on the use-case implementation. This defeats the purpose of the composition root pattern — consumers should only see port interfaces.

**Impact**: Any code receiving `AppContext` can call `NotificationOrchestrator`-specific methods (`start()`, `stop()`, `registerAgent()`, `handleEvent()`, `markAgentDone()`, `markAgentFailed()`) that are NOT part of `INotificationQueryPort`. This leaks use-case internals through the composition root.

---

## HIGH Findings

### H1. CLI Adapter Defines a Shadow AppContext Interface
**Severity**: HIGH
**File**: `src/adapters/primary/cli-adapter.ts:18-23`
**Evidence**:
```typescript
export interface AppContext {
  rootPath: string;
  archAnalyzer: IArchAnalysisPort;
  ast: IASTPort;
  fs: IFileSystemPort;
}
```
**Analysis**: This is a structurally different type from `composition-root.ts`'s `AppContext`. While TypeScript structural typing means the full `AppContext` is assignable to the CLI's narrower one, this creates a contract ambiguity: two exported types named `AppContext` in the same project with different shapes. The CLI's version omits `git`, `worktree`, `build`, `eventBus`, `notifier`, `swarm`, and `notificationOrchestrator`. If a consumer imports the wrong `AppContext`, they get a silently incompatible type.

**Impact**: Refactoring either `AppContext` can break the other without compiler warnings if they diverge in non-structural ways.

**Recommendation**: The CLI adapter should import the canonical `AppContext` from composition-root (via port re-export or a shared types file), or use `Pick<AppContext, 'rootPath' | 'archAnalyzer' | 'ast' | 'fs'>`.

---

### H2. cli.ts Imports Directly from Adapter — Bypasses Composition Root Boundary
**Severity**: HIGH
**File**: `src/cli.ts:7-8`
**Evidence**:
```typescript
import { createAppContext } from './composition-root.js';
import { CLIAdapter } from './adapters/primary/cli-adapter.js';
```
**Analysis**: `cli.ts` is not inside the `adapters/` directory, nor is it the composition root. It imports a concrete adapter class (`CLIAdapter`) directly. In strict hexagonal architecture, only the composition root should know about concrete adapters. `cli.ts` acts as a second composition point, which is undocumented.

**Impact**: If the layer-classifier were run on this file, it would classify as `unknown` (not in any hex layer) and skip validation — a blind spot in the project's own analysis tool.

---

### H3. Inline NULL_EVENT_BUS Violates Adapter Boundary
**Severity**: HIGH
**File**: `src/composition-root.ts:53-60`
**Evidence**:
```typescript
const NULL_EVENT_BUS: IEventBusPort = {
  async publish() {},
  subscribe() { return { id: 'noop', unsubscribe() {} }; },
  ...
};
```
**Analysis**: The composition root contains an inline adapter implementation (Null Object pattern). While the composition root is allowed to import adapters, it should not BE an adapter. This null implementation should live in `src/adapters/secondary/null-event-bus.ts` to maintain the single-responsibility of the composition root as a wiring-only file.

**Impact**: When a real event bus adapter is added, the null fallback will remain as dead code inside the composition root, creating maintenance confusion.

---

### H4. Inline Stub AST Adapter in Composition Root
**Severity**: HIGH
**File**: `src/composition-root.ts:79-89`
**Evidence**:
```typescript
ast = {
  async extractSummary(filePath, level) { ... },
  diffStructural() { return { added: [], removed: [], modified: [] }; },
};
```
**Analysis**: Same violation as H3. A fallback IASTPort implementation is defined inline in the composition root's catch block. This is a second adapter implementation living outside the adapters directory.

---

### H5. ISwarmPort Uses Inline Type Import in AppContext
**Severity**: HIGH
**File**: `src/composition-root.ts:48`
**Evidence**:
```typescript
swarm: import('./core/ports/swarm.js').ISwarmPort;
```
**Analysis**: Dynamic `import()` type syntax for `ISwarmPort` while all other ports use standard `import type` at the top of the file. This inconsistency suggests `swarm.ts` was added later and the developer avoided updating the import block. While functionally equivalent, it signals an integration seam that was never cleaned up.

---

### H6. Seven Port Interfaces Have Zero Adapter Implementations
**Severity**: HIGH
**Files**: `src/core/ports/index.ts`, `src/core/ports/cross-lang.ts`, `src/core/ports/notification.ts`
**Evidence**: The following ports are defined but have no corresponding adapter in `src/adapters/secondary/`:
1. `ILLMPort` — declared at `ports/index.ts:182-185`
2. `ICodeGenerationPort` — declared at `ports/index.ts:160-163` (input port, no use case)
3. `IWorkplanPort` — declared at `ports/index.ts:165-168` (input port, no use case)
4. `ISummaryPort` — declared at `ports/index.ts:170-173` (input port, no use case)
5. `ISerializationPort` — declared at `ports/cross-lang.ts:134-146`
6. `IWASMBridgePort` — declared at `ports/cross-lang.ts:153-168`
7. `IFFIPort` — declared at `ports/cross-lang.ts:175-190`
8. `IServiceMeshPort` — declared at `ports/cross-lang.ts:197-220`
9. `ISchemaPort` — declared at `ports/cross-lang.ts:227-245`
10. `INotificationQueryPort` — has a use-case impl but no primary adapter wires to it

**Analysis**: These are phantom ports — they define contracts that nothing fulfills. Input ports (1-4) have no use-case implementations. Output ports (5-9) have no adapter implementations. The architecture promises capabilities it cannot deliver.

**Impact**: Consumers of the npm package (`src/index.ts` exports `ILLMPort`, `ICodeGenerationPort`, etc.) receive type definitions for ports that cannot be instantiated through the composition root.

---

## MEDIUM Findings

### M1. Layer Classifier Allows Bidirectional domain <-> ports
**Severity**: MEDIUM
**File**: `src/core/usecases/layer-classifier.ts:21-22`
**Evidence**:
```typescript
'domain': new Set<DependencyDirection>(['ports']),
'ports':  new Set<DependencyDirection>(['domain']),
```
**Analysis**: This rule set explicitly permits the circular dependency identified in C1/C2. A strict hexagonal model would have ports depend on domain, but NOT domain depend on ports. The tool encodes the project's own violations as "allowed."

---

### M2. FeedbackLoop Entity is Mutable — Not Event-Sourced
**Severity**: MEDIUM
**File**: `src/core/domain/entities.ts:66-100`
**Evidence**: `FeedbackLoop` has a `private iterations: FeedbackIteration[] = []` field mutated by `record()`.

**Analysis**: The CLAUDE.md states "Use event sourcing for state changes." `FeedbackLoop` uses direct mutation, not event sourcing. `TaskGraph` similarly mutates via `addStep()`.

---

### M3. FileSystemAdapter Uses Bun-Specific API
**Severity**: MEDIUM
**File**: `src/adapters/secondary/filesystem-adapter.ts:39`
**Evidence**:
```typescript
const g = new Bun.Glob(pattern);
```
**Analysis**: The `glob()` method uses `Bun.Glob`, which does not exist in Node.js. The port interface `IFileSystemPort.glob()` makes no mention of runtime requirements. Any consumer running on Node.js will get a runtime crash, not a compile-time error.

**Impact**: The port contract is silently narrower than advertised — it only works on Bun.

---

### M4. RufloAdapter Silently Swallows Parse Failures
**Severity**: MEDIUM
**File**: `src/adapters/secondary/ruflo-adapter.ts:128-139, 143-144, 147-148`
**Evidence**: `parseStatus()`, `parseTasks()`, and `parseAgents()` all catch JSON parse errors and return fabricated default objects instead of propagating failures.

**Analysis**: When the CLI returns unexpected output, the adapter returns fake data (e.g., a `SwarmStatus` with `id: swarm-${Date.now()}`). Callers cannot distinguish between a real idle swarm and a parse failure. This violates the principle that adapters should faithfully translate external system responses.

---

### M5. ArchAnalyzer Ignores rootPath Parameter
**Severity**: MEDIUM
**File**: `src/core/usecases/arch-analyzer.ts:65, 82, 114, 142, 187`
**Evidence**: Every public method accepts `rootPath` but uses `_rootPath` (prefixed underscore = unused). The actual file collection uses `this.fs.glob('**/*.ts')` which operates on the `IFileSystemPort`'s configured root.

**Analysis**: The `IArchAnalysisPort` interface declares `rootPath` as a parameter on every method, but the implementation ignores it entirely. This is a contract lie — callers believe they can analyze arbitrary paths, but the analyzer always scans the filesystem adapter's root.

---

### M6. Dead Export Detection Has False Positives for Name Collisions
**Severity**: MEDIUM
**File**: `src/core/usecases/arch-analyzer.ts:86-111`
**Analysis**: The dead export detection builds a flat `Set<string>` of all imported names across ALL files, then checks if each export name appears in that set. This means if file A exports `Language` and file B also exports `Language`, and only B's `Language` is imported anywhere, A's `Language` will be incorrectly marked as alive (the name matches, even though the import is from a different module).

---

## LOW Findings

### L1. `process.stdout` Default in Multiple Adapters
**File**: `src/adapters/primary/cli-adapter.ts:86`, `src/adapters/secondary/terminal-notifier.ts:85`
**Analysis**: Both default to `process.stdout`, coupling to Node/Bun runtime. Minor since these are adapters, but the `WritableOutput` abstraction in terminal-notifier is not shared with the CLI adapter.

### L2. `index.ts` Exports Domain Classes, Not Just Types
**File**: `src/index.ts:43`
**Evidence**: `export { QualityScore, FeedbackLoop, TaskGraph } from './core/domain/entities.js';`
**Analysis**: These are runtime class exports from the domain layer. Consumers can instantiate domain entities directly, bypassing use cases. This is technically fine in hexagonal architecture (domain is the stable core) but exposes implementation details.

### L3. No `ISwarmPort` Re-export from `ports/index.ts`
**File**: `src/core/ports/index.ts` does not re-export from `swarm.ts`
**Analysis**: `ISwarmPort` and all swarm types live in a separate `ports/swarm.ts` file and are never re-exported through the barrel `ports/index.ts`. This forces consumers to know the internal file structure of the ports layer.

### L4. Notification Port Types Not Re-exported from ports/index.ts Either
**File**: `src/core/ports/index.ts`
**Analysis**: Same as L3 — `IEventBusPort`, `INotificationEmitPort`, `INotificationQueryPort` are not available through the barrel export. The ports layer has three separate entry points (`index.ts`, `event-bus.ts`, `notification.ts`, `cross-lang.ts`) instead of one.

---

## Summary Scorecard

| Severity | Count | Findings |
|----------|-------|----------|
| CRITICAL | 3 | C1, C2, C3 |
| HIGH | 6 | H1, H2, H3, H4, H5, H6 |
| MEDIUM | 6 | M1, M2, M3, M4, M5, M6 |
| LOW | 4 | L1, L2, L3, L4 |
| **Total** | **19** | |

## Top 3 Action Items

1. **Resolve the domain <-> ports cycle** (C1, C2, M1): Move shared value objects into `src/core/domain/value-objects.ts`. Update the layer-classifier rules to disallow `domain -> ports`. This is the foundational fix.

2. **Fix composition root type leaks** (C3, H3, H4, H5): Replace `NotificationOrchestrator` with `INotificationQueryPort` in `AppContext`. Extract null/stub adapters to proper adapter files.

3. **Reconcile dual AppContext types** (H1, H2): Either have CLI adapter import from composition-root using `Pick<>`, or define a shared `AppContext` interface in the ports layer.
