# Investigation Recommendation: Claude Code Hooks + Minimal SpacetimeDB

**Date:** 2026-04-01
**Question:** Should hex's architecture enforcement be migrated to Claude Code hooks, and can SpacetimeDB be scoped down to only multi-agent/multi-host coordination?

**Answer: Yes to both — with a clear boundary between them.**

---

## TL;DR

| Layer | Recommendation |
|-------|----------------|
| Architecture enforcement (local) | **Migrate to Claude Code hooks** |
| Treesitter analysis gate | **Migrate to Claude Code hooks** |
| Swarm / task coordination | **Keep SpacetimeDB** (irreplaceable for multi-host) |
| Inference routing (multi-provider) | **Keep SpacetimeDB** (no fallback today) |
| Secret grant distribution | **Keep SpacetimeDB** (network service required) |
| 14 of 19 WASM modules | **Remove** (never wired or duplicated) |

---

## What the Investigation Found

Three specialist agents audited the full codebase across three dimensions:

### 1. Feature Classification (audit.md)

Every hex feature was classified: HOOKS (session-local), STDB (distributed state), BOTH, or NEITHER.

**HOOKS only (~12 features):** `pre_bash` destructive command detection, specs-required gate, architecture-gate YAML, `sandboxed_fs`, `permission`, `rate_limiter`, `env_secrets`, `anthropic`, `tools`, `prompt`, `mcp_config`, `haiku_preflight`

**STDB only (~16 features):** All swarm/task/memory CLI commands, `stdb_task_poller`, `controller_worker`, `spacetime_coordination`, `stdb_connection`

**BOTH (~18 features):** `route`, `pre_edit`, `post_edit`, `pre_agent`, `subagent_start/stop`, `session_start/end`, `hub_client`, `output_analyzer`, `context_manager`, lifecycle-enforcement YAML

**Key insight:** Most operational features are BOTH — they need a hook as a trigger AND SpacetimeDB for persistent cross-session/cross-host state. Pure-HOOKS features are those entirely contained within a single agent session.

### 2. SpacetimeDB Module Audit (stdb.md)

19 WASM modules exist; only 5 databases are actually called from the nexus state adapter.

**Must keep (4 modules):** `hexflo-coordination`, `agent-registry`, `inference-gateway`, `secret-grant`

**Keep for multi-host (2 modules):** `fleet-state`, `file-lock-manager`

**Remove — 14 modules** are never wired, duplicated by hexflo-coordination, or replaced by YAML files: `rl-engine`, `neural-lab`, `hexflo-cleanup`, `hexflo-lifecycle`, `workplan-state`, `conflict-resolver`, `architecture-enforcer`, `skill-registry`, `agent-definition-registry`, `hook-registry`, `inference-bridge`, `test-results`, plus two others.

**Critical gap found:** `remote-agent-registry` module referenced in `stdb_connection.rs` does not exist in `spacetime-modules/`. The WebSocket task claiming path is dead code — all agents fall back to REST polling today.

### 3. Risk Assessment (risk.md)

**Verdict: CONDITIONAL GO**

- Arch enforcement migration: **Low risk** — 163-line `ConstraintEnforcer` → ~50-line hook script
- `hex analyze` as PostToolUse hook: **Low risk** — binary already exists, 15-line wrapper
- `stdb_inference.rs` removal: **High risk** — code has explicit hard-failure path, no fallback
- Swarm coordination removal: **Critical risk** — 30+ MCP tools permanently broken
- Remote secrets removal: **Not feasible** — network service required for multi-machine

---

## Proposed Architecture

```
Claude Code hooks (local, per-session)
  PreToolUse  → check hex layer, forbidden paths, destructive commands
  PostToolUse → run hex analyze, update local edit log
  Stop        → fail if arch violations exist
  UserPromptSubmit → intent classification, workplan gate

SpacetimeDB (minimal, 4 core modules only)
  hexflo-coordination → swarm/task/agent/memory (multi-host atomic ops)
  agent-registry      → live agent visibility across hosts
  inference-gateway   → multi-provider LLM routing
  secret-grant        → TTL-based key distribution to remote agents
```

---

## What to Ship in Phase 1

Three hooks in `settings.json` deliver ~80% of current enforcement with zero daemon:

```json
{
  "hooks": {
    "PreToolUse": [{
      "matcher": "Write|Edit|MultiEdit",
      "hooks": [{ "type": "command", "command": "hex enforce check-file $TOOL_INPUT_PATH", "blocking": true }]
    }],
    "PostToolUse": [{
      "matcher": "Write|Edit|MultiEdit",
      "hooks": [{ "type": "command", "command": "hex analyze --file $TOOL_INPUT_PATH --quiet", "blocking": false }]
    }],
    "Stop": [{
      "hooks": [{ "type": "command", "command": "hex analyze --violations-only --exit-code", "blocking": true }]
    }]
  }
}
```

This requires `hex enforce check-file <path>` as a thin CLI wrapper reading `.hex/adr-rules.toml` locally — no daemon needed.

---

## What NOT to Change

1. **Do not remove `stdb_inference.rs`** without first building a direct Anthropic/Ollama adapter that doesn't require SpacetimeDB. The code has an explicit no-fallback path.
2. **Do not remove swarm CLI commands** — 30+ MCP tools would permanently break.
3. **Do not attempt to replace secret grants with hooks** — hooks run locally and cannot distribute time-limited keys to remote agents over the network.

---

## Migration Sequencing

| Phase | Scope | Risk | Timeline |
|-------|-------|------|----------|
| 1 | Implement 3 enforcement hooks; add `hex enforce check-file` CLI | Low | 1 week |
| 2 | Extract inference providers to `.hex/providers.toml`; allow daemon-free provider config | Medium | 2 weeks |
| 3 | Remove 14 unused WASM modules from `spacetime-modules/` | Low | 1 week |
| Later | Build non-STDB inference adapter before touching `stdb_inference.rs` | High | Defer |

---

## Evidence Files

- `docs/analysis/hooks-investigation-audit.md` — full feature classification (HOOKS/STDB/BOTH/NEITHER)
- `docs/analysis/hooks-investigation-stdb.md` — WASM module audit, module-by-module verdict
- `docs/analysis/hooks-investigation-risk.md` — break risk, migration complexity, rollback paths
- `docs/analysis/hooks-prototype/` — working prototype: `pre-tool-use.sh`, `post-tool-use.sh`, `settings-snippet.json`
