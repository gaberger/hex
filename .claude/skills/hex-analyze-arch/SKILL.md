---
name: hex-analyze-arch
description: Check hexagonal architecture health, find dead code, and validate boundary rules. Use when the user asks to "check architecture", "find dead code", "validate hex boundaries", "architecture health", "detect circular dependencies", or "hex analyze".
---

# Hex Analyze Arch — Architecture Health Check

Runs a full hexagonal architecture analysis: dead export detection, dependency direction validation, and circular dependency detection.

## Parameters

Ask the user for:
- **rootPath** (optional, default: "src"): Root directory to analyze
- **excludePatterns** (optional, default: "node_modules,dist,*.test.ts,*.spec.ts"): Comma-separated glob patterns to exclude

## Execution Steps

### 1. Run hex analyze

```bash
npx hex analyze {rootPath}
```

If hex is not available, perform manual analysis as described below.

### 2. Collect File Inventory

Glob all source files in rootPath, excluding the specified patterns. Extract L1 AST summaries (exports, imports, dependencies) for each file.

### 3. Dead Export Detection

Identify exported symbols that are not imported by any other file in the project. For each dead export, report:
- File path and export name
- Whether it is a public API entry point (acceptable) or truly unused
- Suggested action: remove, mark as public API, or investigate

### 4. Hex Boundary Validation

Classify each file into its hexagonal layer:
- **core/domain** — Domain entities and value objects
- **core/ports** — Port interfaces (input and output)
- **core/usecases** — Use case orchestrations
- **adapters/primary** — Driving adapters (CLI, HTTP, etc.)
- **adapters/secondary** — Driven adapters (DB, filesystem, etc.)
- **infrastructure** — Cross-cutting concerns

Validate import direction rules:
- Domain MUST NOT import from ports, usecases, adapters, or infrastructure
- Ports MUST NOT import from usecases, adapters, or infrastructure
- Usecases MAY import from domain and ports only
- Adapters MAY import from ports and domain only
- Infrastructure MAY import from anything

Report every violation with file path, import statement, and the rule violated.

### 5. Circular Dependency Detection

Build a directed dependency graph from all imports. Run DFS cycle detection. For each circular dependency chain found, report:
- The full cycle path (A -> B -> C -> A)
- Suggested resolution strategy (extract interface, event-based decoupling, etc.)

### 6. Compute Health Score

Calculate an overall health score (0-100):
- Start at 100
- Subtract 5 per boundary violation
- Subtract 3 per circular dependency
- Subtract 1 per dead export (non-public-API)
- Minimum score is 0

### 7. Write Report

Write the analysis to `docs/analysis/arch-report.md` with sections for:
- Health score and summary
- Dead exports table
- Boundary violations table
- Circular dependencies list
- Recommendations

## Output

Report the health score, number of violations found, and path to the full report.
