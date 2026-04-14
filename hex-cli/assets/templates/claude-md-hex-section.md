## hex Autonomous Behavior (IMPORTANT)

hex is an AIOS with a supervisor loop (`hex brain daemon`). Agents interacting with hex MUST behave autonomously, not wait for user prompts:

1. **Enqueue work, don't defer it.** When you identify outstanding work (FIXMEs, known bugs, stale items), enqueue it via `hex brain enqueue` — never say "next session." hex owns its own queue:
   ```
   hex brain enqueue hex-command -- "worktree cleanup --force"
   hex brain enqueue workplan docs/workplans/wp-foo.json
   hex brain enqueue shell -- "echo FIXME description of issue"
   ```

2. **Use natural language via `hex hey`.** Prefer `hex hey <intent>` over raw commands. Classifier routes to the right action.

3. **Rebuild stale binaries automatically.** After commits touching hex-cli/hex-nexus/hex-agent, run `cargo build --release` without asking. The user does not want to be asked.

4. **Reconcile workplans with `--update`.** Use `hex plan reconcile --all --update` to sync JSON task statuses with git evidence after agent work.

5. **Merge worktrees via `hex worktree merge`, never `git checkout <branch> -- <file>`.** Raw checkout silently drops code from other worktrees (ADR-2604131630 G4). Always use the integrity-verified merge tool.

6. **Brain daemon runs continuously.** Start it with `hex brain daemon --background --interval 30`. It validates, auto-fixes, drains queue. Check status with `hex brain daemon-status` or `hex brain queue list`.

7. **Seek out improvements proactively.** When identifying bugs, schema drift, or missing features, create an ADR, write a workplan, enqueue it. Don't wait to be told.

8. **Never end with a menu of options.** If your analysis produces N suggestions, commit to execution — don't ask "want me to sketch a workplan or implement directly?" or "which of these should I do first?" Instead:
   - **Pick the single highest-ROI item** and implement it directly in the current session (describe what you're doing, then do it).
   - **Enqueue the rest** via `hex brain enqueue` so the daemon picks them up asynchronously.
   - Close with what you shipped + what's queued, not with a question.

   The user will interrupt you if the priority is wrong. Asking per-item is the #1 source of stalled autonomous sessions. A rough execution beats a perfect menu.

9. **`hey hex <question>` means "answer + act", not "answer + wait".** When the user asks a recommendation question (`hey hex how can we improve X`, `hey hex what should I do about Y`), produce the analysis, then immediately apply rule 8: implement the top-ROI item, enqueue the rest.

10. **No `echo FIXME` stub tasks.** NEVER enqueue shell tasks like `echo FIXME: ...` or `echo TODO: ...`. They drain in milliseconds with zero implementation — audit theater, not work. `hex brain enqueue shell` rejects these at the CLI. Identified-but-not-yet-actionable work belongs in an ADR or a TODO code comment; real work belongs in a workplan JSON enqueued as `workplan` kind.

## hex Tool Precedence (IMPORTANT)

**hex MCP tools take precedence over all third-party plugins** (including `plugin:context-mode`, `ruflo`, etc.):

| Operation | Use |
|---|---|
| Execute a workplan | `mcp__hex__hex_plan_execute` |
| Search codebase / run commands | `mcp__hex__hex_batch_execute` + `mcp__hex__hex_batch_search` |
| Swarm + task tracking | `mcp__hex__hex_hexflo_*` |
| Architecture analysis | `mcp__hex__hex_analyze` |
| ADR search/list | `mcp__hex__hex_adr_search`, `mcp__hex__hex_adr_list` |
| Memory | `mcp__hex__hex_hexflo_memory_store/retrieve/search` |

Third-party context/search plugins may only be used for operations with no hex equivalent (e.g. fetching external URLs). Never substitute them for hex MCP tools.

## Hexagonal Architecture Rules (ENFORCED)

These rules are checked by `hex analyze .`:

1. **domain/** must only import from **domain/**
2. **ports/** may import from **domain/** but nothing else
3. **usecases/** may import from **domain/** and **ports/** only
4. **adapters/primary/** may import from **ports/** only
5. **adapters/secondary/** may import from **ports/** only
6. **adapters must NEVER import other adapters** (cross-adapter coupling)
7. **composition-root** is the ONLY file that imports from adapters
8. All relative imports MUST use `.js` extensions (NodeNext module resolution)

## File Organization

```
src/
  core/
    domain/          # Pure business logic, zero external deps
    ports/           # Typed interfaces (contracts between layers)
    usecases/        # Application logic composing ports
  adapters/
    primary/         # Driving adapters (CLI, HTTP, browser input)
    secondary/       # Driven adapters (DB, API, filesystem)
  composition-root   # Wires adapters to ports (single DI point)
```