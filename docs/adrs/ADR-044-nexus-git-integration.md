# ADR-044: Nexus Git Integration — Project-Scoped Git Intelligence

**Status:** Accepted
**Accepted Date:** 2026-03-21
## Date: 2026-03-21

> **Implementation Evidence:** `hex-nexus/src/git/` implements the full three-layer integration: status.rs, log.rs, diff.rs, worktree.rs, blame.rs (Layer 1), plus correlation.rs, timeline.rs, poller.rs (Layer 2/3 extras). REST endpoints and WebSocket events are wired.

## Context

hex-nexus is the orchestration hub for hex projects. It tracks registered projects, runs architecture analysis, coordinates swarms, and serves a SolidJS web UI. However, **git integration is effectively absent**:

- `ProjectDetail.tsx` renders hardcoded mock worktrees (lines 16-35)
- `ControlPlane.tsx` shows `worktreeCount: 0` with a comment "Not yet tracked"
- No REST endpoint returns git branch, commit, diff, or status information
- `HexFlo.task_complete()` accepts a `commit_hash` but there's no way to *discover* commits
- ADR-004 (Git Worktrees for Parallel Agent Isolation) is accepted but only the *locking* layer was implemented — not the actual git operations
- The coordination module tracks `WorktreeLock` and `UnstagedFile` state, but these are reported by clients (heartbeat), not read from git

For hex-nexus to function as a true AIIDE (AI IDE), it needs to know what's happening in each project's git repository: current branch, recent commits, dirty files, active worktrees, and diffs.

## Decision

Implement a **three-layer git integration** in hex-nexus:

### Layer 1: Git Query Service (Rust backend)

