# ADR-2603232005: Self-Sufficient hex-agent with TUI

**Status:** Proposed
**Date:** 2026-03-23
**Drivers:** hex has the full development pipeline (ADR → Workplan → Swarm → Code → Validate), inference provider management (OpenRouter, Ollama, vLLM), RL model selection, and architecture enforcement — but it delegates all generative work to external AI agents (Claude Code, opencode). hex should be self-sufficient: a complete AI-assisted development environment that uses its own inference providers to do real work, with zero dependency on any external AI tool.
**Supersedes:** None (extends ADR-2603231600, ADR-030, ADR-031, ADR-027)

## Context

hex today is a **coordination layer** that depends on external AI agents for execution:

```
hex CLI ──coordinates──▶ Claude Code / opencode ──calls──▶ LLMs
                              ▲
                              │
                    hex can't work without these
```

This creates several problems:

1. **Vendor lock-in**: hex's value proposition is tied to Anthropic (Claude Code) or specific editor integrations (opencode). If those tools change, break, or become unavailable, hex stops being useful.
2. **Redundant plumbing**: hex already manages inference providers, routes requests, tracks costs, and selects models via RL. But it hands all this to an external agent that has its own inference stack.
3. **Context loss**: When the user switches from hex CLI to Claude Code, the rich context hex has (architecture state, ADR history, workplan structure, port interfaces) must be re-injected into the external agent's context window.
4. **Cost opacity**: External agents make their own inference calls outside hex's cost tracking and RL optimization.
5. **Offline impossible**: Claude Code requires internet. With Ollama registered in hex, there's no reason the pipeline can't run offline.

### The vision

```
hex dev "add response caching"
  │
  ├── hex-agent drafts ADR          (calls OpenRouter/Ollama directly)
  ├── hex-agent generates workplan  (calls inference, validates against schema)
  ├── hex-agent initializes swarm   (HexFlo, no external dependency)
  ├── hex-agent writes code         (calls inference per workplan step)
  ├── hex-agent runs validation     (hex analyze, tree-sitter, boundary checks)
  └── hex-agent presents for commit (TUI shows diffs, user approves)
```

Zero external AI agents. hex owns the full loop.

### Forces

- **hex already has the inference layer**: 300+ models via OpenRouter, local models via Ollama, any OpenAI-compatible endpoint. The plumbing exists.
- **hex has the context**: Architecture analysis, ADR history, AST summaries, port interfaces, workplan schemas. No external tool has this unless we re-inject it.
- **Bounded tasks, not open-ended chat**: Each pipeline phase is a single well-defined task (draft an ADR, decompose into steps, generate one file). This doesn't need a conversational agent — it needs focused, single-shot inference calls with good prompts.
- **Local models are good enough**: Llama 4 Maverick (1M context), DeepSeek R1 (reasoning), Qwen 3 235B (multilingual) — these can generate ADRs, workplans, and code competently for most tasks.
- **ratatui is battle-tested**: lazygit, k9s, bottom, gitui all use it. Rust TUI is a solved problem.

## Decision

### 1. `hex dev` — Single Entry Point

```bash
# Full pipeline with TUI
hex dev "add response caching to inference endpoints"

# Quick mode — skip ADR/workplan for small fixes
hex dev "fix typo in dashboard header" --quick

# Specify model explicitly
hex dev "add caching" --model openrouter-deepseek-deepseek-r1

# Offline with local model
hex dev "add error handling" --model ollama-qwen3-32b

# Non-interactive (CI, scripts)
hex dev "add caching" --auto --budget 2.00

# Resume interrupted session
hex dev --resume
```

### 2. hex-agent Inference Client

hex-agent calls inference directly through hex-nexus. No external agent runtime needed.

```
hex-agent                        hex-nexus                    Provider
   │                                │                            │
   ├─ POST /api/chat/completions ──▶│                            │
   │   { model, messages,           ├─ route by provider ──────▶│
   │     system_prompt }             │   add headers (OpenRouter) │
   │                                │   track cost               │
   │◀── { content, usage, cost } ──┤◀── response ──────────────┤
   │                                │                            │
   ├─ write file to disk            │                            │
   ├─ update HexFlo task status     │                            │
   └─ update TUI                    │                            │
```

The inference client lives in `hex-agent/src/adapters/secondary/inference_client.rs` and reuses the existing `OpenAiCompatAdapter` for wire protocol.

### 3. Pipeline Phases (hex-agent drives all)

| Phase | hex-agent Action | Context Assembled | Gate |
|-------|-----------------|-------------------|------|
| **ADR** | Calls inference to draft ADR from description | Existing ADRs (style), `hex analyze` summary, related ADRs | User approves/edits |
| **Workplan** | Calls inference to decompose into hex-bounded steps | ADR content, workplan JSON schema, architecture rules, tier definitions | User approves/edits |
| **Swarm** | Initializes HexFlo swarm + creates tasks from workplan | Workplan steps → task mapping (no inference needed) | None (automatic) |
| **Code** | Per-step: calls inference to generate code | Workplan step, target file AST, port interfaces, hex boundary rules | User reviews diff |
| **Validate** | Runs `hex analyze`, checks boundaries | Analysis output (no inference needed) | User sees results |
| **Commit** | Presents full diff for approval | Git diff (no inference needed) | User approves |

