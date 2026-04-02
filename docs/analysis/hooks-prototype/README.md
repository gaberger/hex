# Hooks-Based Architecture Enforcement — Proof of Concept

This directory prototypes replacing hex-agent's Rust enforcement with plain shell
scripts wired into Claude Code's PreToolUse / PostToolUse hook system.

## What hex-agent currently does (Rust)

`hex-agent` enforces hexagonal architecture at two points:

1. **Context injection** (`hex_knowledge.rs`) — injects tier-0/1/2 architecture
   rules into the agent's system prompt based on which layer is being edited.
   This is the primary mechanism: the agent *knows* the rules before writing code.

2. **Hook YAMLs** (`hex-cli/assets/hooks/hex/`) — seven hook definitions cover:
   - `hex-boundary-check.yml` — PostToolUse grep for cross-layer imports
   - `hex-architecture-gate.yml` — blocks commits if `hex analyze` fails
   - `hex-specs-required.yml` — blocks new ports without a spec file
   - `hex-no-rest-state-mutation.yml` — blocks direct DB writes via REST
   - `hex-lifecycle-enforcement.yml` — enforces ADR status transitions
   - `hex-merge-validation.yml` — runs full test suite before worktree merge
   - `hex-adr-lifecycle.yml` — warns on abandoned ADRs

3. **MCP tools** (`hex_analyze`, `hex_enforce_list`) — on-demand analysis via
   `hex analyze .` (tree-sitter boundary checker with cycle detection).

## What these shell scripts replace

| Rust / hex-agent mechanism | Shell hook equivalent |
|----------------------------|-----------------------|
| Tier-1 context injection per layer | Not replicated — context is static in prompts |
| `hex-boundary-check.yml` PostToolUse grep | `post-tool-use.sh` (grep on written file) |
| Dangerous bash blocking | `pre-tool-use.sh` (pattern match on command) |
| Layer boundary pre-check | `pre-tool-use.sh` (parse new_string imports) |
| `hex analyze .` tree-sitter analysis | Not replicated — still needs the Rust binary |

## How it works

### pre-tool-use.sh (PreToolUse)

Receives the full tool input JSON on stdin. Two enforcement modes:

**Bash tool** — scans the `command` field for dangerous shell patterns
(`rm -rf /`, `dd if=`, `mkfs.*`, fork bombs, `chmod -R 777`). Returns `deny`
with a human-readable reason if matched.

**Edit / Write tools** — extracts `file_path` and `new_string`/`content`,
classifies the target file's hex layer (domain / ports / usecases /
adapter\_primary / adapter\_secondary), then scans the incoming content for
TypeScript `import ... from` lines that cross the layer boundary rules from
`hex_knowledge.rs`. Returns `deny` before the file is written.

### post-tool-use.sh (PostToolUse)

After every Edit or Write, re-reads the written file from disk and runs the
same import-line grep. If violations are found, returns `additionalContext`
with a formatted warning message including line numbers. This catches cases
where pre-tool-use missed something (e.g. content injected by multi-part edits).

## Wiring into settings.json

Merge `settings-snippet.json` into your project's `.claude/settings.json`
(or `~/.claude/settings.json` for global enforcement):

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash|Edit|Write",
        "hooks": [{ "type": "command", "command": "/path/to/pre-tool-use.sh" }]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Edit|Write",
        "hooks": [{ "type": "command", "command": "/path/to/post-tool-use.sh" }]
      }
    ]
  }
}
```

Make the scripts executable: `chmod +x pre-tool-use.sh post-tool-use.sh`

## Limitations vs. hex-agent

1. **No tree-sitter** — import detection is grep-based; aliased re-exports,
   barrel files, and dynamic `import()` calls are not caught. `hex analyze .`
   remains the authoritative checker.

2. **No cross-file cycle detection** — the Rust analyzer builds a full import
   graph and detects cycles; shell can only check one file at a time.

3. **No Rust import support** — Rust `use` statements have a different grammar
   (path segments with `::`, glob imports `*`, nested `{a, b}`). The scripts
   have partial Rust support but it is not comprehensive.

4. **No context injection** — the main power of hex-agent is injecting the
   right layer rules into the agent prompt *before* it writes code. Shell hooks
   can only react after the fact (PostToolUse) or block at tool invocation time
   (PreToolUse), not modify the system prompt.

5. **No ADR / spec gates** — the `hex-specs-required` and `hex-adr-lifecycle`
   hooks depend on querying the hex-nexus REST API; shell scripts would need
   `curl` calls to replicate this.

## Verdict

Shell hooks cover ~40% of what hex-agent does: dangerous-command blocking and
simple import-line scanning work well. The remaining 60% (tree-sitter analysis,
cross-file cycles, context injection, ADR gates) still requires the Rust daemon.
The practical hybrid is: shell hooks as a fast first-pass guard, `hex analyze .`
as the authoritative gate before commit.