A new `hex-nexus/src/git/` module that executes git operations against registered project paths. Uses the `git2` crate (libgit2 bindings) for read operations and shells out to `git` CLI for worktree management (libgit2's worktree support is limited).

```
hex-nexus/src/git/
  mod.rs          # GitService struct — holds no state, operates on paths
  status.rs       # Branch, dirty files, ahead/behind tracking remote
  log.rs          # Commit history with pagination
  diff.rs         # File-level and hunk-level diffs
  worktree.rs     # List/create/remove worktrees via git CLI
  blame.rs        # Per-file blame (future, not in v1)
```

**Key design constraint:** GitService is *stateless* — it reads from the filesystem on every call. This avoids stale caches and keeps the source of truth in git itself. The only caching is short-lived (5 second TTL) for expensive operations like `log` with graph rendering.

**Security:** All paths are validated against registered project `root_path` values. GitService refuses to operate on paths not in the project registry. This prevents path traversal attacks via the API.

### Layer 2: REST API Endpoints

All endpoints are project-scoped under `/api/{project_id}/git/...`:

| Endpoint | Method | Returns | Use Case |
|----------|--------|---------|----------|
| `/api/{project_id}/git/status` | GET | Branch, dirty files, ahead/behind, stash count | Dashboard header, health bar |
| `/api/{project_id}/git/log` | GET | Paginated commits (`?limit=20&offset=0&branch=main`) | Commit history panel |
| `/api/{project_id}/git/log/{sha}` | GET | Single commit detail with full diff | Commit detail view |
| `/api/{project_id}/git/diff` | GET | Working tree diff (`?staged=true` for index) | Unstaged/staged changes viewer |
| `/api/{project_id}/git/diff/{base}...{head}` | GET | Diff between two refs | Branch comparison |
| `/api/{project_id}/git/branches` | GET | Local + remote branches with head SHA | Branch picker |
| `/api/{project_id}/git/worktrees` | GET | Active worktrees with branch, path, commit | Worktree panel (replaces mocks) |
| `/api/{project_id}/git/worktrees` | POST | Create worktree (`{branch, path}`) | "New Worktree" button |
| `/api/{project_id}/git/worktrees/{name}` | DELETE | Remove worktree and optionally delete branch | Worktree cleanup |

**Response format** — all endpoints return JSON with consistent structure:

```json
{
  "ok": true,
  "data": { ... },
  "cachedAt": null  // or ISO timestamp if served from cache
}
```

**Pagination** — `git/log` uses cursor-based pagination (SHA of last commit) rather than offset, since commit history can be rewritten.

### Layer 3: WebSocket Events + UI Integration

Git state changes are broadcast via the existing `ws_tx` channel:

```json
{ "topic": "project:<id>:git", "event": "status-changed", "data": { "branch": "main", "dirty": 3 } }
{ "topic": "project:<id>:git", "event": "commit-pushed",  "data": { "sha": "abc1234", "message": "..." } }
{ "topic": "project:<id>:git", "event": "worktree-created", "data": { "branch": "feat/auth", "path": "..." } }
```

**Polling strategy:** A background task polls `git status` every 10 seconds for each registered project (configurable). This is lightweight — `git2` status checks are <5ms for typical repos. WebSocket events fire only on *change*, not on every poll.

### UI Components (SolidJS)

| Component | Location | Data Source |
|-----------|----------|-------------|
| `GitStatusBadge` | Project header | `GET /git/status` + WS |
| `CommitLog` | ProjectDetail tab | `GET /git/log` (paginated) |
| `DiffViewer` | ProjectDetail tab (existing component, now wired) | `GET /git/diff` |
| `WorktreePanel` | ProjectDetail section (replaces mock) | `GET /git/worktrees` + WS |
| `BranchPicker` | Project header dropdown | `GET /git/branches` |

### Integration with Existing Systems

**HexFlo Tasks:** When `task_complete(task_id, result, commit_hash)` is called, the git log endpoint can cross-reference task IDs with commit messages (convention: `feat(task-id): ...`).

**Worktree Locks:** The existing `WorktreeLock` coordination state (in `coordination.rs`) maps to *real* worktrees via the `git/worktrees` endpoint. The lock prevents two agents from claiming the same worktree; the git endpoint shows what's actually on disk.

**Architecture Analysis:** `hex analyze` results are already stored in `ProjectState.health`. The git integration adds *when* violations were introduced by correlating violations with commit history.

**Unstaged Files:** The heartbeat-reported `UnstagedFile` state is replaced by (or validated against) the authoritative `git status` output.

## Implementation Plan

### Phase 1: Read-Only Git Queries (v1)
1. Add `git2` dependency to `hex-nexus/Cargo.toml`
2. Implement `git/status.rs`, `git/log.rs`, `git/diff.rs`, `git/worktree.rs`
3. Add REST routes under `/api/{project_id}/git/...`
4. Wire `GitStatusBadge` and `CommitLog` into `ProjectDetail.tsx`
5. Replace `MOCK_WORKTREES` with real data from `GET /git/worktrees`

### Phase 2: Write Operations + WebSocket (v2)
1. Worktree create/delete via POST/DELETE endpoints
2. Background polling task for git status changes
3. WebSocket broadcast on status change
4. `BranchPicker` and `DiffViewer` components

### Phase 3: Cross-Cutting Intelligence (v3)
1. Commit-to-task correlation (HexFlo task_id in commit messages)
2. Violation-to-commit mapping (when was a boundary violation introduced?)
3. Agent activity timeline overlaid with commit history
4. Blame integration for architecture violations

## Consequences

### Positive

- Projects in the dashboard become *alive* — users see real branch, commit, and dirty-file state
- Worktree panel shows actual agent isolation state, not mocks
- Git history provides audit trail for swarm-generated code
- Foundation for future PR review, merge conflict resolution, and CI integration
- `git2` crate is pure Rust with no runtime dependency on git CLI (except for worktree ops)

### Negative

- `git2` adds ~2MB to the binary and introduces a C dependency (libgit2)
- Polling every 10s across many projects adds minor CPU overhead
- Git operations block the async runtime briefly — may need `spawn_blocking` for large repos
- Write operations (worktree create) require careful error handling for disk space, permissions

### Risks

- **Path traversal:** Mitigated by validating all paths against registered project `root_path` values
- **Large repos:** `git log` on repos with 100k+ commits could be slow — mitigated by mandatory pagination and `--first-parent` for main branch
- **Concurrent writes:** Multiple agents creating worktrees simultaneously — mitigated by the existing `WorktreeLock` mechanism
- **libgit2 vs git CLI divergence:** libgit2 may not support all git features (e.g., partial clone, sparse checkout) — document which operations use CLI fallback

## References

- ADR-004: Git Worktrees for Parallel Agent Isolation
- ADR-027: HexFlo Swarm Coordination
- ADR-011: Coordination and Multi-Instance Locking
- ADR-039: Nexus Agent Control Plane