### 4. TUI Layout (ratatui)

```
┌─ hex dev ──────────────────────────────────────────────────┐
│ Feature: add response caching to inference endpoints       │
│ [■ ADR] [■ Plan] [▶ Code] [ Validate] [ Commit]           │
├────────────────────────────────────────────────────────────┤
│                                                            │
│  ▶ Phase 3: Code Generation                                │
│  ┌─ Tasks ───────────────────────────────────────────┐     │
│  │ ✓ step-1: CachePort trait (ports/)         2.1s   │     │
│  │ ✓ step-2: Redis adapter (adapters/sec/)    4.8s   │     │
│  │ ▶ step-3: Wire into inference route        ...    │     │
│  │ ○ step-4: Integration tests                       │     │
│  └───────────────────────────────────────────────────┘     │
│                                                            │
│  Provider: openrouter (deepseek-r1)                        │
│  Cost: $0.03  Tokens: 4.2K  Budget: $0.03/$1.00           │
│                                                            │
├────────────────────────────────────────────────────────────┤
│ [a]pprove  [e]dit  [r]etry  [m]odel  [p]ause  [q]uit      │
└────────────────────────────────────────────────────────────┘
```

Controls:
- `a` — approve gate, continue to next phase
- `e` — open current artifact in `$EDITOR`
- `r` — retry current step with same or different model
- `m` — switch model for next inference call
- `p` — pause (session persists, resume with `hex dev --resume`)
- `q` — quit (session saved)
- `d` — show diff for current step
- `l` — show inference log (prompt/response)

### 5. Prompt Templates

System prompts live in `hex-cli/assets/prompts/`, baked into the binary via `rust-embed`:

```
hex-cli/assets/prompts/
  adr-generate.md          # "You are a software architect writing an ADR..."
  workplan-generate.md     # "Decompose this ADR into hex-bounded steps..."
  code-generate.md         # "Generate code for this workplan step..."
  test-generate.md         # "Write tests for this module..."
  fix-violations.md        # "Fix these architecture violations..."
```

Templates use `{{placeholders}}` filled at runtime:
- `{{existing_adrs}}` — titles + statuses of recent ADRs
- `{{architecture_summary}}` — output of `hex analyze`
- `{{workplan_schema}}` — canonical JSON schema
- `{{port_interfaces}}` — relevant port trait definitions
- `{{ast_summary}}` — tree-sitter summary of target files

### 6. Model Selection

The RL engine (ADR-031) selects models per phase automatically:

**OpenRouter is the primary provider.** It gives access to the best open-source and commercial models without managing GPUs or multiple accounts. The RL engine selects the best OpenRouter model per phase.

#### Three-Tier Fallback Chain

Each phase has a fallback chain: **paid model → free model → Ollama**. If a paid model fails (402 insufficient credits, 429 rate limit), hex automatically falls back to a free-tier model. If free models are also rate-limited, Ollama provides the offline last resort.

| Phase | Task Type | Paid (default) | Free fallback | Offline fallback |
|-------|-----------|---------------|---------------|-----------------|
| ADR | `reasoning` | `deepseek/deepseek-r1` | `qwen/qwen3-next-80b-a3b-instruct:free` → `nvidia/nemotron-3-super-120b-a12b:free` → `openai/gpt-oss-120b:free` | Ollama |
| Workplan | `structured_output` | `meta-llama/llama-4-maverick` | `nvidia/nemotron-3-super-120b-a12b:free` → `qwen/qwen3-next-80b-a3b-instruct:free` | Ollama |
| Code | `code_generation` | `meta-llama/llama-4-maverick` | `qwen/qwen3-coder:free` → `nvidia/nemotron-3-super-120b-a12b:free` | Ollama |
| Fix | `code_edit` | `deepseek/deepseek-r1` | `qwen/qwen3-next-80b-a3b-instruct:free` → `qwen/qwen3-coder:free` | Ollama |

#### Cost Tiers

| Tier | Cost | Quality | When Used |
|------|------|---------|-----------|
| **Paid** | $0.008/app | Best | Default — credits available |
| **Free** | $0.000/app | Good | Credits exhausted — auto-fallback |
| **Offline** | $0.000/app | Varies | No internet — Ollama local models |

A typical `hex dev` session: **$0.008 with paid models, $0.000 with free models**. The pipeline works the same either way.

The RL engine learns from outcomes across all tiers — if a free model produces code that passes `hex analyze` on first try, its reward increases for that task type. Over time, hex learns which models (paid or free) work best for your codebase.

```bash
# Default: paid models with free fallback
hex dev start "add response caching" --auto

# Force a specific free model
hex dev start "feature" --model "qwen/qwen3-coder:free"

# Force local (offline mode)
hex dev start "feature" --provider ollama
```

