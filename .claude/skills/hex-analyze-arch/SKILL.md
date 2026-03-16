---
name: hex-analyze-arch
description: Check hexagonal architecture health, find dead code, and validate boundary rules. Automatically creates tasks and spawns fix agents for violations found. Use when the user asks to "check architecture", "find dead code", "validate hex boundaries", "architecture health", "detect circular dependencies", or "hex analyze".
---

# Hex Analyze Arch — Architecture Health Check with Auto-Fix

Runs a full hexagonal architecture analysis. When violations are found, automatically creates ruflo tasks and spawns a fix swarm — no manual triage needed.

## Parameters

- **rootPath** (optional, default: "."): Root directory to analyze
- **autoFix** (implicit: true when critical/high issues found): Spawn agents to fix violations
- **reportOnly** (optional, default: false): Skip auto-fix, just report

## Execution Steps

### 1. Run Architecture Analysis

Call the hex_analyze MCP tool:

```
mcp__hex__hex_analyze({ path: rootPath })
```

This returns the formatted HEXAGONAL ARCHITECTURE HEALTH REPORT with:
- Summary table (files, exports, violations, circular deps, dead exports, repo hygiene)
- Error rates with thresholds
- Layer breakdown
- Boundary violations grouped by severity (CRITICAL/WARNING)
- Circular dependencies
- Dead exports grouped by file
- Unused ports & adapters
- **Repo hygiene (anti-slop)**: uncommitted files, staged-not-committed, orphan worktrees, embedded git repos, untracked build artifacts, runtime state dirs
- Health score and grade (A-F)

Display the full report to the user.

### 2. Extract Action Items

Use the action item extractor from the domain layer to convert findings into structured tasks:

```typescript
import { buildActionItemReport, formatActionItems } from 'src/core/domain/action-items.ts';

const actionReport = buildActionItemReport(archResult);
const formatted = formatActionItems(actionReport);
```

Display the ACTION ITEMS report showing:
- MUST FIX: Critical and high-priority items with suggested fixes
- SHOULD FIX: Medium and low-priority items

### 3. Register Tasks in Ruflo

For each action item with priority `critical` or `high`, create a ruflo task:

```
mcp__ruflo__task_create({
  title: item.title,
  metadata: {
    category: item.category,        // bug, violation, circular-dep
    priority: item.priority,        // critical, high
    file: item.file,                // affected file path
    layer: item.layer,              // hex layer name
    suggestedFix: item.suggestedFix,
    source: "hex-analyze-arch",
    autoFixable: item.autoFixable
  }
})
```

Store the full report in ruflo memory:
```
mcp__ruflo__memory_store("devtracker/validation-actions", JSON.stringify(actionReport))
```

### 4. Auto-Fix Decision

If `reportOnly` is true, stop here with the report.

Otherwise, evaluate whether to auto-fix:

| Condition | Action |
|-----------|--------|
| Critical violations (cross-adapter, domain leak) | Spawn fix agent immediately |
| Circular dependencies | Spawn fix agent immediately |
| High-priority violations only | Spawn fix agent |
| Medium/low only (dead exports, unused ports) | Report only, do not auto-fix |
| Score >= 90 (Grade A) | No action needed |

### 5. Spawn Fix Swarm

For each fixable issue, spawn a targeted agent in a worktree:

```
Agent tool: {
  subagent_type: "general-purpose",
  mode: "bypassPermissions",
  run_in_background: true,
  isolation: "worktree",
  prompt: <fix prompt with full context>
}
```

#### Fix Prompts by Category

**Boundary Violation** (adapter imports from wrong layer):
```
Fix hex boundary violation in {file}.
Current: imports {names} from {wrongLayer}
Required: import through ports layer only.

Check if the types are already re-exported through a port file.
If yes: change the import path to use the port.
If no: add re-exports to the appropriate port file, then update the import.

Rules:
- adapters/ may only import from ports/
- domain/ may only import from domain/
- usecases/ may only import from domain/ and ports/

After fixing, run: bun test
```

**Circular Dependency**:
```
Break circular dependency: {cycle}

Strategies (pick the simplest):
1. Extract shared types to a common domain file both can import
2. Introduce a port interface to break the direct dependency
3. Use event-based decoupling if the cycle is behavioral

After fixing, run: bun test
```

**Domain Leak** (domain imports non-domain):
```
Fix domain purity violation in {file}.
Domain layer must have zero external dependencies.
Move the imported types to domain/ or inject via port interface.

After fixing, run: bun test
```

### 6. Monitor Fix Results

After spawning fix agents:

1. Wait for agent completion notifications
2. For each completed fix:
   - Run `mcp__hex__hex_validate_boundaries(".")` to verify the fix
   - If violation resolved: `mcp__ruflo__task_complete(taskId, commitHash)`
   - If still broken: report failure, do NOT retry automatically
3. Run final `mcp__hex__hex_analyze(".")` to get updated health score

### 7. Write Report

Write the full analysis + action items + fix results to `docs/analysis/arch-report.md`:

```markdown
# Architecture Health Report — {date}

## Score: {score}/100 ({grade})

## Summary
{summary table}

## Error Rates
{error rates table}

## Violations Found
{violations table with severity}

## Action Items Created
{list of ruflo tasks created}

## Fixes Applied
{list of auto-fixed issues with commit hashes}

## Remaining Issues
{items that need manual attention}
```

### 8. Final Status

Display to the user:
- Before/after health score comparison
- Number of issues auto-fixed vs remaining
- Any items that need manual attention
- Link to the full report

## Exclude Patterns

The analyzer excludes these by default:
- `node_modules`, `dist`, `examples`
- `*.test.ts`, `*.spec.ts`, `*_test.go`, `*.test.rs`
- `/tests/`
- `**/target/**` (Rust/Cargo build artifacts)

## Output

- Full health report displayed inline
- Action items report displayed inline
- Ruflo tasks created for critical/high issues
- Fix agents spawned for violations (unless reportOnly)
- Report written to `docs/analysis/arch-report.md`
