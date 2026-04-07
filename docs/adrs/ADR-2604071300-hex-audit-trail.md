# ADR-2604071300: Unified Hex Dev Audit Trail via SpacetimeDB

## Status
Accepted

## Context
The hex dev pipeline produces code that compiles and passes tests (ADR-2604070400), but the audit trail is broken. Three data layers exist but never integrate:

1. **Session layer** (`~/.hex/sessions/dev/*.json`) — local JSON files that record tool_calls and status, but `completed_steps` is always empty, `quality_result` is always null, and `status` stays "in_progress" after completion
2. **Supervisor layer** (in-memory) — builds comprehensive quality_result from objective evaluations but never persists it anywhere
3. **Nexus layer** (SpacetimeDB) — has `swarm_task`, `quality_gate_task`, `fix_task`, `inference_task` tables but no `dev_session` table to tie them together

**The fundamental problem:** Session state lives in local JSON files instead of SpacetimeDB. Per ADR-046, SpacetimeDB is the single source of truth — the dashboard, CLI, and MCP tools should all read from the same place. Local files can't be queried from the dashboard, can't be subscribed to via WebSocket, and don't sync across hosts.

**Existing SpacetimeDB tables (hexflo-coordination module):**

| Table | Has | Missing |
|-------|-----|---------|
| `swarm` | id, project, topology, status, owner | — |
| `swarm_task` | id, swarm_id, title, status, agent_id, result | model, tokens, cost, duration |
| `quality_gate_task` | id, swarm_id, tier, gate_type, status, score, grade | session linkage |
| `fix_task` | id, gate_task_id, model_used, tokens, cost | — |
| `inference_task` | id, workplan_id, phase, prompt, status, result | tokens, cost, duration |
| **`dev_session`** | **DOES NOT EXIST** | — |

**Concrete bugs observed on Bazzite v26.4.17:**
- `hex report list` shows session as "in_progress" after pipeline finished
- `hex plan report <id>` returns 404 — workplan ID != session ID
- `completed_steps: []` — never populated (0 assignments in codebase)
- `quality_result: null` — supervisor builds it but never persists
- `model`/`tokens`/`cost` null in code phase tool_calls
- Context window sizes not tracked
- Dashboard can't see session data (it's in local files, not STDB)

## Decision

### Principle: SpacetimeDB is the single source of truth (ADR-046)

All audit data flows through SpacetimeDB reducers. Local JSON files become a read cache / offline fallback only. The dashboard gets real-time session visibility via WebSocket subscriptions.

### P0: Add `dev_session` table to hexflo-coordination WASM module

New table in `spacetime-modules/hexflo-coordination/src/lib.rs`:

```rust
#[table(name = dev_session, public)]
pub struct DevSession {
    #[primary_key]
    pub id: String,
    pub project_id: String,
    pub feature_description: String,
    /// "pending", "adr", "workplan", "scaffold", "code", "validate", "completed", "failed"
    pub status: String,
    pub current_phase: String,
    pub model: String,
    pub provider: String,
    pub adr_path: String,
    pub workplan_path: String,
    pub swarm_id: String,
    pub output_dir: String,
    pub agent_id: String,
    pub total_tokens: u64,
    /// Cost stored as string for WASM f64 compatibility
    pub total_cost_usd: String,
    pub architecture_grade: String,
    pub architecture_score: u32,
    /// Comma-separated completed step IDs
    pub completed_steps: String,
    /// Comma-separated objective verdicts: "CodeGenerated:pass,CodeCompiles:pass,..."
    pub objective_results: String,
    pub created_at: String,
    pub updated_at: String,
}
```

Reducers:
- `session_create(id, project_id, feature, model, provider)` — called at `hex dev start`
- `session_update_phase(id, phase)` — called at each phase transition
- `session_complete_step(id, step_id)` — appends to completed_steps CSV
- `session_set_quality(id, grade, score, objectives)` — called after supervisor evaluates
- `session_finalize(id, status)` — called at pipeline end ("completed" or "failed")

### P1: Add `inference_log` table for per-call tracking

```rust
#[table(name = inference_log, public)]
pub struct InferenceLog {
    #[primary_key]
    pub id: String,
    pub session_id: String,
    pub phase: String,
    pub model: String,
    pub provider: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Cost stored as string for WASM f64 compatibility
    pub cost_usd: String,
    pub duration_ms: u64,
    /// Context window size of the model
    pub context_window: u64,
    /// What was generated: file path, ADR path, workplan path
    pub artifact: String,
    pub status: String,
    pub created_at: String,
}
```

Reducer: `inference_log_create(...)` — called after every inference/complete response.

### P2: Wire supervisor → SpacetimeDB via hex-nexus REST

The supervisor already talks to hex-nexus REST for swarm/task operations. Add endpoints:

