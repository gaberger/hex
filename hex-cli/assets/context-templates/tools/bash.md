Executes a given bash command and returns its output. The working directory persists between commands, but shell state does not.

AVOID using for: cat, head, tail, sed, awk, echo — use dedicated tools instead.

## hex-specific rules

**Bash is reserved for:**
- `git` operations (status, log, diff, add, commit, push)
- `cargo build / cargo test / cargo check` — Rust build and test
- `mkdir / rm / mv` — directory and file lifecycle
- `hex` CLI commands — architecture checks, swarm ops, nexus control
- Short-output system commands (process management, env inspection)

**NEVER use Bash for:**
- Reading files → use Read
- Searching file content → use Grep
- Finding files by pattern → use Glob
- Writing or creating files → use Write or Edit
- Any command likely to produce >20 lines of output → use hex MCP batch tools

## Git rules

- Prefer creating new commits rather than amending
- Never skip hooks (`--no-verify`) unless explicitly asked
- Never force-push to main/master
- Always stage specific files — avoid `git add -A` or `git add .`
- Background worktree agents MUST explicitly commit: `git add <files> && git commit`

## For PRs and GitHub

Use `gh` CLI for ALL GitHub-related tasks (issues, PRs, checks, releases).

## Cargo (Rust builds)

Use `cargo build` (debug) during iterative development — not `--release`.
Run `cargo test -p <crate>` to test a single crate, not the entire workspace.

## hex CLI commands

```bash
hex analyze .            # Architecture health check
hex nexus status         # Check nexus daemon health
hex adr list             # List ADRs
hex swarm init <name>    # Initialize a HexFlo swarm
hex task list            # List HexFlo tasks
hex memory store <k> <v> # Persist key-value across sessions
hex inbox list           # Check agent notifications (ADR-060)
```
