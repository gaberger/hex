# Prompt Prefix Audit — Agent YAML Templates

**Date**: 2026-04-13
**Source**: `hex-cli/assets/agents/hex/hex/*.yml` (18 agent definitions)
**Purpose**: Identify shared prefix tokens across agent prompts to maximize KV-cache hit rate for tiered inference routing.

---

## Agent Inventory

| Agent | Type | Tier | Has `constraints`? | Has `tools`? | Has `model`? | Has `context`? |
|---|---|---|---|---|---|---|
| hex-coder | coder | 2 | Y | Y | Y | Y |
| planner | planner | 3 | Y | Y | Y | Y |
| hex-reviewer | reviewer | 2 | N | N | Y | N |
| integrator | integrator | 3 | Y | Y | Y | Y |
| validation-judge | reviewer | 3 | Y | Y | N* | Y |
| swarm-coordinator | hierarchical-coordinator | 3 | Y | Y | Y | Y |
| behavioral-spec-writer | researcher | 3 | Y | Y | N* | Y |
| adr-reviewer | reviewer | 2 | Y | Y | Y | Y |
| dead-code-analyzer | reviewer | 2 | Y | N | N* | Y |
| dependency-analyst | researcher | 3 | Y | N | N* | Y |
| dev-tracker | coordinator | 3 | Y | Y | Y | Y |
| feature-developer | hierarchical-coordinator | 3 | Y | Y | Y | Y |
| scaffold-validator | reviewer | 2 | Y | N | N* | Y |
| status-monitor | monitor | 2 | Y | Y | Y | Y |
| rust-refactorer | — | — | Y | Y | N* | N |
| hex-tester | tester | 2 | N | N | Y | N |
| hex-fixer | fixer | 2 | N | N | Y | N |
| adversarial-reviewer | reviewer | 3 | Y | Y | Y | Y |

*N\* = uses legacy `model_tier` or inline format instead of structured `model:` block*

---

## Shared Tokens (Candidates for KV-Cache Prefix)

### 1. TOOL PREFERENCE constraint (~45 tokens, 15/18 agents)

Verbatim or near-verbatim across 15 agents:

```
TOOL PREFERENCE: Always use mcp__hex__* MCP tools before falling back to Bash.
e.g. use mcp__hex__hex_analyze not `hex analyze` via Bash,
mcp__hex__hex_adr_search not `hex adr search`.
```

**Variants**: The example commands differ per agent (e.g., `hex_hexflo_task_complete` for integrator, `hex_hexflo_swarm_init` for swarm-coordinator), but the rule itself is identical.

**Recommendation**: Extract the rule as a fixed prefix sentence. Append role-specific examples as a suffix.

### 2. ADR-060 PRIORITY constraint (~55 tokens, 8/18 agents)

Present in: hex-coder, planner, integrator, swarm-coordinator, feature-developer, status-monitor, behavioral-spec-writer (via constraints list), adversarial-reviewer (implicitly).

```
ADR-060 PRIORITY: If a critical notification (priority 2) appears in hook output,
STOP current work, save state, acknowledge via hex inbox ack, and inform the user.
This overrides all other work.
```

**Variants**: The "save state" verb differs (save workplan state / save swarm state / save feature state / save integration state), but the structure is identical.

**Recommendation**: Fixed prefix with a `{{state_noun}}` slot.

### 3. Core tool set (~30 tokens, 12/18 agents)

Most agents require the same base tools:

```yaml
tools:
  required:
    - Read
    - Glob
    - Grep
    - Bash
```

**Present verbatim in**: hex-coder, planner, integrator, adr-reviewer, dev-tracker, feature-developer, swarm-coordinator, adversarial-reviewer, status-monitor (Read/Glob/Grep only), rust-refactorer (different names but same semantics).

### 4. HexFlo tool block (~60 tokens, 6/18 agents)

Agents that participate in swarm coordination share a nearly identical HexFlo tool block:

```yaml
hexflo:
  - mcp__hex__hex_hexflo_swarm_init
  - mcp__hex__hex_hexflo_swarm_status
  - mcp__hex__hex_hexflo_task_create
  - mcp__hex__hex_hexflo_task_assign
  - mcp__hex__hex_hexflo_task_list
  - mcp__hex__hex_hexflo_task_complete
  - mcp__hex__hex_hexflo_memory_store
  - mcp__hex__hex_hexflo_memory_retrieve
  - mcp__hex__hex_hexflo_memory_search
```

**Present in**: swarm-coordinator, feature-developer, dev-tracker (subset), integrator (subset).

### 5. Inbox tool block (~20 tokens, 5/18 agents)

```yaml
inbox:
  - mcp__hex__hex_inbox_notify
  - mcp__hex__hex_inbox_query
  - mcp__hex__hex_inbox_ack
```

**Present in**: hex-coder, swarm-coordinator, feature-developer, status-monitor, dev-tracker (implicitly via memory).

### 6. Token budget pressure config (~40 tokens, 8/18 agents)

Identical structure with different thresholds:

```yaml
pressure:
  warn_at_pct: 70
  compress_at_pct: 80
  block_at_pct: 90
  relief: summarize_history | escalate
```

**Present in**: hex-coder, planner, integrator, swarm-coordinator, feature-developer, dev-tracker, adr-reviewer, status-monitor.

### 7. "Never cross adapter boundaries" / hex architecture rules (~30 tokens, 7/18 agents)

Various phrasings of the same rule:
- "Never import from other adapters (only from core/ports and core/domain)"
- "adapters/ may only import from ports/ (never from other adapters)"
- "Never modify adapter internals during merge"

