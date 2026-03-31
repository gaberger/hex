# hex Gap Analysis Report — 2026-03-31

**Updated:** 2026-03-31 (v2 — ground-truth assessment using `hex project report` + code-presence verification).

> **Methodology change from v1:** v1 relied solely on workplan `"status"` JSON fields, which were often stale.
> v2 uses two authoritative sources:
> 1. **SpacetimeDB execution state** — `hex project report` shows actual pipeline phase progress (P0/N = never executed through the pipeline)
> 2. **Code-presence verification** — key deliverable files from each workplan were checked for existence and content
>
> "Pipeline executed" and "code exists" are independent axes. Many workplans have complete code from prior runs or manual implementation, even when SpacetimeDB shows P0/N.

---

## Summary

| Category | Total | Done (code verified) | Partial | Not Started | Stale |
|----------|-------|---------------------|---------|-------------|-------|
| ADRs | 114 | 97 accepted | — | 12 proposed | 5 deprecated |
| Workplans (active) | 20 | 14 | 4 | 1 | 1 |

**Swarm state (from `hex project report`):** 161 swarms · 603/1,090 tasks done · 1 failed

---

## Active Workplan Assessment (All 20)

These workplans all show P0/N in SpacetimeDB (pipeline phases not tracked), but most have complete code.

| Workplan | Pipeline | Code | Classification | Notes |
|----------|----------|------|----------------|-------|
| feat-docker-sandbox-agent-coordination | P0/4 | ✓ Complete | **DONE** | Dockerfile, sandbox.yml, hook gate, docker-first spawn, MCP tests — all verified |
| feat-declarative-swarm-agent-behavior | P0/5 | ✓ Complete | **DONE** | agent_def.rs, 14 YAMLs, supervisor wired |
| feat-tabled-cli-output | P0/4 | ✓ Complete | **DONE** | fmt.rs, tabled dep, all CLI commands migrated |
| feat-fix-task-list | P0/3 | ✓ Complete | **DONE** | task.rs sorting, agent column, integration tests pass |
| wp-agent-oneshot-execution | P0/4 | ✓ Complete | **DONE** | --agent flag, --prompt mode, wait_for_completion in SpawnConfig |
| feat-hex-dev-remaining-fixes | P0/3 | ✓ Complete | **DONE** | retry logic fallback, grade normalization, scaffold generation |
| feat-make-hex-dev-end-to-end-usable | P0/3 | ✓ Complete | **DONE** | 120s timeout, per-phase model routing, error retry |
| feat-opencode-integration | P1/7 | ✓ Complete | **DONE** | opencode.rs command module, MCP injection, all 12 steps |
| feat-v2-swarm-controlled-quality-orchestration | P0/5 | ✓ Complete | **DONE** | quality_agent.rs, fix_agent.rs, hex-pipeline topology |
| feat-store-tool-calls-in-spacetimedb | P0/4 | ✓ Complete | **DONE** | dev_tool_call table, POST/GET endpoints, dual-write |
| feat-validate-loop | P0/4 | ✓ Complete | **DONE** | POST /api/analyze, compile+test runner, A-F grades, retry |
| feat-quantization-aware-inference-routing | P0/6 | ✓ Complete | **DONE** | quantization.rs, complexity.rs, quant_router.rs all present |
| feat-rust-workspace-boundary-analysis | P0/6 | ✓ Complete | **DONE** | scan_rust_workspace_layers() in analyze.rs, violation detection |
| feat-workflow-reliability-hardening (P0) | P2/5 | ✓ Partial | **PARTIAL** | P0-P1 done (TaskCompletionBody), P2 agent lifecycle not started |
| feat-architecture-context-injection | P0/7 | ⚠ Partial | **PARTIAL** | fingerprint_extractor.rs exists; routes/fingerprint.rs missing; MCP tools incomplete |
| feat-agent-swarm-ownership | P0/7 | ⚠ Partial | **PARTIAL** | owner_agent_id + version in hexflo-coordination; route endpoints incomplete |
| feat-neural-lab | P0/5 | ⚠ Partial | **PARTIAL** | WASM module + scheduled reducers done; API/CLI/dashboard partial |
| feat-secure-inference-and-secrets | P0/6 | ✗ Missing | **NOT STARTED** | vault.rs missing, SecretRef not implemented, key_ref schema incomplete |
| feat-swarm-agent-personalities | — | — | **STALE** | Superseded by feat-declarative-swarm-agent-behavior |

---

## Stale Swarms (require cleanup)

