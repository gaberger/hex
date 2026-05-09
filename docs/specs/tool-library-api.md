# Tool library API spec — first wave

ADR-2605082500. Each tool is a Rust function with typed input/output that
also exports a JSON schema for Anthropic function-calling. The `Tool`
trait + `ToolRegistry` live in `hex-nexus/src/tools/`.

## Tool trait

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// JSON schema (Anthropic function-calling format) for the input.
    fn input_schema(&self) -> serde_json::Value;
    /// Execute the tool with the given JSON input. Must be deterministic
    /// for the same input (callers may cache).
    async fn execute(&self, input: serde_json::Value) -> ToolResult;
}

pub struct ToolResult {
    pub ok: bool,
    pub output: serde_json::Value,
    pub error: Option<String>,
    pub elapsed_ms: u64,
}
```

## First 4 tools

### cargo_check

```
name:        cargo_check
description: Run `cargo check` on a single crate; return errors + warnings.
input:       { crate: string, target?: string ("dev" default) }
output:      { ok: bool, errors: [{file, line, col, msg, level}], warnings: [...] }
constraints: 60s timeout, run from repo root, --message-format=json
```

### repo_grep

```
name:        repo_grep
description: Run ripgrep over the repo; return matching file:line snippets.
input:       { pattern: string, glob?: string, max_matches?: u32 (default 50) }
output:      { matches: [{path, line, content}], total_matches: u32, truncated: bool }
constraints: 5s timeout, max 500 lines returned, no binary files
```

### adr_draft

```
name:        adr_draft
description: Draft a new ADR. Writes a proposed_action(kind=file_write) row;
             the digital-twin executor materialises the file after approval.
input:       { id: string, title: string, status: "proposed"|"accepted"|"superseded",
               body: string }
output:      { proposed_action_id: u64, target_path: string }
constraints: id MUST match /^[0-9]{10,}$/ (timestamp form), title <= 80 chars,
             body 200-50000 bytes, must include "## Context", "## Decision",
             "## Consequences" sections (verifier checks).
```

### escalate_to_operator

```
name:        escalate_to_operator
description: When the persona genuinely cannot proceed (paradigm question,
             ambiguous ask, novel domain), emit an operator inbox notification.
             Priority-2; surfaces in dashboard within 4s.
input:       { reason: string, urgency: "low"|"med"|"high",
               options?: [string] (proposed paths the operator can pick from) }
output:      { inbox_id: u64 }
constraints: reason <= 500 chars, options <= 6 items, no PII / secrets in payload
```

## SOP wiring

Phase 3 REASON sends the registered tool list with the inference request.
The Anthropic API returns a sequence of `tool_use` blocks; the executor
calls each tool, feeds results back as `tool_result`, and continues until
the model returns a non-tool message OR emits a final structured action
via the special `final_action` tool (one of `adr_draft`,
`escalate_to_operator`, or `acknowledge_no_action`).

Tool call depth cap: 8 round trips. Beyond that, escalate.

## Validation invariants

1. Every tool has a JSON schema. `cargo test tools::schema_validation` checks every registered tool's schema parses as a valid Anthropic function-call definition.
2. `cargo_check` is the verifier of last resort. Phase 4's adr_schema_validate AND a `cargo_check` of the repo (no regression) gates every emit.
3. Tool outputs are bounded (no unbounded streams). Each tool truncates + sets `truncated: true` when limits hit.
4. The persona prompt does NOT include free-form output instructions. The ONLY reply channels are the registered tools.

## Wave 2 (queued for tomorrow's first push)

`repo_read`, `git_log`, `cargo_test`, `cargo_clippy`, `analyze_deps` (wraps hex-analyzer), `adr_search`, `workplan_emit`, `boundary_check`, `merge_request_open`, `swarm_init`. Each follows the same Tool trait shape.
