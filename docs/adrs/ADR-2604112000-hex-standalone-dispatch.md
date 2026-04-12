# ADR-2604112000: Hex Self-Sufficient Dispatch (Standalone Mode)

**Status:** Accepted
**Date:** 2026-04-11
**Drivers:** Completeness audit on 2026-04-11 confirmed hex is ~70% functional in Claude-integrated mode but ~30% in standalone mode. The executor dispatch layer gates every phase on `AgentManager` being populated (`hex-nexus/src/orchestration/workplan_executor.rs:746-750`), a slot that today is only ever filled when Claude Code is the driver. `wp-plan-execute-user-feedback` shipped only because the operator hand-bootstrapped dispatch via the Claude `Agent` tool. Memory: `project_wp_plan_execute_user_feedback_shipped.md`.
**Supersedes:** None (complements ADR-2604111800 executor dispatch-evidence guard, ADR-2604101500 local-inference-first, ADR-027 HexFlo coordination)

<!-- ID format: YYMMDDHHMM — 2604112000 = 2026-04-11 20:00 local -->

## Context

Hex is positioned as an AIOS — the OS layer for AI-driven development, installable into *any* target project. The AIOS claim implies hex must be able to run **without** Claude Code sitting above it: an operator should be able to `hex nexus start`, open the dashboard, and execute a workplan end-to-end using hex's own inference + HexFlo coordination.

Today that claim is false. The audit found three structural gaps:

1. **`AgentManager` is a Claude-shaped slot.** `workplan_executor.rs:746-750` aborts any phase with `"AgentManager not initialized"` if the slot is empty. The only composition path that populates it assumes Claude Code is the outer shell: the session-start hook writes `~/.hex/sessions/agent-{CLAUDE_SESSION_ID}.json`, the `SubagentStart`/`SubagentStop` hooks reconcile task state from stdin, and the executor reaches into that file to assign agents to phases. Remove Claude from the loop and the slot is never filled — the executor has no fallback composition path that constructs an `AgentManager` from HexFlo + local inference.

2. **No concrete inference adapter ships in the binary.** `hex-nexus/src/adapters/inference*` contains `InferenceRouterAdapter` (selects a server based on task tier / tags) and SpacetimeDB bindings for the `inference-gateway` WASM module, but **no concrete provider implementation** — no `OllamaInferenceAdapter`, no `OpenAIInferenceAdapter`, no wired-up `ClaudeCodeInferenceAdapter` that uses `claude -p --dangerously-skip-permissions` (memory: `feedback_claude_bypass_permissions`). The router can pick a target and fail to call it, because nothing implements the call.

3. **The SpacetimeDB state adapter is stubbed on the HexFlo-memory axis.** `hex-nexus/src/adapters/spacetime_state.rs` implements `IHexFloMemoryStatePort` but `hexflo_memory_store`, `hexflo_memory_retrieve`, `hexflo_memory_search`, and `hexflo_memory_delete` all return `Err(Self::err())`. The reducers exist in `spacetime-modules/hexflo-coordination/` and are reachable from the CLI (`hex memory store` → REST → adapter), but the adapter's call path is a placeholder. A standalone hex run cannot persist coordination memory across agents.

Taken together: hex-as-AIOS can't execute a workplan without a human operator manually dispatching agents via a tool that lives *outside* hex. This is a category-defining gap for an operating system. The ADR-2604111800 dispatch-evidence guard made the failure mode loud instead of silent, but loudness is not a fix — it converts "phantom completions" into "loud refusals", and the underlying dispatch is still broken.

## Decision

Hex SHALL grow a **second composition path** for executor dispatch — one that does not depend on Claude Code being the outer shell. The path has three required pieces, each the subject of a phase in the accompanying workplan.

1. **Standalone `AgentManager` composition.** When `hex nexus start` boots without a `CLAUDE_SESSION_ID` in env and without `~/.hex/sessions/agent-*.json` present, the composition root wires an `AgentManager` backed by the HexFlo dispatch layer + the default inference adapter. The executor's `AgentManager not initialized` branch is replaced by an explicit error that tells the operator which of the three prerequisites (inference adapter, HexFlo reachability, port wiring) is missing.

2. **One first-class standalone inference adapter.** Ollama is the reference implementation — it has zero licensing cost, runs locally, is already validated on the operator's Bazzite / Strix Halo rig at 32 tok/s with Vulkan (memory: `reference_bazzite_ollama_vulkan`), and is consistent with ADR-2604101500 `local-inference-first`. The adapter implements `IInferencePort` end-to-end: prompt → HTTP POST → token stream → response. `ClaudeCodeInferenceAdapter` gets a parallel fix to pass `--dangerously-skip-permissions` unconditionally when spawned non-interactively (memory: `feedback_claude_bypass_permissions`), so the Claude path continues to work when Claude is *an inference backend* rather than *the outer shell*. OpenAI / Anthropic cloud adapters remain out of scope for this ADR — a separate workplan can add them once the composition path is proven against Ollama.

3. **Un-stub the HexFlo memory adapter.** `spacetime_state.rs::hexflo_memory_*` methods replace their `Err(Self::err())` bodies with real SpacetimeDB reducer calls against `hexflo-coordination`. The SQLite fallback path (ADR-011, `~/.hex/hub.db`) remains the offline mode. Heartbeat / task claim / task complete reducers are already wired via `inference_task_*`; this ADR completes the matching data-plane calls for memory store/retrieve/search/delete.

