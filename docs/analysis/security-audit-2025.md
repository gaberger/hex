# hex Adversarial Security Audit

**Date**: 2025-03-15
**Scope**: All files under `src/` -- adapters, composition root, CLI, use cases
**Methodology**: Manual adversarial review targeting security, error handling, failure modes

---

## Finding 1: Path Traversal in FileSystemAdapter

**Severity**: CRITICAL
**File**: `src/adapters/secondary/filesystem-adapter.ts:47-49`

```typescript
private resolve(filePath: string): string {
  return join(this.root, filePath);
}
```

`path.join` does NOT prevent traversal. `join('/project', '../../etc/passwd')` resolves to `/etc/passwd`. There is zero sanitization. The `write()` method at line 23-27 will also `mkdir -p` arbitrary directories before writing, amplifying the impact.

**Proof of concept**: Any caller passing `../../etc/cron.d/backdoor` to `write()` creates the directory and writes the file.

**The CLAUDE.md claims "path sanitization" -- this claim is FALSE.** No `resolve`+`startsWith` check exists anywhere in this file.

**Fix**: After `join`, call `resolve()` and assert `resolvedPath.startsWith(this.root)`.

---

## Finding 2: Silent Failure Swallowing in RufloAdapter (x6 instances)

**Severity**: HIGH
**File**: `src/adapters/secondary/ruflo-adapter.ts`

| Line | Pattern | What is lost |
|------|---------|-------------|
| 98-100 | `memoryRetrieve` catch returns `null` | Cannot distinguish "key not found" from "CLI crashed" |
| 109-111 | `memorySearch` catch returns `[]` | Broken CLI looks like empty results |
| 131-140 | `parseStatus` catch returns fake idle status | Broken swarm reports as healthy idle |
| 144 | `parseTasks` catch returns `[]` | Task list failures invisible |
| 148 | `parseAgents` catch returns `[]` | Agent list failures invisible |
| 123-126 | `extractId` fallback to `hex-${Date.now()}` | CLI returns garbage, adapter invents an ID |

The `parseStatus` fallback at line 128-140 is the worst: when the ruflo CLI is genuinely broken, `status()` returns `{ status: 'idle', agentCount: 0 }`. Any monitoring code will conclude the swarm is healthy and idle when it is actually unreachable. This is a **silent correctness failure**.

**Fix**: Distinguish parse errors from empty results. Throw on non-JSON stdout when JSON was expected. Use a dedicated `RufloUnreachableError`.

---

## Finding 3: Tree-Sitter Stub Fallback Produces False Health Reports

**Severity**: HIGH
**File**: `src/composition-root.ts:78-89`

When tree-sitter initialization fails, the stub AST adapter returns `{ exports: [], imports: [], lineCount: 0 }` for every file. The `ArchAnalyzer` then computes:
- 0 dead exports (nothing is exported)
- 0 violations (no imports to violate)
- 0 circular deps (no edges)
- Health score: **100/100**

This means `hex analyze` will report **perfect architecture health** when the analyzer is completely non-functional. A user running this in CI would get a green check on a broken codebase.

**Fix**: The stub should throw or the ArchAnalyzer should detect when all summaries have 0 exports and flag it as suspicious. At minimum, log a warning that tree-sitter failed.

---

## Finding 4: No Timeout on BuildAdapter and GitAdapter Processes

**Severity**: MEDIUM
**File**: `src/adapters/secondary/build-adapter.ts:136-145`, `src/adapters/secondary/git-adapter.ts:47-63`

RufloAdapter has a 30s timeout. BuildAdapter and GitAdapter have **no timeout at all**. A `tsc --noEmit` on a large project, or a `git diff` on a huge repo, can hang indefinitely.

```typescript
// build-adapter.ts:141 -- no timeout
return execFile(cmd, args, { cwd: cwd ?? this.projectPath, maxBuffer: 10 * 1024 * 1024 });

// git-adapter.ts:51 -- no timeout
return await execFile('git', args, { cwd: this.repoPath, maxBuffer: 10 * 1024 * 1024 });
```

**Fix**: Add `timeout: 120_000` (or configurable) to both adapters' execFile options.

---

## Finding 5: Worktree Leak on Process Crash

**Severity**: MEDIUM
**File**: `src/adapters/secondary/worktree-adapter.ts`

`create()` at line 27-30 creates a git worktree on disk. `cleanup()` at line 49-51 removes it. There is no `finally` block, no shutdown hook, no cleanup-on-init reconciliation. If the process crashes between `create()` and `cleanup()`, the worktree persists on disk forever.

The `worktreeDir` is set in composition-root.ts:68 to `${projectPath}/../hex-worktrees` -- a sibling directory outside the project. Accumulated leaked worktrees will consume disk and pollute git state.

**Fix**: On startup, call `list()` and remove any worktrees with the `hex-` prefix that do not match active tasks. Register a `process.on('exit')` cleanup handler.

---

## Finding 6: setInterval Leak in NotificationOrchestrator

**Severity**: MEDIUM
**File**: `src/core/usecases/notification-orchestrator.ts:94-101`

```typescript
start(swarmId: string, phase: string): void {
  this.swarmId = swarmId;
  this.currentPhase = phase;
  this.stallCheckTimer = setInterval(...);
}
```

