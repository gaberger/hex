# Adversarial Resilience Audit -- Error Handling & Silent Failures

**Date**: 2025-03-15
**Auditor**: Error Handling Resilience Tester
**Scope**: All files under `src/` (44 TypeScript source files)
**Methodology**: Manual line-by-line review of every try/catch, fallback, JSON.parse, async boundary, and resource lifecycle

---

## Executive Summary

The codebase has **37 catch blocks** across `src/`. Of these, **21 are bare `catch {` blocks** (no error variable captured). While several are justified (file-existence checks, JSON parsing with user feedback), a significant number silently swallow errors that could mask production failures. The most severe issues are:

1. **Registry file has no concurrent-access protection** -- data corruption risk
2. **Webhook notifier silently drops messages** after retry exhaustion -- no logging
3. **Multiple memory/search operations silently return empty** -- callers have no idea the backend failed
4. **CLI entry point has no top-level error handler** -- unhandled rejections crash the process
5. **LLM SSE parser silently drops malformed chunks** -- debugging streaming failures is impossible
6. **No timeout on HTTP server `.listen()`** -- can hang forever if port is taken (EADDRINUSE surfaces eventually but the Promise never rejects on some platforms)

---

## CRITICAL Findings

### C-01: Registry read/write has no file locking -- data corruption

**Location**: `src/adapters/secondary/registry-adapter.ts:108-120`

```typescript
private async readRegistry(): Promise<ProjectRegistry> {
  try {
    const content = await readFile(REGISTRY_PATH, 'utf-8');
    return JSON.parse(content) as ProjectRegistry;
  } catch {
    return { version: 1, projects: [] };
  }
}

private async writeRegistry(registry: ProjectRegistry): Promise<void> {
  await mkdir(REGISTRY_DIR, { recursive: true });
  await writeFile(REGISTRY_PATH, JSON.stringify(registry, null, 2) + '\n');
}
```

**Issue**: `readRegistry` + `writeRegistry` is a classic TOCTOU race. Two concurrent `hex` processes (e.g., two terminals running `hex dashboard`) will both read, modify, and write -- the second write silently overwrites the first's changes. The global registry at `~/.hex/registry.json` is shared across all projects.

**Hidden errors in the catch block**: The bare `catch {}` also swallows:
- Permission denied (EACCES) -- user thinks registry is empty, not inaccessible
- Corrupted JSON from a partial write (ENOSPC mid-write) -- silently resets to empty
- Disk I/O errors -- treated identically to "file doesn't exist yet"

**Severity**: CRITICAL
**User impact**: Port allocations can be duplicated, projects can vanish from the registry, or a corrupted file silently resets all registrations.
**Recommendation**:
1. Use `writeFile` with `{ flag: 'wx' }` + rename pattern (atomic write)
2. Distinguish ENOENT (file missing -- expected) from other errors (log and re-throw)
3. Consider `proper-lockfile` or a simple `.lock` file for cross-process safety

---

### C-02: CLI entry point has no top-level error boundary

**Location**: `src/cli.ts:13-18`

```typescript
const ctx = await createAppContext(process.cwd());
ctx.autoConfirm = autoConfirm;
const cli = new CLIAdapter(ctx);
const exitCode = await cli.run(filteredArgs);
process.exit(exitCode);
```

