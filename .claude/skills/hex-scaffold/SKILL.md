---
name: hex-scaffold
description: Scaffold a new hexagonal architecture project. Use when the user asks to "create hex project", "scaffold hexagonal", "new ports and adapters project", "hex-intf init", or "init hexagonal project".
---

# Hex Scaffold — Create a Hexagonal Architecture Project

## Parameters

Ask the user for:
- **name** (required): Project name for directory and package naming
- **language** (optional, default: typescript): One of `typescript`, `go`, or `rust`
- **adapters** (optional, default: cli, http, filesystem, git): List of adapters to scaffold

## Step 1: Create Directory Structure

Run all mkdir commands in a single bash call:

```bash
mkdir -p {name}/src/core/{domain,ports,usecases}
mkdir -p {name}/src/adapters/{primary,secondary}
mkdir -p {name}/src/infrastructure/{treesitter,swarm,worktree}
mkdir -p {name}/tests/{unit,integration}
mkdir -p {name}/docs/{architecture,adrs,skills}
mkdir -p {name}/config
mkdir -p {name}/scripts
mkdir -p {name}/examples
```

## Step 2: Generate Port Interfaces

Create typed port interfaces in `src/core/ports/index.ts` (or language equivalent). Include these ports:

**Input Ports (primary):**
- `ICodeGenerationPort` — generateFromSpec, refineFromFeedback
- `IWorkplanPort` — createPlan, executePlan
- `ISummaryPort` — summarizeFile, summarizeProject

**Output Ports (secondary):**
- `IASTPort` — extractSummary, diffStructural
- `ILLMPort` — prompt, streamPrompt
- `IBuildPort` — compile, lint, test
- `IWorktreePort` — create, merge, cleanup, list
- `IGitPort` — commit, createBranch, diff, currentBranch
- `IFileSystemPort` — read, write, exists, glob

All methods must use proper types: Promise-based returns, typed parameters, domain value objects.

## Step 3: Generate Domain Entities

Create domain entity stubs in `src/core/domain/`:
- Entities: Project, CodeUnit, TestSuite, BuildResult
- Value Objects: Language, ASTSummary, TokenBudget, QualityScore

## Step 4: Scaffold Adapter Stubs

For each adapter in the adapters list, create:
- Implementation file in `src/adapters/{primary|secondary}/{adapter-name}/index.{ext}`
- Unit test file in `tests/unit/{adapter-name}.test.{ext}` (London-school mocks)

Each adapter must implement its corresponding port interface with dependency injection.

## Step 5: Setup Build Configuration

**TypeScript:**
- `tsconfig.json` with strict mode, noUncheckedIndexedAccess, outDir: dist
- `package.json` with scripts: build (tsc), lint (eslint), test (vitest run), test:watch (vitest)
- `vitest.config.ts`
- `.gitignore` (node_modules, dist, .env)

**Go:**
- `go.mod`, `Makefile`, `.golangci.yml`

**Rust:**
- `Cargo.toml`, `clippy.toml`, `rust-toolchain.toml`

## Step 6: Install Dependencies

For TypeScript: `bun install` or `npm install` with hex-intf and tree-sitter-wasms as dependencies.

## Step 7: Verify Scaffold

Run the compile and lint commands to verify everything builds cleanly:
- TypeScript: `npx tsc --noEmit && npx eslint src/ --ext .ts`
- Go: `go build ./... && golangci-lint run`
- Rust: `cargo check && cargo clippy -- -D warnings`

Fix any errors in a feedback loop (max 3 iterations).

## Output

Report: project name, language, files created, and next steps:
1. `cd {name} && npm install` (or go mod tidy / cargo build)
2. Generate adapters with `hex-generate` skill
3. View structure with `hex-summarize` skill at L1