If `start()` is called twice without `stop()`, the first interval handle is overwritten and leaked. The old timer continues firing `checkForStalls()` with stale state.

**Fix**: Call `this.stop()` at the top of `start()`.

---

## Finding 7: Unbounded Notification Array (Memory Leak)

**Severity**: MEDIUM
**File**: `src/core/usecases/notification-orchestrator.ts:61,468`

```typescript
private readonly notifications: Notification[] = [];
// ...
this.notifications.push(notification);  // line 468 -- never trimmed
```

Every notification is appended and never evicted. In a long-running swarm with thousands of events, this grows without bound. `getRecent()` slices from the end but never truncates the array.

**Fix**: Cap at a configurable maximum (e.g., 10,000) and evict oldest entries, or use a ring buffer.

---

## Finding 8: Unbounded qualityHistory Array per Agent

**Severity**: LOW
**File**: `src/core/usecases/notification-orchestrator.ts:51,298-303`

`tracked.qualityHistory` is pushed to on every test event and never trimmed (except on a `reset` decision). With many iterations this grows unbounded per agent.

**Fix**: Cap at a rolling window (e.g., last 20 entries).

---

## Finding 9: NULL_EVENT_BUS Silently Discards All Events

**Severity**: MEDIUM
**File**: `src/composition-root.ts:53-60`

```typescript
const NULL_EVENT_BUS: IEventBusPort = {
  async publish() {},
  subscribe() { return { id: 'noop', unsubscribe() {} }; },
  // ...
};
```

This is used as the **production default** (line 104). Any use case that publishes domain events for cross-cutting concerns (audit logging, integration triggers, swarm coordination) will silently lose all events. The NotificationOrchestrator does NOT use the event bus -- it has its own `handleEvent` method -- so there is no bridge. Events published to the bus go nowhere.

**Fix**: Either wire up the event bus to a real implementation or document clearly that it is a no-op and no code should depend on it for correctness.

---

## Finding 10: CLI Adapter Missing Input Validation at Boundary

**Severity**: MEDIUM
**File**: `src/adapters/primary/cli-adapter.ts`

The `analyze` command at line 121 passes `args.positional[0] ?? '.'` directly to `archAnalyzer.analyzeArchitecture()`. The `summarize` command at line 192 passes `filePath` directly to `ast.extractSummary()`. Neither validates:
- Path existence before passing to the port
- Path characters (null bytes, newlines)
- Whether the path points outside the project

The `--level` flag IS validated (line 185-189), which shows the pattern was understood but not applied consistently.

**Fix**: Validate and sanitize all path inputs at the CLI boundary before passing to domain ports.

---

## Finding 11: WorktreeAdapter Branch Name Injection

**Severity**: LOW
**File**: `src/adapters/secondary/worktree-adapter.ts:27-30`

```typescript
async create(branchName: string): Promise<WorktreePath> {
  const absolutePath = this.worktreePath(branchName);
  await this.git('worktree', 'add', absolutePath, '-b', branchName);
```

`branchName` is passed directly as an argument to `git`. While `execFile` prevents shell injection, a branch name containing spaces or starting with `-` could cause `git` to misinterpret arguments (argument injection). For example, `branchName = "--detach"` would be parsed as a flag by git.

**Fix**: Validate branch names against `git check-ref-format` rules or prefix with `refs/heads/`.

---

## Finding 12: RufloAdapter npx Fetches `@latest` on Every Invocation

**Severity**: LOW
**File**: `src/adapters/secondary/ruflo-adapter.ts:25`

```typescript
const CLI_PKG = '@claude-flow/cli@latest';
```

Every call to `run()` invokes `npx @claude-flow/cli@latest`, which may fetch the latest version from npm on each execution. This is a supply-chain risk (a compromised package version gets auto-pulled) and a performance problem (network round-trip on every CLI call).

**Fix**: Pin to a specific version or use a locally installed binary.

---

## Summary Table

| # | Finding | Severity | File |
|---|---------|----------|------|
| 1 | Path traversal -- no sanitization | CRITICAL | filesystem-adapter.ts:47 |
| 2 | Silent failure swallowing (x6) | HIGH | ruflo-adapter.ts (multiple) |
| 3 | Stub fallback reports 100% health | HIGH | composition-root.ts:78 |
| 4 | No timeout on build/git processes | MEDIUM | build-adapter.ts:141, git-adapter.ts:51 |
| 5 | Worktree leak on crash | MEDIUM | worktree-adapter.ts |
| 6 | setInterval leak on double-start | MEDIUM | notification-orchestrator.ts:94 |
| 7 | Unbounded notification array | MEDIUM | notification-orchestrator.ts:61 |
| 8 | Unbounded qualityHistory per agent | LOW | notification-orchestrator.ts:51 |
| 9 | NULL_EVENT_BUS discards all events | MEDIUM | composition-root.ts:53 |
| 10 | Missing CLI input validation | MEDIUM | cli-adapter.ts:121,192 |
| 11 | Branch name argument injection | LOW | worktree-adapter.ts:27 |
| 12 | npx @latest supply-chain risk | LOW | ruflo-adapter.ts:25 |

**CRITICAL: 1 | HIGH: 2 | MEDIUM: 6 | LOW: 3**
