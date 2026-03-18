# How Could We Have Detected These Issues Earlier?
**Date**: 2026-03-17
**Context**: Post-mortem on dashboard status bug and ADR tracking bug

---

## The Issues We Found

### Issue #1: ADR Tracking "Not Available"
**Root Cause**: `AppContext.adrQuery` typed as nullable but always initialized
**Detection Method**: Manual testing with MCP tools
**Time to Find**: ~30 minutes of investigation

### Issue #2: Dashboard Status Reports "Not Running"
**Root Cause**: CLI uses `DaemonManager`, composition-root uses `HubLauncher` (architecture mismatch)
**Detection Method**: Manual testing, process inspection
**Time to Find**: ~20 minutes of investigation

---

## Early Detection Strategy #1: Memory Systems

### How Memory Could Have Caught This

**Pattern Storage in AgentDB**:
```json
{
  "name": "AppContext fields must never be nullable if always initialized",
  "category": "architecture-invariant",
  "content": "When composition-root.ts ALWAYS creates a port (no conditional),
             the AppContext type must NOT include | null. This creates defensive
             null checks that always fail.",
  "confidence": 0.95,
  "tags": ["type-safety", "false-positive-check", "composition-root"]
}
```

**How It Works**:
1. During commit `875cb4e`, we wired up `adrQuery` in composition-root (line 410, 465)
2. AgentDB pattern matcher sees: "always initialized, but typed as nullable"
3. Pre-commit hook warns: "⚠️ adrQuery is always created but typed as | null"

**Implementation**:
```typescript
// In hex's own pre-commit hook
async function checkNullablePatterns(ctx: AppContext) {
  const patterns = await ctx.swarm.patternSearch('nullable-but-always-initialized');

  // Check composition-root.ts for "always create X" patterns
  const compositionCode = await ctx.fs.read('src/composition-root.ts');
  const appContextType = await ctx.fs.read('src/core/ports/app-context.ts');

  // Look for: const X = new Y(...) followed by { X, ... } in return
  // Cross-reference with AppContext type having X: Type | null

  // Warn if mismatch found
}
```

**Why This Would Work**:
- Composition root is THE SINGLE source of truth for what's initialized
- Type mismatch between "always present" and "nullable" is a code smell
- AgentDB learns this pattern from previous fixes

---

## Early Detection Strategy #2: Context Checking (Cross-File Analysis)

### Static Analysis Rules

**Rule 1: Single Responsibility Checker**
```typescript
// Detect when two adapters solve the same problem
interface ConflictingAdapters {
  adapter1: string;  // 'daemon-manager.ts'
  adapter2: string;  // 'hub-launcher.ts'
  overlap: string;   // 'Both manage dashboard daemon lifecycle'
  risk: 'high';      // Likely to have inconsistencies
}
```

**How It Detects Dashboard Bug**:
```bash
$ hex analyze . --cross-file-conflicts

⚠️ Potential Conflict Detected:
  - adapters/primary/daemon-manager.ts
  - adapters/secondary/hub-launcher.ts

Both implement:
  - start() → spawn dashboard process
  - status() → check if running
  - stop() → kill process

Risk: CLI may call wrong implementation.
Recommendation: Consolidate into one adapter or document which is canonical.
```

**Implementation**:
```typescript
function detectDuplicateAdapters(files: ASTSummary[]): ConflictingAdapters[] {
  const adapters = files.filter(f => f.filePath.includes('adapter'));
  const methodSignatures = new Map<string, string[]>();

  // Group adapters by similar method names
  for (const adapter of adapters) {
    const methods = adapter.exports.filter(e => e.kind === 'function')
                                   .map(e => e.name);

    for (const method of methods) {
      if (!methodSignatures.has(method)) methodSignatures.set(method, []);
      methodSignatures.get(method)!.push(adapter.filePath);
    }
  }

  // Flag if 2+ adapters share 3+ identical method names
  const conflicts = [];
  for (const [method, adapters] of methodSignatures) {
    if (adapters.length >= 2) {
      const sharedMethods = Array.from(methodSignatures.values())
                                 .filter(a => a.length >= 2).length;
      if (sharedMethods >= 3) {
        conflicts.push({ adapters, sharedMethods });
      }
    }
  }

  return conflicts;
}
```

---

## Early Detection Strategy #3: Inference from Call Graphs

### Import Graph Analysis

**What We Know**:
- `cli-adapter.ts` imports `DaemonManager`
- `composition-root.ts` imports `HubLauncher`
- Both call `.status()` and `.start()`

**Inference Rule**:
```
IF:
  - Two files import different adapters for the same port
  - Both adapters have overlapping methods
  - No ADR documents "use X in context Y, use Z in context W"

THEN:
  - WARN: "Inconsistent adapter usage detected"
  - SUGGEST: "Document canonical adapter in ADR or unify"
```

**How It Would Warn**:
```bash
$ hex analyze . --infer-inconsistencies

🔍 Inference: Inconsistent Daemon Management

  cli-adapter.ts        → DaemonManager.status()
  composition-root.ts   → HubLauncher.status()

Both check dashboard status but use different implementations.
This may cause:
  - CLI reports "not running" while hub IS running
  - Race conditions between two lifecycle managers

Suggested Fix:
  1. Choose ONE canonical implementation (HubLauncher recommended)
  2. Update CLI to use HubLauncher
  3. Document in ADR-XXX: "Dashboard Lifecycle Management"
```