### 7. Standalone Binary — Zero External Dependencies

After this ADR, the only runtime dependencies for `hex dev` are:

| Dependency | Required? | Purpose |
|-----------|-----------|---------|
| hex-nexus | Yes | Inference routing, state persistence, analysis |
| SpacetimeDB | Yes | HexFlo coordination, cost tracking |
| OpenRouter API key | Recommended | 300+ models, no GPU, $0.05–$0.30/session |
| Ollama (optional) | No | Offline fallback, zero cost |
| Claude Code | **No** | Not needed |
| opencode | **No** | Not needed |
| Any editor plugin | **No** | Not needed |

```bash
# Recommended setup — OpenRouter (one API key, 300+ models)
hex nexus start
hex secrets set OPENROUTER_API_KEY sk-or-v1-...
hex inference discover --provider openrouter
hex dev "add response caching"
# hex-agent picks the best model per phase via RL
# Typical session cost: $0.05–$0.30

# Offline setup — local models only
hex nexus start
hex inference add ollama http://localhost:11434
hex dev "add response caching" --provider ollama
```

### 8. Session Persistence

Sessions survive interruptions and are stored in `~/.hex/sessions/dev/`:

```bash
hex dev --resume              # Resume most recent
hex dev --resume <session-id> # Resume specific
hex dev --list                # List all sessions with status
hex dev --clean               # Remove completed sessions
```

Session state:
- Current phase and step
- Generated artifacts (ADR text, workplan JSON)
- Completed diffs per step
- Accumulated cost
- Model selections and RL rewards

### 9. Modes

| Mode | Flag | Behavior |
|------|------|----------|
| Interactive (default) | none | TUI, pauses at gates |
| Quick | `--quick` | Skips ADR + workplan, goes straight to code |
| Auto | `--auto` | No gates, runs to completion (for CI/batch) |
| Dry run | `--dry-run` | Shows what would happen without calling inference |

## Consequences

### Positive
- **Zero external AI tool dependency** — hex is a complete AAIDE, not a coordination layer for someone else's agent
- **OpenRouter unlocks 300+ models for $0.05–$0.30/session** — full pipeline (ADR → code) costs less than a single Opus conversation. One API key, no GPU, no per-provider accounts.
- **RL learns your codebase** — the model selection engine discovers which OpenRouter models produce the best ADRs, workplans, and code for your specific project
- **Full context advantage** — hex-agent has architecture state, ADR history, port interfaces natively. No re-injection needed.
- **Cost-controlled** — every inference call tracked, OpenRouter actual cost captured, budget caps enforced
- **Single binary** — `hex` is the only tool a developer needs (plus hex-nexus and SpacetimeDB)
- **Offline fallback** — Ollama for air-gapped or zero-cost development when needed

### Negative
- **TUI development effort** — ratatui layout, event handling, alternate screen management
- **Prompt engineering** — 5+ prompt templates that need tuning per model family
- **Quality ceiling** — for complex reasoning tasks, local models may not match Opus-class performance
- **Scope** — this is a significant feature that touches hex-cli, hex-agent, and hex-nexus

### Mitigations
- **TUI**: Start with minimal layout (pipeline bar + task list + status). Iterate.
- **Prompts**: Templates are editable files in `assets/prompts/`. Community can tune them. RL engine discovers which models need which prompt styles.
- **Quality**: `--model` override lets users pick frontier models when quality matters. RL engine learns. Gates catch bad output before it lands.
- **Scope**: Implement incrementally — P1-P3 deliver `hex dev` with ADR generation alone. Each subsequent phase adds value independently.

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Inference client in hex-agent (calls hex-nexus `/api/chat/completions`) | Pending |
| P2 | Prompt templates in `hex-cli/assets/prompts/` with `{{placeholder}}` expansion | Pending |
| P3 | `hex dev` command scaffold + ratatui TUI (pipeline bar, controls, status) | Pending |
| P4 | ADR generation phase — context assembly, prompt, gate UX, file write | Pending |
| P5 | Workplan generation phase — structured output, JSON schema validation | Pending |
| P6 | Auto swarm init + task creation from workplan steps | Pending |
| P7 | Code generation phase — per-step context, diff display, file write | Pending |
| P8 | Validation phase — `hex analyze` integration, violation fix loop | Pending |
| P9 | Session persistence + `--resume` | Pending |
| P10 | `--quick`, `--auto`, `--dry-run` modes | Pending |
| P11 | RL task type wiring for per-phase model selection | Pending |
| P12 | Cost tracking + budget caps in TUI status bar | Pending |

## References

- ADR-2603231600: OpenRouter Inference Integration
- ADR-030: Multi-Provider Inference Broker
- ADR-031: RL-Driven Model Selection & Token Management
- ADR-027: HexFlo Native Coordination
- ADR-2603221939: Mandatory Swarm Tracking
- ADR-035: Architecture V2 — Inference as Pluggable Adapters
- [ratatui](https://ratatui.rs) — Rust TUI framework