The following swarms in SpacetimeDB show `0/N tasks done, N running` — tasks are stuck `in_progress` with no active agent:

| Swarm | Tasks | Status |
|-------|-------|--------|
| create-a-hello-world-cli-application-in (×2) | 0/10, 0/4 | Stale — test swarms from e2e pipeline runs |
| docker-sandbox-e2e | 0/16 | Stale — 16 tasks stuck in_progress from sandbox testing |
| build-a-simple-greeter-cli | 0/4 | Stale |
| build-a-fibonacci-cli | 0/4 | Stale |
| build-a-sophisticated-f1-race-standings (×2) | 0/9, 0/8 | Stale |
| build-a-url-shortener-rest-api-in-go (×2) | 0/2 | Stale |
| dashboard-overhaul | no tasks | Empty swarm |
| worktree-enforcement | no tasks | Empty swarm |

**Root cause:** These swarms were created during pipeline e2e testing. Tasks were assigned but agents disconnected before completing. The tasks are stuck `in_progress` because no agent reclaimed them (heartbeat timeout cleanup should have run).

**Action:** Run `hex swarm cleanup` or `POST /api/hexflo/cleanup` to reclaim dead-agent tasks.

---

## Priority Action List (Updated)

### P0 — Blocking

1. **Secrets vault** (`feat-secure-inference-and-secrets`) — only truly unstarted workplan. `vault.rs`, `SecretRef`, AES-256-GCM encryption, and key_ref schema are all missing. ADR-2603261000 spec is clear.
2. **Stale swarm cleanup** — 8+ swarms with tasks stuck `in_progress` skew the `603/1090 tasks done` metric and may delay new task claims.

### P1 — High Priority

3. **Architecture context injection** — fingerprint route and MCP tools incomplete (P3-P7 of 7 missing). ADR-2603301200.
4. **Agent swarm ownership** — route endpoints incomplete. ADR-2603241900.
5. **Workflow reliability hardening** — P2 (agent lifecycle hardening) not started. ADR-2603311000.

### P2 — Medium Priority

6. **Neural lab** — WASM + reducers done; API/CLI/dashboard integration partial. ADR-2603241230.
7. **hex-agent STDB WebSocket client** — no code found. ADR-2603300100.

### P3 — Low Priority / Research

8. **Opencode integration** — shows P1/7 in pipeline (1 phase done), code complete — may just need pipeline phase markers updated.
9. **Registration lifecycle gaps** — ADR-065 open.
10. **Batch command execution context indexing** — ADR-2603301600, workplan exists.

---

## ADR Status (114 total)

| Status | Count | Notes |
|--------|-------|-------|
| Accepted | 97 | Includes 7 corrected from invalid "Implemented" value (2026-03-31) |
| Proposed | 12 | Active decisions pending implementation |
| Deprecated | 5 | ADR-009 (Ruflo → HexFlo), others retired |

**Proposed ADRs with no workplan or code:**
- ADR-040: Remote Agent WebSocket/SSH Transport
- ADR-056: Frontend Hexagonal Architecture
- ADR-063: SQLite → SpacetimeDB Migration
- ADR-064: Rust Compilation Performance
- ADR-2603232230: Tool Call Tracking in SpacetimeDB ← workplan exists but marked done; re-check
- ADR-2603250900: RL Reviewer Structured Output
- ADR-2603240045: Free Model Performance Tracking

---

## Notes on v1 Inaccuracies

The original gap report (v1) misclassified several workplans:

| Workplan | v1 Said | v2 Reality |
|----------|---------|------------|
| feat-opencode-integration | planned (no code) | DONE — opencode.rs exists |
| feat-quantization-aware-inference-routing | planned | DONE — quant_router.rs exists |
| feat-rust-workspace-boundary-analysis | planned | DONE — scan_rust_workspace_layers() exists |
| feat-tabled-cli-output | partial | DONE — fmt.rs fully implemented |
| feat-validate-loop | partial | DONE — full grade loop implemented |
| feat-store-tool-calls | planned | DONE — dev_tool_call table + endpoints exist |
| feat-swarm-agent-personalities | implemented | STALE — superseded by declarative-agents |

v1 limitation: `status` fields in workplan JSON are set by agents at the end of a successful run. When agents disconnect mid-run or runs predate the status field, the field stays at its initial value regardless of code state.

---

*Generated 2026-03-31 by gap analysis swarm (bb4ade1e) + code-presence verification + `hex project report` ground truth.*