**Implementation**:
```typescript
async function inferInconsistentPortUsage(ctx: AppContext): Promise<Warning[]> {
  const warnings = [];
  const imports = await ctx.archAnalyzer.buildDependencyGraph(ctx.rootPath);

  // Build "who calls what" graph
  const portUsage = new Map<string, Set<string>>(); // port → callers

  for (const edge of imports) {
    if (edge.from.includes('adapter') && edge.to.includes('adapter')) {
      // Two adapters for same purpose?
      const methods = await extractSharedMethods(edge.from, edge.to);
      if (methods.length >= 3) {
        warnings.push({
          type: 'inconsistent-adapter-usage',
          files: [edge.from, edge.to],
          sharedMethods: methods,
          recommendation: 'Choose one canonical implementation',
        });
      }
    }
  }

  return warnings;
}
```

---

## Early Detection Strategy #4: Test Coverage for Integration Points

### Missing Test: "CLI and Composition-Root Use Same Adapter"

```typescript
// tests/integration/adapter-consistency.test.ts
import { describe, test, expect } from 'bun:test';
import { createAppContext } from '../../src/composition-root.js';

describe('Adapter Consistency', () => {
  test('CLI daemon command uses same adapter as composition-root', async () => {
    // Parse CLI code to find daemon manager import
    const cliCode = await Bun.file('src/adapters/primary/cli-adapter.ts').text();
    const daemonImport = cliCode.match(/from ['"]\.\/(.+?)['"];/g)
                                .find(line => line.includes('daemon'));

    // Parse composition-root to find hub manager import
    const rootCode = await Bun.file('src/composition-root.ts').text();
    const hubImport = rootCode.match(/from ['"]\.\/adapters\/secondary\/(.+?)['"];/g)
                              .find(line => line.includes('hub') || line.includes('daemon'));

    // They should import the SAME adapter
    expect(daemonImport).toBe(hubImport);
  });

  test('daemon status matches hub status', async () => {
    const ctx = await createAppContext(process.cwd());

    // Start hub via composition-root
    await ctx.hubLauncher.start();

    // Check via CLI's daemon manager
    const { DaemonManager } = await import('../../src/adapters/primary/daemon-manager.js');
    const daemon = new DaemonManager();
    const status = await daemon.status();

    // Should report same state
    expect(status.running).toBe(true);
  });
});
```

**Why This Would Catch the Bug**:
- Test would FAIL when CLI reports "not running" but hub IS running
- Forces us to reconcile the two systems before shipping

---

## Recommendation: Implement All 4 Strategies

### Priority Order:

1. **Memory (AgentDB patterns)** — HIGH
   - Low overhead, high value
   - Learns from every fix we make
   - Pre-commit hook integration

2. **Cross-File Analysis** — MEDIUM
   - Part of `hex analyze`
   - Detects architectural drift
   - Already have AST infrastructure

3. **Inference Engine** — MEDIUM
   - Extends `hex analyze --infer`
   - Uses call graph + import graph
   - Suggests fixes, not just warnings

4. **Integration Tests** — LOW
   - Most expensive (requires maintaining tests)
   - But catches real bugs before users see them
   - Good for critical paths (CLI, MCP, composition-root)

---

## Implementation Plan

### Phase 1: Memory Patterns (Sprint 1)
```typescript
// Add to pre-commit hook
async function checkArchitectureInvariants() {
  const patterns = [
    'nullable-but-always-initialized',
    'duplicate-adapter-responsibility',
    'missing-adr-for-dual-implementation',
  ];

  for (const pattern of patterns) {
    const matches = await agentdb.patternSearch(pattern);
    // Check current code against learned patterns
    // Warn if violation detected
  }
}
```

### Phase 2: Cross-File Conflict Detector (Sprint 2)
```bash
$ hex analyze . --check-conflicts
```

### Phase 3: Inference Engine (Sprint 3)
```bash
$ hex analyze . --infer-issues
```

### Phase 4: Integration Test Suite (Sprint 4)
```bash
$ bun test tests/integration/adapter-consistency.test.ts
```

---

## Learning for Future

### What We Should Remember:

1. **When you see `Type | null` but "always initialized"** → Code smell
2. **When two files have similar method names** → Possible duplication
3. **When CLI and composition-root diverge** → Integration test gap
4. **When status commands fail but process runs** → Check lock files vs HTTP

### AgentDB Pattern to Store:
```json
{
  "name": "Detect nullable-but-always-initialized anti-pattern",
  "trigger": "git pre-commit",
  "check": "composition-root.ts always creates X, but AppContext has X: T | null",
  "action": "WARN and suggest removing | null",
  "learnedFrom": "commit a274139 (ADR tracking fix)",
  "confidence": 0.9
}
```

---

## Meta-Learning: How We Found It

**Actual Process**:
1. Manual testing → "ADR tracking not available"
2. Grep for error message → Find null check
3. Grep for adrQuery → Find it's always initialized
4. Remove null, test, commit

**Better Process (with memory)**:
1. Pre-commit hook → "⚠️ adrQuery is nullable but always initialized"
2. Remove null, test, commit
3. Store pattern in AgentDB for next time

**Best Process (with inference)**:
1. During development: "🔍 Detected: adrQuery initialization mismatch"
2. Fix suggested: "Remove | null from line 118"
3. Apply fix, test, commit
4. Pattern automatically stored

---

## Conclusion

We COULD have caught both bugs earlier with:
- **Memory**: Pattern matching against known anti-patterns
- **Context**: Cross-file analysis of adapter conflicts
- **Inference**: Call graph analysis of inconsistent usage

**Next Step**: Implement memory-based pre-commit checks for hex itself.
