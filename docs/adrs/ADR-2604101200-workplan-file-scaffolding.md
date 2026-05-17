# ADR-2604101200: Workplan File Scaffolding — Create Files Before Agents Edit

**Status:** Accepted
**Date:** 2026-04-10
**Drivers:** Workplan execution failed because agents were spawned to "modify" files that don't exist yet. The execute_phase() spawns agents but never checks if target files exist first. This is the fundamental flaw hex is trying to fix — autonomous agents that can't create from scratch.
**Supersedes:** None

## Context

The managed-agents-autonomy workplan (ID: `ff3a19c0-c841-40ac-a092-8bc64d484d0c`) executed but all 6 tasks in P0 failed with:

```
Task 'T0.1: SSE streaming handler in hex-nexus': hex-agent exited with exit status: 1
Task 'T0.2: SpacetimeDB subscription infrastructure': hex-agent exited with exit status: 1
... (all 6 tasks in P0 failed the same way)
```

The workplan tasks listed files to create/modify:
- `hex-nexus/src/routes/sse.rs` (doesn't exist)
- `hex-agent/src/mcp/tools/bash.rs` (doesn't exist)
- `hex-nexus/src/routes/steering.rs` (doesn't exist)

Tracing the code in `workplan_executor.rs`:

1. `execute_phase()` builds a prompt from task name + description + files list
2. Spawns a hex-agent with that prompt via Path B (inference queue)
3. The agent receives: "create/modify these files" but they don't exist
4. Agent tries to use Claude Code Edit/Write tools on non-existent files → exit 1

**The fundamental flaw:** `execute_phase()` has no pre-flight check for file existence.

## Impact Analysis

### Consumer Dependency Map

| Component | Role | Issue |
|-----------|------|-------|
| `workplan_executor.rs:execute_phase()` | Spawns agents | Never checks if target files exist |
| `WorkplanTask.files` | Lists target files | Only used in prompt; no existence check |
| Agent tools (Edit/Write) | Modify files | Require file to exist first; fail otherwise |
| Path B inference queue | Routes to CC session | Doesn't scaffold files |

### Cross-Crate Dependencies

| Dependency | How Affected |
|------------|---------------|
| hex-nexus (orchestration) | execute_phase() doesn't scaffold |
| hex-agent (agent runtime) | Receives impossible task |
| hex-cli (workplan execution) | Uses workplan_executor from hex-nexus |

### Blast Radius

| Component | Consumers | Impact | Mitigation |
|-----------|-----------|--------|------------|
| execute_phase() | All workplan executions | CRITICAL — breaks all new-feature workplans | Add scaffolding step |
| WorkplanTask.files | All task definitions | HIGH — empty for new features | Validate at parse time |
| Agent tools | All agents | HIGH — can't create new files | N/A (agent limitation) |

### Build Verification Gates

- Pre-flight: verify file existence before spawn
- Post-spawn: if file didn't exist, verify it was created
- Phase gate: `ls -la {files}` to verify existence

## Decision

We will add a **File Scaffolding Step** to `execute_phase()` that runs BEFORE agent spawn:

### 1. Pre-flight: File Existence Check

In `execute_phase()`, before spawning agents, for each task:

```rust
// For each file in task.files:
for file in &task.files {
    if !Path::new(file).exists() {
        // Check if this is a NEW file (not modification)
        // by verifying it's not in any known crate
        let is_new = !file_exists_in_crate(file);
        
        if is_new {
            // Scaffolding needed: create empty file with module structure
            scaffold_file(file)?;
        }
    }
}
```

### 2. Workplan Task Classification

Classify tasks by whether they create NEW files vs modify existing:

| Classification | Criteria | Handling |
|---------------|----------|----------|
| **NEW** | File not in any crate's Cargo.toml | Scaffold first, then edit |
| **MODIFY** | File exists in crate | Direct edit |
| **CREATE** | Non-Rust/TSE file (e.g., `.html`) | Direct create |

### 3. Skeleton Generation

Scaffold new files with proper module structure:

```rust
fn scaffold_file(path: &str) -> Result<(), String> {
    match path.extension().unwrap_or_default().to_str() {
        "rs" => {
            // Parse module name, generate `pub mod {name};` structure
            let module = path.file_stem().unwrap();
            let content = format!("// Auto-scaffolded module\npub mod {};\n", module);
            path.write(&content)?;
        }
        "ts" | "tsx" => {
            // Generate TypeScript module
            let content = "// Auto-scaffolded\nexport {};\n";
            path.write(&content)?;
        }
        _ => {
            // Empty file for docs, configs
            path.write("")?;
        }
    }
    Ok(())
}
```

### 4. Prompt Enrichment

After scaffolding, update the agent prompt to indicate files were scaffolded:

```rust
// In execute_phase(), after scaffolding:
let prompt = format!(
    "## File scaffolded (files did not exist, created skeleton)\n{}\n\n## Original task\n{}",
    scaffolded_files.join("\n"),
    original_prompt
);
```

## Consequences

**Positive:**
- Workplans can create NEW features from scratch
- No more "file not found" failures for new feature workplans
- Consistent file structure via scaffolding templates

**Negative:**
- Agent editing scaffolded file may conflict with scaffolding structure
- Need to handle file path conventions across multiple languages

**Mitigations:**
- Scaffold with minimal structure (just module declaration)
- Agent can replace entire file content
- Per-language scaffolding templates

## Implementation

| Phase | Description | Validation Gate | Status |
|-------|-------------|-----------------|--------|
| P1 | Add `scaffold_file()` function to hex-nexus | `cargo check -p hex-nexus` | Pending |
| P2 | Add file existence check in `execute_phase()` | `ls -la` check | Pending |
| P3 | Classify task as NEW vs MODIFY | Works for existing | Pending |
| P4 | Prompt enrichment with scaffolded file list | Test run | Pending |
| P5 | Integration test: run managed-agents workplan again | All tasks pass | Pending |

## References

- ADR-2604050900: Module deletion consumer analysis
- ADR-2603291900: Docker worker first-class execution
- Original workplan: `docs/workplans/feat-managed-agents-autonomy.json`
- Failed execution: `ff3a19c0-c841-40ac-a092-8bc64d484d0c`