The **guardrail** for standalone mode is a new acceptance gate: a `hex ci` invocation must be able to execute a one-task workplan end-to-end on a host where the `CLAUDE_*` env vars are unset and no Claude CLI is installed. This is the test that proves the gap is closed. It lives as the final phase of the workplan.

## Consequences

**What this ADR gives us:**
- Hex fulfils its AIOS positioning: `hex nexus start && hex plan execute wp-foo` works on a fresh Bazzite box with only Ollama installed. No Claude Code required.
- The executor's `AgentManager` check becomes an observable contract rather than an implicit Claude dependency. Operators get a clear error message when composition is incomplete.
- ADR-2604101500 `local-inference-first` gets its first concrete adapter — "local inference first" stops being aspirational.
- The Claude-integrated path is unaffected as a *fast path*: when `CLAUDE_SESSION_ID` is present, the existing composition takes precedence. Standalone is the fallback, not a replacement.

**What this ADR costs us:**
- New code surface: one `OllamaInferenceAdapter`, one standalone composition-root variant, four un-stubbed adapter methods. All of it inside `hex-nexus/src/adapters/` and `hex-nexus/src/composition.rs` (or equivalent). No new crates.
- New test infrastructure: the standalone CI gate requires either a mocked Ollama endpoint in tests or a live Ollama instance in the CI runner. The workplan picks the mocked approach (a local HTTP fixture) to keep CI hermetic.
- `ClaudeCodeInferenceAdapter` has to be reworked so it doesn't assume the outer shell is Claude — it becomes one inference backend among several, callable from a HexFlo-driven dispatch loop.
- Risk: the operator has historically preferred manual `Agent`-tool bootstraps when hex dispatch was broken. Shipping a working dispatch path means removing that escape hatch — or at least making it the exceptional path, not the default. The workplan P6 adds a doctor check that warns if a workplan last ran via manual bootstrap on the current machine.

**What this ADR does NOT do:**
- Does not touch the hook routing layer, T1/T2/T3 classification, or the auto-invoke planner behavior (ADR-2604110227). Those remain Claude-integrated-mode features.
- Does not redesign the executor. The `AgentManager` abstraction stays; only its composition changes.
- Does not ship additional inference providers beyond Ollama. OpenAI / Anthropic cloud / vLLM remain future work.
- Does not address dashboard live-state wiring. That is a separate gap from the audit, scoped to its own workplan if/when the operator prioritises it.

## Alternatives Considered

- **(A) Rewrite the executor to not depend on `AgentManager`.** Rejected: `AgentManager` is a perfectly good abstraction — it aggregates live agent handles, dispatches tasks, and reports heartbeats. The problem is not the interface, it is the single composition path. Fixing composition is a smaller blast radius than rewriting dispatch.

- **(B) Ship a "Claude-required" banner and call standalone mode out of scope.** Rejected: explicitly contradicts the AIOS positioning in `CLAUDE.md` and the project objective. If hex cannot run without Claude, it is a Claude plugin, not an operating system. That framing is load-bearing for the rest of the roadmap (ADR-2604101500, ADR-2604102200).

- **(C) Add OpenAI as the first standalone adapter instead of Ollama.** Rejected: requires a vault-managed API key on every CI run, reintroduces the vault-credits coupling that motivated the Claude bypass work (memory: `project_bypass_mode`), and conflicts with ADR-2604101500 `local-inference-first`. Ollama is free, local, and already validated on the operator's hardware.

- **(D) Keep the manual `Agent`-tool bootstrap as the blessed standalone path.** Rejected: the `Agent` tool is a Claude Code capability. Relying on it for standalone mode is just Claude-integrated mode with extra steps. The whole point of this ADR is that a hex operator should not need a Claude install at all.

- **(E) Ship a dry-run / no-inference mode where the executor pretends to dispatch and writes canned outputs.** Rejected as dishonest — it would make the audit's "~30% complete" number go up without the AIOS claim actually becoming true. A running workplan that produces no real code is worse than a failing one, because it erodes trust in the `done` status.

## Notes

- This ADR is the first of two standalone-mode ADRs foreseen by the audit. The second, not yet written, would cover **dashboard live-state bindings** — today the dashboard renders scaffolding but doesn't subscribe to SpacetimeDB for live updates. Dispatch is the higher-priority gap, so it goes first.
- The dispatch-evidence guard from ADR-2604111800 stays in place. Its job is to reject vacuous completions; un-stubbing dispatch gives it real evidence to accept. The two ADRs are complementary: the guard is the *contract*, this ADR is the *implementation*.
- The workplan `wp-hex-standalone-dispatch` tracks execution. It is P0-BLOCKER because every other standalone-mode claim in the project (dashboard, standalone deployment, Bazzite e2e) is gated on dispatch actually running.
- Related: ADR-2604101500 (local-inference-first), ADR-2604111800 (dispatch-evidence guard), ADR-027 (HexFlo native coordination), ADR-011 (SQLite fallback), ADR-2604110227 (auto-invoke planner, Claude-mode only).