**Issue**: If `createAppContext` throws (e.g., `mkdir` fails for `.hex/`, or `TreeSitterAdapter.create` throws something unexpected beyond what's caught internally), the process crashes with an unhandled rejection and a raw stack trace. There is no `try/catch` or `process.on('unhandledRejection')` handler.

**Severity**: CRITICAL
**User impact**: Users see a raw Node.js stack trace instead of a helpful error message. No exit code is set (defaults to 1 on crash, but no cleanup runs).
**Recommendation**: Wrap the entire entry point in a try/catch that prints a user-friendly message and exits with code 1.

---

### C-03: Composition root silently swallows `.hex` directory creation failure

**Location**: `src/composition-root.ts:50`

```typescript
await mkdir(outputDir, { recursive: true }).catch(() => {});
```

**Issue**: This `.catch(() => {})` swallows every possible error: permission denied, read-only filesystem, disk full. The rest of the application proceeds assuming `outputDir` exists, and then file-log-notifier, registry writes, and other operations fail later with confusing errors far from the root cause.

**Severity**: CRITICAL
**User impact**: On a read-only filesystem or permission-restricted directory, the user gets cascading failures with no indication that the root cause was the initial directory creation.
**Recommendation**: At minimum, log the error to stderr. Better: distinguish EEXIST (fine) from EACCES/ENOSPC (fatal).

---

## HIGH Findings

### H-01: Webhook notifier silently drops messages after retry exhaustion

**Location**: `src/adapters/secondary/webhook-notifier.ts:155-181`

```typescript
private async sendPayload(text: string): Promise<void> {
  // ... retry loop ...
  // Silently drop after max retries -- do not block the agent pipeline.
  if (lastError) {
    // Could emit to a fallback channel in a future iteration.
  }
}
```

**Issue**: After 3 failed attempts with exponential backoff, the error is silently discarded. The comment acknowledges this is a problem ("Could emit to a fallback channel") but the `if (lastError)` block is literally empty. No logging, no counter, no metric. In production, a misconfigured webhook URL means ALL notifications vanish with zero indication.

**Severity**: HIGH
**User impact**: Webhook delivery fails silently. User configures a webhook, sees no errors, assumes it's working, but no messages arrive.
**Recommendation**: At minimum, `console.error('[webhook] Delivery failed after ${maxRetries} attempts: ${lastError.message}')`. Ideally, expose a delivery failure counter.

---

### H-02: RufloAdapter.memoryRetrieve and memorySearch silently return null/empty

**Location**: `src/adapters/secondary/ruflo-adapter.ts:124-143`

```typescript
async memoryRetrieve(key: string, namespace: string): Promise<string | null> {
  try {
    const result = await this.mcpExec('memory_retrieve', { key, namespace });
    return result.value ?? null;
  } catch {
    return null;
  }
}

async memorySearch(query: string, namespace?: string): Promise<SwarmMemoryEntry[]> {
  try { ... }
  catch {
    return [];
  }
}
```

**Issue**: Both methods catch ALL errors (including `SwarmConnectionError` when ruflo is down) and return empty results. The caller cannot distinguish "key not found" from "ruflo is completely unreachable." This masks infrastructure failures -- swarm coordination silently degrades to having no memory.

**Hidden errors swallowed**: Network timeouts, ruflo daemon not running, authentication failures, malformed responses.

**Severity**: HIGH
**User impact**: Swarm agents lose memory persistence without any indication. Coordination data appears to be "empty" rather than "unavailable."
**Recommendation**: Catch only "not found" scenarios (e.g., check error message or type). Re-throw connection errors so callers know the backend is down.

---

### H-03: LLM adapter parseSSELine silently drops malformed SSE chunks

**Location**: `src/adapters/secondary/llm-adapter.ts:144-161`

```typescript
private parseSSELine(line: string): string | null {
  // ...
  try {
    const data = JSON.parse(trimmed.slice(6)) as Record<string, unknown>;
    // ...
  } catch {
    return null;
  }
}
```

**Issue**: If the LLM API returns malformed JSON in an SSE chunk (which happens during API outages, rate limiting mid-stream, or proxy interference), the error is silently swallowed and the chunk is skipped. The caller (`streamPrompt`) has no idea content was lost. Partial responses could be returned as if they were complete.

**Severity**: HIGH
**User impact**: During streaming code generation, chunks may be silently lost, producing truncated or corrupted code that appears to be a valid (but incomplete) LLM response.
**Recommendation**: Track consecutive parse failures. After N failures, throw or yield an error signal. Log the first failure with the raw line content for debugging.

---

### H-04: LLM adapter parseResponse has no null checks -- crashes on unexpected API response shape

**Location**: `src/adapters/secondary/llm-adapter.ts:125-142`

```typescript
private parseResponse(json: Record<string, unknown>): LLMResponse {
  if (this.config.provider === 'anthropic') {
    const content = json.content as Array<{ text: string }>;
    const usage = json.usage as { input_tokens: number; output_tokens: number };
    return {
      content: content.map((c) => c.text).join(''),
      // ...
    };
  }
  const choices = json.choices as Array<{ message: { content: string } }>;
  const usage = json.usage as { prompt_tokens: number; completion_tokens: number };
  // ...
}
```

**Issue**: The `as` type assertions provide zero runtime safety. If the API returns an error response (e.g., `{ "error": { "message": "rate limited" } }`) that passed the `res.ok` check (some proxies return 200 with error bodies), `content.map` throws `TypeError: Cannot read properties of undefined (reading 'map')`. The error message is completely unhelpful.

**Severity**: HIGH
**User impact**: Cryptic `TypeError` instead of "LLM returned an unexpected response format."
**Recommendation**: Validate the response shape before accessing nested properties. Throw a descriptive error if the expected fields are missing.

---

### H-05: Dashboard/Hub swarm endpoint returns HTTP 200 with error data on failure

**Location**: `src/adapters/primary/dashboard-adapter.ts:291-299` and `src/adapters/primary/dashboard-hub.ts:336-350`

```typescript
} catch (err) {
  const message = err instanceof Error ? err.message : String(err);
  console.error('[dashboard] swarm query failed:', message);
  this.json(res, 200, {
    status: { id: 'none', ... error: message },
    tasks: [],
    agents: [],
  });
}
```

**Issue**: When the swarm backend fails, the dashboard returns HTTP 200 with mock/empty data that includes an `error` field buried in the status object. Frontend consumers checking `response.ok` (which is true for 200) will display empty data instead of an error state. The error is hidden in a nested property that most consumers won't check.

**Severity**: HIGH
**User impact**: Dashboard shows "no agents, no tasks, idle" when the real problem is that ruflo is unreachable. Users think nothing is running rather than knowing the monitoring is broken.
**Recommendation**: Return HTTP 503 (Service Unavailable) with the error, or at minimum use a top-level `{ error: ..., partial: true }` field that consumers can check.

---

### H-06: Composition root falls back to a mock IASTPort on tree-sitter failure

**Location**: `src/composition-root.ts:76-90`

```typescript
} catch (err) {
  const msg = err instanceof Error ? err.message : String(err);
  process.stderr.write(`[hex] WARNING: Tree-sitter init failed: ${msg}. Analysis will return empty results.\n`);
  astIsStub = true;
  ast = {
    async extractSummary(filePath, level) {
      return { filePath, language: 'typescript', level, exports: [], imports: [], dependencies: [], lineCount: 0, tokenEstimate: 0 };
    },
    diffStructural() { return { added: [], removed: [], modified: [] }; },
  };
}
```

**Issue**: When tree-sitter fails to load, the entire AST subsystem is replaced with a stub that returns empty results for everything. While `astIsStub` is set and a warning is printed, every downstream consumer (`archAnalyzer`, `summaryService`, `codeGenerator`) proceeds silently. The architecture analyzer will report "0 violations, 0 dead exports, 100% health" because it has no data -- a dangerously misleading result.

**Severity**: HIGH
**User impact**: `hex analyze .` reports perfect health when tree-sitter is broken, giving false confidence. The warning is only printed once at startup and easily missed.
**Recommendation**: When `astIsStub` is true, `analyzeArchitecture` should return a result with `healthScore: -1` or include a prominent warning. The CLI `analyze` command should print a warning banner.

---

## MEDIUM Findings

### M-01: FileLogNotifier.readEntries silently returns empty on ANY error

**Location**: `src/adapters/secondary/file-log-notifier.ts:163-173`

```typescript
async readEntries(): Promise<LogEntry[]> {
  try {
    const raw = await this.fs.readFile(this.logPath, 'utf-8');
    return raw.split('\n').filter(Boolean).map((line) => JSON.parse(line) as LogEntry);
  } catch {
    return [];
  }
}
```

**Hidden errors**: Permission denied, corrupted JSONL (one bad line kills ALL entries due to `.map(JSON.parse)` throwing on the first bad line), disk errors.

**Severity**: MEDIUM
**Recommendation**: Parse lines individually, skip bad lines with logging, distinguish file-not-found from other errors.

---

### M-02: FileLogNotifier.rotate silently swallows rename errors

**Location**: `src/adapters/secondary/file-log-notifier.ts:207-214`

```typescript
private async rotate(): Promise<void> {
  const rotated = `${this.logDir}/${this.logFile}.${Date.now()}.bak`;
  try {
    await this.fs.rename(this.logPath, rotated);
  } catch {
    // File may not exist yet; ignore.
  }
  this.currentSize = 0;
}
```

**Issue**: `currentSize` is reset to 0 even if the rename failed (e.g., disk full, permission denied). The next `appendEntry` will write to the same oversized file, believing rotation succeeded. On a full disk, this creates an infinite loop of failed rotations.

**Severity**: MEDIUM
**Recommendation**: Only reset `currentSize` if rename succeeds. Log the error if it's not ENOENT.

---

### M-03: RegistryAdapter.readLocalIdentity swallows all errors including corrupted JSON

**Location**: `src/adapters/secondary/registry-adapter.ts:83-90`

```typescript
async readLocalIdentity(rootPath: string): Promise<LocalProjectIdentity | null> {
  try {
    const content = await readFile(join(rootPath, '.hex', 'project.json'), 'utf-8');
    return JSON.parse(content) as LocalProjectIdentity;
  } catch {
    return null;
  }
}
```

**Hidden errors**: Corrupted JSON (user edited the file), permission denied, symlink loops. All return `null` as if the file simply doesn't exist.

**Severity**: MEDIUM
**Recommendation**: Catch ENOENT separately and return null. For parse errors or permission issues, throw or log.

---

### M-04: BuildAdapter.parseEslintOutput swallows JSON parse errors with a lossy fallback

**Location**: `src/adapters/secondary/build-adapter.ts:80-109`

```typescript
private parseEslintOutput(raw: string): LintResult {
  try {
    const files = JSON.parse(raw) as Array<...>;
    // ...
  } catch {
    return { success: false, errors: [], warningCount: 0, errorCount: 1 };
  }
}
```

**Issue**: If ESLint's JSON output is truncated (process killed, maxBuffer exceeded), the method returns "1 error" with no error details. The caller (`refineFromFeedback`) receives zero actionable information about what went wrong.

**Severity**: MEDIUM
**Recommendation**: Include the raw output (truncated) in the error list so the LLM refinement loop has something to work with.

---

### M-05: EventBusNotifier and NotificationOrchestrator swallow listener errors

**Location**: `src/adapters/secondary/event-bus-notifier.ts:59`, `src/core/usecases/notification-orchestrator.ts:483`, `src/adapters/primary/notification-query-adapter.ts:126`

```typescript
try { sub.handler(full); } catch { /* subscriber errors must not break the bus */ }
try { fn(notification); } catch { /* listener errors must not break emission */ }
try { fn(full); } catch { /* listener errors must not break ingestion */ }
```

**Issue**: Three separate locations silently swallow subscriber/listener errors. While the rationale is sound (one bad subscriber shouldn't crash the bus), these errors are completely invisible. A dashboard SSE listener that throws on every notification would never be detected.

**Severity**: MEDIUM
**Recommendation**: Log the error at debug/trace level: `console.error('[event-bus] subscriber error:', err)`. This preserves the isolation guarantee while making failures diagnosable.

---

### M-06: SummaryService.summarizeProject and CodeGenerator.loadPortSummaries silently skip files

**Location**: `src/core/usecases/summary-service.ts:40-45`, `src/core/usecases/code-generator.ts:258-265`

```typescript
} catch {
  // Skip files that cannot be parsed
}
```

**Issue**: If tree-sitter crashes on a specific file (not just "can't parse"), the error is silently skipped. The caller gets a partial result with no indication that files were omitted. In `loadPortSummaries`, a skipped port file means the code generator's system prompt is missing port definitions -- it will generate code that doesn't implement the right interfaces.

**Severity**: MEDIUM
**Recommendation**: Collect skipped files and their error messages. Return them alongside the results so callers can decide whether the partial result is acceptable.

---

### M-07: WorkplanExecutor.parsePlanResponse uses unguarded JSON.parse

**Location**: `src/core/usecases/workplan-executor.ts:132-161`

```typescript
private parsePlanResponse(content: string): Workplan {
  const jsonMatch = content.match(/\{[\s\S]*\}/);
  if (!jsonMatch) {
    throw new Error('LLM response did not contain valid JSON workplan');
  }
  const parsed = JSON.parse(jsonMatch[0]) as { ... };
```

**Issue**: The regex `\{[\s\S]*\}` is greedy and will match from the FIRST `{` to the LAST `}` in the entire response. If the LLM includes explanatory text with JSON examples, the regex captures garbage. `JSON.parse` then throws a raw `SyntaxError` with no context about what was being parsed or what the LLM returned.

**Severity**: MEDIUM
**Recommendation**: Wrap `JSON.parse` in a try/catch that includes the matched string (truncated) in the error message. Use a more precise extraction strategy.

---

### M-08: Dashboard server.listen() has no error handler

**Location**: `src/adapters/primary/dashboard-adapter.ts:76-91`, `src/adapters/primary/dashboard-hub.ts:104-113`

```typescript
return new Promise((ok) => {
  server.listen(this.port, () => {
    const url = `http://localhost:${this.port}`;
    ok({ url, close: () => { ... } });
  });
});
```

**Issue**: The Promise only has a resolve callback, no reject. If `server.listen` fails (EADDRINUSE, EACCES), the `'error'` event fires on the server but the Promise hangs forever -- it never resolves or rejects. The CLI command that started the dashboard will appear to freeze.

**Severity**: MEDIUM
**Recommendation**: Add `server.on('error', reject)` before calling `server.listen`.

---

### M-09: No timeout on execFile calls in BuildAdapter

**Location**: `src/adapters/secondary/build-adapter.ts:136-145`

```typescript
private async run(cmd: string, args: string[], cwd?: string): Promise<...> {
  return execFile(cmd, args, {
    cwd: cwd ?? this.projectPath,
    maxBuffer: 10 * 1024 * 1024,
  });
}
```

**Issue**: Unlike `RufloAdapter` (which has `timeout: 30000`), `BuildAdapter.run()` has no timeout. A hanging `tsc`, `eslint`, or `bun test` process will block the entire pipeline indefinitely. This is particularly dangerous in the code-generator's compile-lint-test feedback loop.

**Severity**: MEDIUM
**Recommendation**: Add `timeout: 120000` (2 minutes) to match reasonable build times. Handle the timeout error specifically.

---

### M-10: GitAdapter and WorktreeAdapter have no timeout

**Location**: `src/adapters/secondary/git-adapter.ts:50-63`, `src/adapters/secondary/worktree-adapter.ts:70-85`

**Issue**: Same as M-09. Git operations (especially `git diff` on large repos or `git merge` with conflicts) can hang. No timeout is set.

**Severity**: MEDIUM
**Recommendation**: Add `timeout: 30000` to git execFile calls.

---

## LOW Findings

### L-01: `readBody` in dashboard can leak if `req.destroy()` doesn't trigger 'end' or 'error'

**Location**: `src/adapters/primary/dashboard-adapter.ts:392-408`

The Promise may hang if `req.destroy()` doesn't trigger either 'end' or 'error' events on all platforms. The `fail` is called inline but the Promise isn't guaranteed to settle.

### L-02: Notification orchestrator stall check timer is never cleaned on process exit

**Location**: `src/core/usecases/notification-orchestrator.ts:107-111`

`setInterval` keeps the Node.js event loop alive. If `stop()` is never called, the process can't exit gracefully.

### L-03: DashboardHub.registerProject uses last path segment as ID -- collision risk

**Location**: `src/adapters/primary/dashboard-hub.ts:135`

```typescript
const id = absPath.split('/').pop() ?? 'unknown';
```

Two projects at `/home/user/project-a/backend` and `/home/user/project-b/backend` both get ID `backend`. The second silently returns the first's context.

### L-04: MCPAdapter.dashboardList fetches from its own HTTP server via localhost

**Location**: `src/adapters/primary/mcp-adapter.ts:407`

```typescript
const response = await fetch(`${this.hubUrl}/api/projects`);
```

No error handling if the server is unreachable (self-fetch failure). The `response.json()` call also has no try/catch.

### L-05: Composition root TreeSitterAdapter.create catches errors but also has isStub() check

**Location**: `src/composition-root.ts:64-90`

The inner `treeSitter.isStub()` check (line 72) means tree-sitter "worked" but found no grammars. The outer catch (line 76) means tree-sitter itself failed to initialize. Both paths set `astIsStub = true` but only the catch path logs a warning. The no-grammars-found path is completely silent.

---

## Resource Leak Inventory

| Resource | File | Cleanup | Status |
|----------|------|---------|--------|
| HTTP server | dashboard-adapter.ts:70 | `close()` callback | OK -- but no timeout on `server.close()` |
| HTTP server | dashboard-hub.ts:100 | `shutdown()` | OK |
| FSWatcher | dashboard-adapter.ts:98 | `close()` in close callback | OK |
| FSWatcher | dashboard-hub.ts:183 | `shutdown()` + `unregisterProject()` | OK |
| setInterval (heartbeat) | dashboard-adapter.ts:332 | `req.on('close')` | OK |
| setInterval (stall check) | notification-orchestrator.ts:107 | `stop()` | RISK -- no process.exit hook |
| setTimeout (debounce) | dashboard-adapter.ts:108 | No explicit cleanup | LOW RISK -- short-lived |
| setTimeout (flush) | webhook-notifier.ts:148 | `flush()` | OK if `flush()` called on shutdown |
| child_process (execFile) | ruflo-adapter.ts:161 | timeout: 30000 | OK |
| child_process (execFile) | build-adapter.ts:141 | NO TIMEOUT | RISK |
| child_process (execFile) | git-adapter.ts:51 | NO TIMEOUT | RISK |
| child_process (execFile) | worktree-adapter.ts:74 | NO TIMEOUT | RISK |
| fetch body reader | llm-adapter.ts:63 | Read to completion | OK |

---

## Graceful Degradation Matrix

| External Dependency | What Happens When Missing/Broken | Quality |
|---|---|---|
| Tree-sitter WASM grammars | Falls back to stub, all analysis returns empty. **Warning printed but results are misleadingly "clean"** | POOR |
| Ruflo CLI (`@claude-flow/cli`) | `SwarmConnectionError` propagates from most methods. `memoryRetrieve`/`memorySearch` silently return empty. | MIXED |
| LLM API key | `llm`, `codeGenerator`, `workplanExecutor` are null. CLI commands check for null. | GOOD |
| LLM API at runtime | HTTP errors thrown with status code and body. Streaming failures silently drop chunks. | MIXED |
| Filesystem (disk full) | `mkdir` failure swallowed in composition-root. Write operations will throw later with confusing errors. | POOR |
| Git not installed | `GitError` with clear message. | GOOD |
| `src/` directory missing | File watcher catches error and logs. Analysis returns empty (no files). | OK |
| Registry JSON corrupted | Silently resets to empty registry. All registrations lost. | POOR |

---

## Summary Statistics

| Category | Count |
|----------|-------|
| Total catch blocks in `src/` | 37 |
| Bare `catch {` (no error variable) | 21 |
| Silent swallow (no log, no user feedback) | 11 |
| Catch with logging | 8 |
| Catch with user-facing error message | 12 |
| Catch with proper error propagation | 6 |
| Missing timeouts on child processes | 3 adapters |
| Unguarded JSON.parse (no try/catch) | 1 (workplan-executor regex extraction) |
| Race conditions | 1 (registry.json concurrent access) |

---

## Priority Remediation Order

1. **C-02**: Add top-level error boundary in `cli.ts`
2. **C-01**: Add atomic writes and error discrimination to registry-adapter
3. **C-03**: Stop swallowing `.hex` mkdir errors in composition-root
4. **H-06**: Make stub AST produce visibly degraded results (not "all clean")
5. **H-01**: Log webhook delivery failures
6. **H-02**: Distinguish "not found" from "backend down" in ruflo memory methods
7. **H-04**: Validate LLM response shape before accessing nested properties
8. **H-05**: Return non-200 status when swarm backend is unreachable
9. **M-08**: Add error handler to server.listen Promise
10. **M-09/M-10**: Add timeouts to build-adapter and git-adapter execFile calls
