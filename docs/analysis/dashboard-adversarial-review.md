# Adversarial Review: Dashboard Adapter Hexagonal Boundary Compliance

**Date**: 2025-03-15
**Reviewer**: Adversarial Tester
**Scope**: dashboard-adapter.ts, dashboard/index.html, cli-adapter.ts, composition-root.ts, ports

---

## Finding 1: Unsafe `as FullAppContext` Cast in CLI Adapter

**Severity**: CRITICAL
**File**: `src/adapters/primary/cli-adapter.ts`, line 339
**Code**: `await startDashboard(this.ctx as FullAppContext, port);`

The CLI adapter's `AppContext` type is missing fields the dashboard requires: `swarm`, `notificationOrchestrator`, `notifier`, and `eventBus`. The cast `as FullAppContext` silences the compiler but produces runtime undefined access. When `handleSwarm()` calls `this.ctx.swarm.status()`, it will throw `TypeError: Cannot read properties of undefined`. The `try/catch` in `handleSwarm` masks this with a fallback, but `handleDecision` calls `this.ctx.notificationOrchestrator.respondToDecision()` with no such protection -- that endpoint will crash.

**Fix**: CLI adapter's `AppContext` must include the dashboard's required fields, or the dashboard command must validate their presence before starting.

---

## Finding 2: Cross-Adapter Import Violation -- Composition Root Type Leak

**Severity**: HIGH
**File**: `src/adapters/primary/dashboard-adapter.ts`, line 23-24

```typescript
import type { AppContext as FullAppContext } from '../../composition-root.js';
import type { ImportEdge } from '../../core/ports/index.js';
```

The `ImportEdge` import from ports is correct. The `FullAppContext` import from `composition-root.ts` is architecturally acceptable (type-only, used to derive a subset via `Pick`). However, `composition-root.ts` references concrete use-case classes like `NotificationOrchestrator` directly in its `AppContext` interface (line 47: `notificationOrchestrator: NotificationOrchestrator`). This means `FullAppContext` transitively couples the dashboard adapter to the concrete use-case class, not to a port interface. `NotificationOrchestrator` is imported from `core/usecases/`, not defined as a port.

**Fix**: `AppContext` in composition-root should reference `INotificationQueryPort` (from the notification port) instead of the concrete `NotificationOrchestrator` class.

---

## Finding 3: CORS Wildcard in Production

**Severity**: HIGH
**File**: `src/adapters/primary/dashboard-adapter.ts`, lines 108-110

```typescript
res.setHeader('Access-Control-Allow-Origin', '*');
```

Unconditional `*` origin allows any website to make API calls to the dashboard. Combined with `POST /api/decisions/:id`, a malicious page could auto-submit decision responses on behalf of the developer if they have the dashboard open.

**Fix**: Restrict to `localhost` origins or make CORS configurable.

---

## Finding 4: No Request Body Size Limit

**Severity**: HIGH
**File**: `src/adapters/primary/dashboard-adapter.ts`, lines 328-335

The `readBody()` function accumulates all chunks with no size cap. An attacker (or malformed client) can POST gigabytes to `/api/decisions/:id` causing OOM. This is a denial-of-service vector.

**Fix**: Add a maximum body size (e.g., 64KB) and abort the request if exceeded.

---

## Finding 5: Path Traversal in `/api/tokens/:file`

**Severity**: HIGH
**File**: `src/adapters/primary/dashboard-adapter.ts`, line 128

```typescript
const file = decodeURIComponent(path.slice('/api/tokens/'.length));
```

The decoded file path is passed directly to `this.ctx.ast.extractSummary(file, ...)`. If `extractSummary` reads the filesystem based on this path, a request to `/api/tokens/..%2F..%2F..%2Fetc%2Fpasswd` could access files outside the project. Whether this is exploitable depends on whether the AST port's implementation validates paths, but the dashboard adapter performs zero sanitization.

**Fix**: Validate that the resolved path stays within `this.ctx.rootPath`.

---

## Finding 6: Stale Cache Showing Misleading Data

**Severity**: MEDIUM
**File**: `src/adapters/primary/dashboard-adapter.ts`, lines 73-74

Health is cached for 10 seconds, tokens for 30 seconds. There is no cache invalidation mechanism. If a developer fixes a boundary violation and refreshes within 10s, the dashboard still shows the old violation count and health score. The 30s token cache is worse -- after editing a file, the token efficiency panel shows pre-edit numbers for up to 30 seconds.

The HTML polling interval (line 881 in index.html) is also 10s, which aligns with the cache TTL -- meaning every poll could hit the cache and the user could see stale data for up to 20s in the worst case (poll at t=0 fills cache, cache expires at t=10, next poll at t=10 refills, user sees t=0 data until t=10).

**Fix**: Add a `?bust=timestamp` parameter or a manual refresh button that bypasses the cache.

---

## Finding 7: SSE Client Set Memory Leak Vector

**Severity**: MEDIUM
**File**: `src/adapters/primary/dashboard-adapter.ts`, lines 259-278

The cleanup relies on the `req.on('close')` event. If a client disconnects uncleanly (network drop without TCP FIN), the `close` event may be significantly delayed or never fire until the next `heartbeat` write fails. During that window, the `ServerResponse` object remains in the `sseClients` set, and `broadcast()` will write to a dead socket. The `heartbeat` interval (15s) also remains active, leaking timers.