**Present in**: hex-coder, integrator, dead-code-analyzer, validation-judge, feature-developer, adversarial-reviewer, planner.

---

## Unique Tokens Per Role

| Agent | Unique sections (not shared) |
|---|---|
| hex-coder | `workflow.phases` (TDD red/green/refactor), `feedback_loop` with compile/lint/test gates, `quality_thresholds`, `test_coverage_gate` |
| planner | `workflow.steps` (analyze→identify→build→assign→emit), `escalation` conditions |
| integrator | `workflow.phases` (merge ordering, conflict resolution strategies), merge commit conventions |
| validation-judge | `phases` (behavioral specs, property tests, smoke scenarios, sign conventions), verdict scoring |
| swarm-coordinator | `swarm` topology config, `progress_tracking`, `error_handling` (crash/stall/exhaustion) |
| behavioral-spec-writer | `phases` (domain research, behavior identification, convention extraction), spec format constraints |
| feature-developer | 7-phase lifecycle (init→specs→plan→worktree→code→validate→integrate), `worktree_conventions`, per-tier HexFlo hooks |
| dev-tracker | `memory_schema` keys, `workflow.phases` (start/plan/resume/validate/status/close), `agent_routing` rules |
| adversarial-reviewer | `workflow.phases` (build-gates, dangling-references, stale-config, cross-crate, security-scan, dead-code) |
| rust-refactorer | `role_prompt` (inline prose), worktree lifecycle, Rust-specific commands |
| status-monitor | `event_handling`, `anomaly_detection`, `output_level_config` |
| dead-code-analyzer | `analysis_phases` (graph build, dead exports, hex validation, circular detection, port coverage) |
| dependency-analyst | `analysis_phases` (decompose, language fit, library selection, runtime requirements, communication pattern, risk) |
| scaffold-validator | `validation_phases` (readme, quickstart, scripts, dev server, env, actually-runs, gitignore) |
| adr-reviewer | `workflow.steps` (index, validate-structure, status-transitions, cross-references, compliance, staleness) |
| hex-reviewer | Minimal — only model config |
| hex-tester | Minimal — only model config |
| hex-fixer | Minimal — only model config |

---

## Proposed Shared Prefix (~800 tokens)

The following blocks should be extracted into a **shared system prompt prefix** that is prepended to every agent invocation. This prefix must be byte-identical across all agents to maximize KV-cache reuse.

```
### hex AIOS Agent — Shared Context

You are an agent operating inside the hex AIOS framework, a microkernel-based
runtime built around hexagonal architecture (Ports & Adapters).

#### Tool Precedence
ALWAYS use mcp__hex__* MCP tools before falling back to Bash. hex MCP tools are
the primary interface for all codebase operations. Use Bash only for operations
with no mcp__hex__* equivalent (git commands, file system navigation).

#### Architecture Rules
1. domain/ must only import from domain/
2. ports/ may import from domain/ only
3. usecases/ may import from domain/ and ports/ only
4. adapters/primary/ may import from ports/ only
5. adapters/secondary/ may import from ports/ only
6. Adapters must NEVER import other adapters
7. composition-root is the ONLY file that wires adapters to ports

#### Priority Interrupts (ADR-060)
If a critical notification (priority 2) appears in hook output: STOP current work,
save state, acknowledge via mcp__hex__hex_inbox_ack, and inform the coordinator.
This overrides all other work.

#### Standard Tools
Base tools available to all agents: Read, Glob, Grep, Bash.
HexFlo tools (when applicable): swarm_init, swarm_status, task_create,
task_assign, task_list, task_complete, memory_store, memory_retrieve, memory_search.
Inbox tools (when applicable): inbox_notify, inbox_query, inbox_ack.

#### Token Budget Protocol
Monitor token usage. At 70%: warn. At 80%: compress history. At 90%: block new
context loading. Use summarize_history or escalate as relief strategy.

#### Commit Protocol
All background agents must use mode=bypassPermissions. Include HEXFLO_TASK:{id}
in prompts for automatic task tracking via SubagentStart/SubagentStop hooks.
```

**Estimated token count**: ~780-820 tokens (varies by tokenizer).

---

## Token Savings Estimate

| Metric | Value |
|---|---|
| Shared prefix size | ~800 tokens |
| Agents that benefit | 15/18 (hex-reviewer, hex-tester, hex-fixer are too minimal) |
| Current duplication | ~200-400 redundant tokens per agent (constraints + tools + architecture rules) |
| Cache hit rate improvement | From 0% (all unique) to ~60-70% prefix reuse across same-tier agents |
| Per-invocation saving | ~800 tokens cached after first agent in a swarm batch |
| Swarm batch saving (8 agents) | ~5,600 tokens of redundant processing eliminated |

---

## Implementation Notes

1. **Byte-identical prefix required**: KV-cache keys are hash-based. Even a single whitespace difference breaks the cache. The shared prefix must be rendered from a single template, not assembled per-agent.

2. **Role-specific suffix**: After the shared prefix, append the agent's unique `description`, `constraints`, `workflow`, `inputs`, and `outputs` sections.

3. **Three minimal agents** (hex-reviewer, hex-tester, hex-fixer) have almost no shared structure — they are model-config-only stubs. They should still receive the shared prefix for consistency, but the savings are proportionally smaller.

4. **Tier-specific sub-prefixes**: Consider a secondary cache tier where T2 agents (coder, reviewer, tester, fixer, monitor) share an additional sub-prefix about fast-model constraints, and T3 agents (planner, integrator, coordinator, feature-developer) share one about orchestration patterns.
