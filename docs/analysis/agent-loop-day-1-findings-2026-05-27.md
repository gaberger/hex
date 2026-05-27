# Agent-Loop Day 1 — Findings & Architecture Inventory (2026-05-27)

A single-session writeup of every commit, every autonomous proof, every
failure mode, and every reverted experiment from the day hex's SOP
agent-loop became a real thing. Written same-day to preserve detail.

## TL;DR

- **6 autonomous commits authored by `hex-autonomous`** through the full
  agent-loop chain — board ask → classifier → typed commitment → drafter
  bridge → agent_loop trajectory → precompile gate → twin → executor →
  git commit. 5 Rust + 1 TypeScript. All Rust commits compile + tests
  pass. The TypeScript was the final-experiment data point that proved
  the twin-model-choice axis.
- **All of wp-sop-agent-loop P1-P6 landed.** P4.2 (STDB compile_status
  column) and P6.3 (Solid view) deferred to a follow-up dashboard
  workplan. P7 acceptance proved live.
- **Three model-capability boundaries surfaced**, each one a concrete
  data point not a guess:
  - qwen3:4b twin (default) hallucinates policy reasons (e.g. "path not
    in allowed patterns" — a string not present in any source file)
  - qwen2.5-coder:14b twin (upgrade) ALSO hallucinates policy reasons
    despite explicit corrective prompts ("memory is selective; don't
    invent rules"); rejection rationales unstable across calls
  - qwen2.5-coder:14b drafter can't recover from precise syntax errors
    in 5k-token Rust drafts even with rustc diagnostics seeded as
    `prior_steps` (P5 mechanism)
- **One safety regression caught + corrected.** The first bootstrap fix
  for source-file edits inadvertently tagged drafter content with
  `proposed_by="tool:code_patch"`, which triggered the twin's auto-
  approve fast path and skipped LLM review. A 61-byte TOML fragment
  named `.rs` landed as commit 5df955dd. Reverted, re-designed with
  separate `agent_loop:<role>` namespace that gets the source-file
  hard-deny exception WITHOUT the auto-approve.
- **Multi-language extension blocked by twin LLM-judge brittleness, not
  by chain capability.** Persona produced clean Go + TS content; twin
  invented "off-topic: not in documented paths" rejections.

## Commit ladder (in chronological order)

```
2aa447b8 fix(nexus): unblock SOP loop end-to-end for local-AI dogfooding
a0084e14 docs: local-AI proof artifact + agent-loop workplan draft
38289b25 feat(nexus): agent_loop P1.1 + P1.2 — IAgentTool + repo_read
89dd0573 feat(nexus): agent_loop P1.3-P1.5 — repo_grep + cargo_check + code_patch_propose
520ad3c7 feat(nexus): agent_loop P2.1-P2.3 — Trajectory + ReAct driver + 6 tests
587e9b44 feat(nexus): agent_loop P3 — drafter bridge behind HEX_AGENT_LOOP_ENABLED
1ab814ec [autonomous] feat(hex-cli): auto — action#13 → agent_loop_smoke3.rs    ← first ever
9e3bc37c fix(nexus): autonomous-commit identity (hex-autonomous)
482b0837 docs: agent-loop dogfood proof — board ask → autonomous commit in 51s
a36e63d2 [autonomous] feat(hex-cli): auto — action#14 → agent_loop_smoke4.rs
e77cf661 feat(nexus): agent_loop P4.1 — pre-twin compile gate + retry-with-diagnostics
9d65e99d feat(nexus): agent_loop P5 — twin rejections seed the next trajectory
4f70b015 [autonomous] feat(hex-cli): auto — action#15 → standalone_gate.rs       ← REVERTED 158215ad (pre-fix gate gap)
91ff8fee fix(nexus): precompile gate must use --test to catch errors in #[test] fns
425944d3 [autonomous] feat(hex-cli): auto — action#16 → standalone_gate_v2.rs
a94e4965 [autonomous] feat(hex-cli): auto — action#17 → standalone_gate_v2.rs   ← P7 acceptance
158215ad chore: remove broken autonomous standalone_gate.rs (pre-fix gate gap)
c9faf00e docs: P7 acceptance — autonomous test for autonomous harness passes
d453165d [autonomous] feat(hex-cli): auto — action#19 → agent_loop_smoke5.rs
bd833b18 feat(nexus): agent_loop P6 — STDB observability for trajectory + step
f115d627 fix(nexus): agent_loop bridge can edit source files (drafter bootstrap)
5df955dd [autonomous] feat(hex-nexus): auto — action#20 → precompile_lang.rs    ← REVERTED 6a01f0f9 (safety regression)
6a01f0f9 Revert "feat(hex-nexus): auto — action#20 → precompile_lang.rs"
cfc57ad7 fix(nexus): agent_loop source edits use agent_loop:<role> tag, NOT tool:
96a6ae37 [autonomous] feat(hex-nexus): auto — action#21 → precompile_lang.rs    ← REVERTED 787bfd68 (old logic)
787bfd68 Revert "feat(hex-nexus): auto — action#21 → precompile_lang.rs"
38909701 fix(nexus): twin LLM judge — stop inventing path-pattern policy
91b9c70c [autonomous] chore(misc): auto — action#27 → agent_loop_ts_smoke_devstral.ts  ← multi-lang proof (Devstral twin)
```

**Counts**: ~25 commits, 5 net-positive autonomous, 3 reverts. The reverts
ARE the dogfood story — the system caught its own gaps, recorded them,
and moved on.

## What works: the tier-1 stack from the AI Harness summary

Mapped to your taxonomy:

| Technique | Hex artifact (commit) |
|---|---|
| Context engineering | `drafter_trajectory::render_task_brief` truncates CEO ask at 2k; system prompt frames tool list + contract |
| Context management | `repo_read` 16k char cap; STDB columns cap thought=2k, observation=4k, args=2k; trajectory total-token budget |
| Sub-agents w/ fresh contexts | Classifier, drafter, twin, executor — four distinct agents, each gets a clean prompt; persona-as-sub-agent in agent_loop driver |
| Constraint harnesses | `policy::allowed_repo_path`, twin hard-deny allowlist, drafter source-file abstain, `agent_loop:<role>` namespace |
| Default-FAIL evaluators | Twin reviewer (every proposed_action defaults to pending); precompile gate (rustc exit 0 required) |
| Sandboxed exec | Tempfile workdirs for precompile (PID + atomic counter naming); tokio::process timeouts |
| Append-only event logs | `commitment`, `proposed_action`, `agent_trajectory`, `agent_step`, `agent_messages` STDB tables |
| Observability | P6 — `agent_trajectory` + `agent_step` tables, queryable per-step thought + tool + observation |
| Brain/Hands/Session decoupling | SpacetimeDB = session; hex-nexus = hands; persona model = brain (replaceable via `HEX_*_MODEL` env) |

## What doesn't (yet): the meta-harness gap

The boundary the day's failures point at is **prompt-tuning small LLM
judges has steeply diminishing returns**. Specifically:

### Finding 1: qwen3:4b twin invents policy

First P7.2 attempt (commit 4f70b015) was rejected by twin (qwen3:4b
default) with `"path not in allowed patterns: hex-cli/tests/..."` —
that string doesn't exist anywhere in `twin_reviewer.rs` or any other
source file. The 4B model fabricated a policy rule.

Fix attempted: upgrade twin to qwen2.5-coder:14b via `HEX_TWIN_MODEL`.

### Finding 2a: Devstral-Small-2:24b twin DOES follow the instructions

End-of-day experiment: pulled `devstral-small-2:24b` and set
`HEX_TWIN_MODEL=devstral-small-2:24b`. Re-dispatched a TypeScript smoke
probe.

Result: commit `91b9c70c` autonomous, file landed cleanly. Devstral's
verdict rationale:

> `"path under examples/ allowlist, content matches .ts extension and is
> plausibly on-topic for a smoke test file"`

Compare to qwen2.5-coder:14b's rejection rationales on the same kind of
file pre-tune: `"off-topic: not in documented paths"` (5 separate
rejections).

Devstral cited the allowlist match (the prompt's APPROVE rule 2), did
extension-content sniffing, and applied the "judge content-vs-PATH when
no CEO request is available" fallback the rewritten prompt
**explicitly added**. The same prompt qwen2.5-coder:14b ignored.

This is the multi-language autonomous proof: **TypeScript content
produced by qwen2.5-coder:14b drafter, approved by devstral-small-2:24b
twin, executor wrote + autonomous commit landed**. Zero frontier calls.

### Finding 2b: qwen2.5-coder:14b twin ALSO invents policy

After the upgrade, twin approved the Rust paths (`hex-cli/tests/*.rs`)
but rejected `examples/*.go` and `examples/*.ts` with `"off-topic:
proposed file path does not match documented patterns in operator
memory"` — again, fabricated. The system prompt explicitly told it
"memory is selective; absence is neutral; don't invent rules" and the
LLM ignored it.

Fix attempted: comprehensive prompt rewrite (`38909701`) adding:
- explicit "HOW TO READ OPERATOR MEMORY" section
- broadened APPROVE rule (2) path list to match the hard-deny allowlist
- fallback "judge content-vs-PATH when no CEO request available"

Result: **fix did not work**. Verdicts 24-26 after the rewrite still
emitted the same hallucinated rationale.

### Finding 3: qwen2.5-coder:14b drafter ceiling at 5k-token drafts

The autonomous chain dispatch of `precompile_lang.rs` (the multi-language
gate extension) hit a real model boundary. Two trajectories:
- Attempt 1: 5 steps, 3198 output tokens, failed precompile gate
- Attempt 2 with diagnostics in prior_steps: 8 steps, 5228 output
  tokens, failed precompile gate with a different specific error
  (`character constant must be escaped: '\n'`)

Drafter abstained after PRECOMPILE_RETRIES=1. **The persona produces
correct content for ≤ ~80-line drafts (smoke tests, standalone_gate_v2)
but loses fidelity in the 100-200 line range.**

### Finding 4: safety-regression slip-through

The first bootstrap fix tagged `proposed_by="tool:code_patch"` for
source-file edits to satisfy the twin's hard-deny exception. **That tag
also triggered the auto-approve fast path** at `twin_reviewer.rs:449`,
which skips LLM review. The autonomous chain committed a 61-byte
Cargo.toml fragment named `.rs` (commit 5df955dd) with no semantic
review.

Caught + reverted + corrected with separate `agent_loop:<role>`
namespace that gets the hard-deny exception WITHOUT the auto-approve
(commit `cfc57ad7`). **The corrected design now correctly forces LLM
review on all agent-loop-produced source-file edits.**

## The architectural takeaway

> *Constraints and feedback loops around the model are doing more
> work than the model itself.* — your AI Harness summary

Today proved this end-to-end:
- The PRECOMPILE GATE (constraint harness, deterministic rustc) caught
  what the LLM judge couldn't — broken Rust bodies, fenced markdown,
  fabricated imports. Five distinct catches today.
- The HARD-DENY ALLOWLIST (constraint harness, deterministic path
  prefix check) is fully reliable across all twin runs.
- The LLM JUDGE (twin's qwen2.5-coder:14b call) is unstable. Hallucinated
  policy in 4+ separate verdicts, ignored explicit corrective prompts.

The pattern is the harness-engineering pattern: deterministic gates do
the cheap heavy lifting; LLMs are unreliable for narrow policy decisions
that have a deterministic answer. The twin runs LLM judgment on every
action including ones a regex could decide.

## Concrete next workplan candidates (ranked by ROI)

### (a) Deterministic-first twin refactor — HIGHEST ROI

Twin currently runs an LLM judge on every proposed_action. Restructure as:

1. **Deterministic gates (90%+ of decisions):**
   - Path allowlist match (already exists; broaden to cover .ts/.go/etc.)
   - Extension-content sniff (regex: .rs starts with `use|pub|fn|mod|#\[|//`; .go starts with `package`; .ts contains `export|import|function|const|let`)
   - Size bounds + destructive-pattern grep
2. **LLM judge ONLY when:**
   - Path passes allowlist AND content sniff passes BUT semantic match-to-ask is genuinely ambiguous
   - Persona claims fabricated facts ("secure channel", invented roles)
   - Size > 30KB or security-sensitive content patterns

Estimated cost: ~200 LoC in `twin_reviewer.rs`, ~10 unit tests, no ADR
needed (already aligned with ADR-2026-05-08-2500's "tool:* skips redundant
LLM judge" pattern — this just narrows it further).

### (b) Meta-harness v0 (the IRIS-style loop)

Use the now-live `agent_trajectory` + `agent_step` data to learn:
- Which tool sequences correlate with TerminalAction + cargo-test-pass
- Which prompt fragments correlate with twin approval
- Which rejection rationales appear most often (so we can patch them
  out at the prompt level OR the deterministic-gate level)

This is the workplan I drafted notes for in the AI-harness-engineering
discussion. Probably 1000-1500 LoC. Prerequisite: (a) lands first to
clean up the noisy LLM-judge signal.

### (c) Multi-language proof completion

Once (a) lands, multi-language probes (Go + TS smoke tests) should pass
twin trivially (the deterministic gate covers `examples/*` paths
already). Pull `gofmt` + `tsc` to PATH (done today). Extend
precompile_check to dispatch by extension (already designed; partial
patch reverted today pending hex-coder dispatch). Roughly half a day.

### (d) Drafter-ceiling investigation

The 5k-token Rust ceiling at qwen2.5-coder:14b is a real boundary but
not blocking — most autonomous work is small-scope. Document the
boundary in the workplan acceptance criteria ("drafts > 80 lines may
need operator-passthrough or T3 escalation") and move on.

## What we shipped today (in numbers)

- 25-ish commits, 3 of which are reverts (real findings preserved in
  git history)
- 5 net autonomous commits authored by `hex-autonomous`
- 70+ unit tests across `agent_loop`, `drafter_trajectory`,
  `observability`
- 6 new STDB tables / reducers (`agent_trajectory`, `agent_step` +
  their open/record/close trio)
- ~3000 lines of net new Rust in hex-nexus/src
- 2 proof writeups + this findings doc

## What's NOT shipped

- Multi-language Go autonomous proof (TypeScript landed under Devstral
  twin; Go dispatch pending — same chain should work)
- Solid `/agent-trajectories` view (data is in STDB, presentation
  deferred)
- STDB `compile_status` column for proposed_action (deferred)
- Meta-harness loop (the next obvious tier-2 step per your taxonomy)
- Drafter capability calibration beyond ~80-line drafts