The `broadcast()` method (line 313) calls `client.write()` on potentially dead connections with no error handling -- a write error on a destroyed socket could throw.

**Fix**: Wrap `client.write()` in try/catch inside `broadcast()`. Consider a maximum client limit.

---

## Finding 8: Health API Returns Misleading Data When Tree-Sitter Is Stub

**Severity**: MEDIUM
**File**: `src/adapters/primary/dashboard-adapter.ts`, line 172

When `astIsStub` is true, the stub AST returns empty arrays for everything. `archAnalyzer.analyzeArchitecture()` will report 0 violations, 0 dead exports, and likely a perfect health score -- because it found nothing, not because the code is healthy. The dashboard has no indication that tree-sitter is a stub. The CLI adapter prints a warning (line 139-142), but the dashboard shows a green "100/100" ring, which is actively deceptive.

The `astIsStub` field IS available in the dashboard's `AppContext` (line 33) but is never used in any API response.

**Fix**: Include `astIsStub` in the `/api/health` response and show a warning banner in the HTML when true.

---

## Finding 9: Graph Layer Classifier Missing Layers

**Severity**: MEDIUM
**File**: `src/adapters/primary/dashboard-adapter.ts`, lines 60-67

The `classifyLayer()` function checks for `/core/domain/`, `/core/ports/`, `/core/usecases/`, `/adapters/primary/`, `/adapters/secondary/`. But the composition root (`src/composition-root.ts`) and entry points (`src/cli.ts`, `src/index.ts`) match none of these patterns and fall into `'other'`. The HTML legend (line 253-258) shows Domain, Ports, Adapters, Use Cases, Violation -- but no "Other" color. Nodes for `composition-root.ts` will be gray (`#9e9e9e`) with no legend entry, confusing users.

Additionally, `'primary-adapter'` and `'secondary-adapter'` are distinct layer values but the HTML `LAYER_COLORS` map (line 695-703) only has `'adapter'` and `'adapters'` -- not `'primary-adapter'` or `'secondary-adapter'`. These nodes will render gray instead of orange.

**Fix**: Add `'primary-adapter'` and `'secondary-adapter'` to `LAYER_COLORS` in the HTML, or normalize to `'adapter'` in `classifyLayer()`.

---

## Finding 10: HTML Uses `textContent` Everywhere -- No XSS

**Severity**: LOW (positive finding)
**File**: `src/adapters/primary/dashboard/index.html`

The HTML exclusively uses `textContent`, `createTextNode`, and `createElement` for DOM manipulation. There is an `escapeHtml` helper defined (line 294) but it is actually never called -- because `textContent` is used instead of `innerHTML` throughout. No `innerHTML` assignments exist anywhere in the script. This is correct and secure.

The only potential issue: `escapeHtml` is dead code. It should be removed or used.

---

## Finding 11: `handleTokenDetail` Returns Nested Objects, HTML Expects Flat

**Severity**: LOW
**File**: `src/adapters/primary/dashboard-adapter.ts`, lines 211-218 vs `index.html` lines 462-484

The API returns `{ l0: ASTSummary, l1: ASTSummary, l2: ASTSummary, l3: ASTSummary }` where each is a full summary object with `tokenEstimate`, `lineCount`, etc. But the HTML `loadFileTokens` function accesses `data.l0`, `data.l1` etc. as if they are raw numbers (line 471: `var val = data[lv.key] || 0`). An `ASTSummary` object is truthy, so `val` becomes the object, and `val / maxTok` produces `NaN`. The token bars will show `NaN%` and zero-width bars.

**Fix**: Either return `{ l0: summary.l0.tokenEstimate, ... }` from the API, or access `data[lv.key].tokenEstimate` in the HTML.

---

## Finding 12: No `decisionId` Validation

**Severity**: LOW
**File**: `src/adapters/primary/dashboard-adapter.ts`, line 141

The decision ID is extracted from the URL path with no validation. Any string (including empty string, special characters, or very long values) is passed to `notificationOrchestrator.respondToDecision()`. Whether this causes issues depends on the orchestrator implementation, but the adapter should validate at the boundary.

---

## Summary Table

| # | Finding | Severity | Category |
|---|---------|----------|----------|
| 1 | Unsafe `as FullAppContext` cast | CRITICAL | Type Safety |
| 2 | Composition root leaks concrete class into AppContext | HIGH | Hex Boundary |
| 3 | CORS wildcard allows cross-origin decision submission | HIGH | Security |
| 4 | No request body size limit (OOM DoS) | HIGH | Security |
| 5 | Path traversal in `/api/tokens/:file` | HIGH | Security |
| 6 | Stale cache with no invalidation mechanism | MEDIUM | UX / Correctness |
| 7 | SSE client set memory leak + unhandled write errors | MEDIUM | Reliability |
| 8 | Stub AST produces misleading 100/100 health score | MEDIUM | Correctness |
| 9 | Graph layer classifier mismatches HTML color map | MEDIUM | Correctness |
| 10 | No innerHTML/XSS vectors (positive) | LOW | Security |
| 11 | API returns objects but HTML expects numbers (broken UI) | LOW | Bug |
| 12 | No decision ID validation at boundary | LOW | Input Validation |

**Critical**: 1 | **High**: 4 | **Medium**: 4 | **Low**: 3
