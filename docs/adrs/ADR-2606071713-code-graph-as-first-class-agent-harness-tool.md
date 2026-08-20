# ADR-2606071713: Code-graph as a first-class agent-harness tool (active, not just passive context)

**Status:** Proposed
**Date:** 2026-06-07
**Epoch:** single-agent
**Drivers:** Operator directive — *"hex graph must be used as part of our agent harness for these
assessments."* During a routing investigation (does `/api/inference/complete` reach the `claude -p`
adapter?) the assessment was done by hand-grep; `hex graph` would have surfaced the neighbourhood
instantly. The `hex-exec` ReAct loop exposes 19 tools and **none of them query the code-graph** — it
can `repo_grep` but cannot ask the graph "who consumes this / how does X reach Y / what's this file's
neighbourhood." The graph is used only *passively* (prepended once by `gather_context`).
**Supersedes:**
**Superseded-By:**

## Context

The single-agent epoch's whole thesis (ADR-2606061359, ARCHITECTURE.md) is that the differentiator is
**the quality of context assembled for the loop — code-graph relevance + ranked lessons — not agent
head-count.** The `hex-graph` crate (`KnowledgeGraph`, `context_for`, `rank_lessons`, `consumers`,
`path`, semantic `query`) is already in-process and powers the `hex graph` CLI.

Yet the agent loop only consumes the graph **passively**: `direct_exec.rs::gather_context` loads
`graph-out/graph.json` once and prepends the target file's neighbourhood + ranked lessons to the edit
prompt. The agent cannot **act** on the graph during reasoning:

- It can't trace **consumers** before proposing an edit (the excision-safety oracle that ADR-2606071340
  relies on, and that CLAUDE.md's "trace ALL consumers before deleting" lesson demands).
- It can't ask **"how does X route to Y"** or **"what's the path between these"** structurally — so it
  greps, which finds strings but not structure and misses *what isn't wired* (precisely the gap that
  made the claude-adapter routing assessment slow).
- It can't pull a **neighbourhood** for a file other than the one being edited.

So the loop is blind to the very structural context that is supposed to be hex's edge. A grep-only agent
is not living the thesis. (This applies to the assistant too: structural/excision/routing questions
should hit `hex graph query|consumers|context|path` first, grep only for the runtime-conditional detail
the graph can't resolve.)

## Decision

Make the code-graph a **first-class, active tool** in the agent harness, alongside `repo_read`/`repo_grep`.

Add to `hex-exec/src/tools/` (backed directly by the in-process `hex_graph` crate — no subprocess, no
nexus round-trip) and register in `ALLOWED_TOOLS` (all read-only, safe inside the edit loop):

- **`graph_context(file)`** — the file's neighbourhood: defines / uses / consumers / community.
  (Wraps `context::context_for`; the active form of what `gather_context` does passively.)
- **`graph_consumers(module_or_file)`** — who depends on this; the excision-safety oracle.
- **`graph_query(question)`** — semantic search over the graph for orientation.
- **`graph_path(a, b)`** — shortest structural path between two nodes ("is X wired to Y").

Harness behaviour: the loop's system prompt instructs the agent to **consult the graph before editing
or assessing structure** — trace consumers before a delete/rename, pull `graph_context` for any file it
is about to touch beyond the target, and use `graph_query`/`graph_path` for "how does this connect"
questions instead of grep. Grep remains for content/string matches the graph doesn't model.

This composes with the loop-guard (ADR-2606071651) and the frontier-model unlock: a Claude-driven loop
*with* active graph tools is the thesis actually realized — strong reasoning over high-quality
structural context.

## Consequences

**Positive**
- The agent grounds assessments in structure, not string-grep — fewer missed consumers, fewer
  "looks unused, delete it" regressions (the exact class CLAUDE.md warns about).
- Reuses the in-process `hex_graph` crate; no new dependency, no subprocess, negligible latency.
- Makes the "context quality is the differentiator" thesis observable and testable (the graph-ablation
  harness can now toggle *active* graph access, not just passive injection).

**Negative / risks**
- Graph freshness: `graph-out/graph.json` can lag the working tree. Tools must report graph staleness
  (build timestamp) so the agent doesn't trust a stale neighbourhood; a `graph build` may be needed
  before high-stakes excision. (Non-blocking — stale graph is still better than grep-only.)
- More tools = more ways for a weak model to wander (see ADR-2606071651). Mitigated by clear
  tool descriptions and the loop-guard's edit-nudge.
- The graph models structure, not runtime conditionals — tool descriptions must say so, so the agent
  still verifies branch logic in source when the question is "which branch fires."

## Implementation

- `hex-exec/src/tools/graph_context.rs`, `graph_consumers.rs`, `graph_query.rs`, `graph_path.rs` —
  thin wrappers over `hex_graph::{context, model}`; load `graph-out/graph.json` (shared loader),
  return compact markdown + a `graph_built_at` staleness stamp.
- Register the four in the tool registry and `ALLOWED_TOOLS` (`hex-exec/src/direct_react.rs`).
- Update the loop system prompt: "consult the graph before editing/assessing structure; trace
  consumers before delete/rename."
- Tests: each tool returns expected neighbourhood/consumers/path on a fixture graph; staleness stamp
  present; `ALLOWED_TOOLS` includes them and the loop dispatches them.

Tracking workplan: create via `hex plan draft`.

## References

- ADR-2606061359 — single-agent epoch (the "context quality is the differentiator" thesis this serves).
- ADR-2606071340 — crate split; `hex graph consumers` as the excision oracle (same capability, now
  handed to the agent).
- ADR-2606071651 — ReAct loop-guard (more tools ⇒ stronger wander-mitigation needed; composes here).
- CLAUDE.md — "Trace ALL consumers before deleting"; the graph tool operationalizes this for the agent.
- Memory: `feedback_graph_in_agent_harness` (the operator directive recorded).
