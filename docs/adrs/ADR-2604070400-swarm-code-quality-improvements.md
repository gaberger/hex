# ADR-2604070400: Swarm Code Generation Quality Improvements

## Status
Proposed

## Context
The hex dev swarm pipeline is fully functional (v26.4.16): worker→claim→generate→complete→STDB sync→supervisor iterate→fixer→compile check. But the generated code doesn't pass `hex ci`. The `qwen3:8b` model hallucinates file names, ignores hex layer rules, and generates off-topic code. After 8 fixer iterations the pipeline halts with compile errors.

The infrastructure works — the LLM output quality doesn't.

## Decision

### P0: Coder Prompt Template (biggest impact)

Current coder prompt is too vague. The model needs:

1. **Explicit file paths** — tell the model exactly where to write:
   ```
   Write the file: src/adapters/secondary/InMemoryTaskRepository.ts
   ```
   Not: "Implement an in-memory task repository"

2. **Project structure context** — inject the current directory tree so the model knows what exists:
   ```
   Current project structure:
   src/core/domain/types.ts (exists)
   src/core/domain/task.ts (exists)
   src/core/ports/ITaskRepository.ts (exists)
   src/adapters/secondary/InMemoryTaskRepository.ts ← WRITE THIS FILE
   ```

3. **Hex layer rules in system prompt** — hard constraints:
   ```
   RULES:
   - Domain files go in src/core/domain/
   - Port interfaces go in src/core/ports/
   - Secondary adapters go in src/adapters/secondary/
   - Primary adapters go in src/adapters/primary/
   - Use cases go in src/usecases/
   - Domain MUST NOT import from adapters
   - Adapters MUST NOT import other adapters
   ```

4. **Import resolution** — include actual exports from existing files:
   ```
   Available imports:
   - from '../domain/types.js': Task, TaskId, Status, Priority
   - from '../ports/ITaskRepository.js': ITaskRepository
   ```

5. **TypeScript strict mode** — "strict TypeScript, no `any` types, use `import type` for type-only imports"

**Implementation**: Modify `hex-cli/src/pipeline/code_phase.rs` `build_prompt()` to inject these from the workplan step metadata + AST summary.

### P1: Two-Model Strategy (RL tiering)

| Task Type | Model | Speed | Rationale |
|-----------|-------|-------|-----------|
| ADR, workplan, review | qwen3:8b | 34 tok/s | Planning needs speed, not precision |
| Code generation | qwen2.5-coder:32b | ~8 tok/s | Purpose-built for code, much higher quality |
| Quick fixes, lint | qwen3:4b | 63 tok/s | Simple transformations |

**Implementation**: 
- Supervisor reads model tier from agent YAML `inference.task_type`
- RL engine tracks `CodeCompiles` pass/fail per model → learns which model works for which task
- `upgrade: { after_iterations: 2, to: qwen2.5-coder:32b }` in hex-coder.yml

### P2: Fixer Loop Detection

After 2 iterations with identical fixer output, escalate:
1. Switch to higher-quality model
2. If still stuck after 2 more iterations, include the compile error in the CODER prompt (not fixer) and regenerate from scratch

**Implementation**: `hex-cli/src/pipeline/supervisor.rs` `run_tier()` — hash fixer output, compare with previous, escalate on match.

### P3: File Path Sanitization

Worker generated filenames containing newlines. Add validation:
```rust
let sanitized = raw_path.replace('\n', "").replace('\r', "");
if sanitized.contains("..") || sanitized.starts_with('/') {
    return Err("invalid file path");
}
```

**Implementation**: `hex-cli/src/commands/agent.rs` worker file write path.

### P4: `hex dev status` Command

Live progress view:
```
⬡ hex dev status

  Session: 7833170d (Build task tracker)
  Model:   qwen3-8b via openai-compat
  
  ━━━ Tier 0 (domain + ports) ━━━━━━━━━━━━━━━━━━━
  Iteration: 4/8
  ✓ CodeGenerated
  ✗ CodeCompiles (2 errors)
  ✗ TestsExist
  ⊘ TestsPass
  ✗ ReviewPasses
  ✓ ArchitectureGradeA (80/100)
  
  Current: hex-fixer addressing CodeCompiles
```

**Implementation**: Read active session from `~/.hex/sessions/`, query swarm status from nexus.

### P5: Provider Persistence

Save registered inference providers to `.hex/inference-providers.json`. Nexus loads on startup.

## Consequences
- P0 alone should get code quality from "doesn't compile" to "compiles with minor issues"
- P1 gives the pipeline self-healing capability — bad model → upgrade → retry
- P2 prevents wasting 8 iterations on unfixable errors
- P3 prevents filesystem corruption from malformed LLM output
- P4 gives developers visibility into swarm progress
- P5 eliminates the "re-register after restart" operational tax