| Endpoint | Purpose |
|----------|---------|
| `POST /api/dev-sessions` | Create session (calls `session_create` reducer) |
| `PATCH /api/dev-sessions/:id` | Update phase/status (calls `session_update_phase`) |
| `POST /api/dev-sessions/:id/steps` | Mark step complete |
| `POST /api/dev-sessions/:id/quality` | Set architecture grade + objectives |
| `POST /api/dev-sessions/:id/finalize` | Set final status |
| `POST /api/dev-sessions/:id/inference-log` | Log an inference call |
| `GET /api/dev-sessions/:id/report` | Full audit report (joins all tables) |
| `GET /api/dev-sessions` | List all sessions (for `hex report list`) |

### P3: Update supervisor to call session endpoints

Key insertion points in existing code:

| Location | Call |
|----------|------|
| `tui/mod.rs` pipeline start | `POST /api/dev-sessions` |
| `supervisor.rs` after `execute_step()` returns | `POST /api/dev-sessions/:id/inference-log` with CodeStepResult fields |
| `supervisor.rs` after each workplan step dispatch | `POST /api/dev-sessions/:id/steps` |
| `supervisor.rs` `to_quality_report()` | `POST /api/dev-sessions/:id/quality` |
| `tui/mod.rs` `finalize_session()` | `POST /api/dev-sessions/:id/finalize` |
| ADR/workplan phase completion | `PATCH /api/dev-sessions/:id` with phase transition |

### P4: Unified `hex report show` reads from SpacetimeDB

`hex report show <id>` calls `GET /api/dev-sessions/:id/report` which joins:

- `dev_session` — top-level metadata
- `inference_log` — per-call token/cost/duration breakdown
- `swarm_task` — task completion status
- `quality_gate_task` — compile/test/analyze results
- `fix_task` — fixer iterations and model upgrades
- Git correlation (from hex-nexus git module)

Output format:
```
hex dev — Audit Report
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  Session:     9748761b
  Feature:     Go REST API bookmark manager
  Status:      completed
  Duration:    9m 31s
  Cost:        $0.00 (local inference)

  Model:       qwen3:8b via ollama
  Provider:    ollama @ localhost:11434

  ── Phases ────────────────────────────────
  ✓ ADR          44s    1,838 tok   docs/adrs/ADR-2604071201-*.md
  ✓ Workplan     71s    3,665 tok   docs/workplans/feat-*.json (8 steps)
  ✓ Scaffold      2s       —        go.mod, cmd/, internal/
  ✓ Code Gen    199s   15,024 tok   5 files generated
  ✓ Fixer         0s       —        0 iterations (passed first try)

  ── Quality Gates ─────────────────────────
  ✓ go build ./...     3.8s
  ✓ go vet ./...       2.0s
  ✓ go test ./...      1.8s

  ── Architecture ──────────────────────────
  Grade: A+ (100/100)
  Violations: 0

  ── Objectives ────────────────────────────
  ✓ CodeGenerated      5 Go source files
  ✓ CodeCompiles       0 errors
  ✓ TestsExist         1 test file
  ✓ TestsPass          1/1 passed
  ⊘ ReviewPasses       (skipped)

  ── Inference Log ─────────────────────────
  #  Phase       Model      In→Out       Time   Context%
  1  adr         qwen3:8b    838→1000    44s    6%
  2  workplan    qwen3:8b   1789→1876    71s    11%
  3  code/red    qwen3:8b   2055→1805    70s    13%
  4  code/green  qwen3:8b   2313→1154    44s    14%
  5  code/refact qwen3:8b   2391→1443    33s    15%

  Total: 5,503 tokens | $0.00 | 9m 31s
```

### P5: Dashboard real-time session view

With `dev_session` in SpacetimeDB, the dashboard gets free real-time updates via WebSocket subscription. Add a "Dev Sessions" panel showing:
- Active sessions with live phase indicator
- Historical sessions with grade, cost, duration
- Click-through to full inference log

### P6: Local JSON as offline fallback only

Keep `~/.hex/sessions/dev/*.json` as a write-through cache for offline/disconnected operation. On reconnect, sync local state → SpacetimeDB via `session_create`/`session_finalize` reducers.

## Migration

1. Publish updated `hexflo-coordination` WASM module with new tables
2. hex-nexus adds REST endpoints (P2)
3. Supervisor writes to STDB endpoints instead of local JSON (P3)
4. `hex report` reads from STDB (P4)
5. Local JSON becomes fallback (P6)
6. Existing sessions can be imported via `hex report import` (reads JSON, calls reducers)

## Consequences
- **Single source of truth** — all clients (CLI, dashboard, MCP) see the same data
- **Real-time visibility** — dashboard shows live session progress via WebSocket
- **Cross-host visibility** — Bazzite sessions visible from Mac dashboard
- **Queryable** — can aggregate cost/tokens across all sessions, compare models
- **Audit-grade** — complete provenance from feature request → ADR → workplan → code → gates → grade
- **Breaking change** — `hex report` output format changes; old JSON sessions need import
