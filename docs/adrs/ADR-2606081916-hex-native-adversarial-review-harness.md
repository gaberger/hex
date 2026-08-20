# ADR-2606081916: hex-native adversarial review harness — `hex swarm review`

**Status:** Accepted
**Date:** 2026-06-08
**Epoch:** hybrid-inference
**Drivers:** A 25-agent cooperative+adversarial workflow built a 2871-LOC concurrent durable job
queue whose *own passing tests* still hid 6 real bugs — an independent adversarial pass found
them. The single-agent loop (`hex do`) and parallel workers (`hex swarm run`) had no equivalent:
hex could *build* and *fan out*, but it could not *adversarially harden*. That is exactly the
capability the retired org-sim tried (and failed, as theater) to provide.
**Supersedes:**
**Superseded-By:**

## Context

The session's throughline — *tests can mirror the bug; independent oracles find what the builder
misses* — was demonstrated at scale: evidence-gated build alone shipped code with a silent
data-loss bug. The valuable, repeatable move was the **adversarial pass on the built artifact**:
multiple independent reviewers, each on a distinct failure class, hunt bugs; each finding is
skeptically verified; confirmed ones are fixed against a ground-truth gate. That pass found real
bugs the build's tests passed over. It was orchestrated by an external harness (Claude Code's
Workflow); hex needed its own.

## Decision

Add **`hex swarm review <path> --gate '<test cmd>'`** — the adversarial half of the
cooperative+adversarial harness, as a first-class hex capability:

```
hunt   — 4 parallel `claude -p` reviewers, each a failure-class lens
         (correctness, concurrency, durability/safety, edges); each emits structured JSON findings
verify — each finding skeptically re-checked by an independent `claude -p`, DEFAULT-REFUTE,
         so plausible-but-wrong findings die before any edit
fix    — confirmed bugs fixed sequentially; each fix gated by the test command (must exit 0),
         the same evidence-gate discipline as the do-loop
```

- **Orchestrator** in `hex-exec/src/adversarial.rs` (`run_review`), with a pure, unit-tested
  `extract_json` for parsing agent prose. Agents are `claude -p` workers — hex's frontier path.
- **CLI verb** in `hex-cli` (`hex swarm review`), alongside `hex swarm run` (parallel workers).
- Fixes are applied to the working tree and left **uncommitted** for operator review.

## Consequences

**Positive**
- hex can now **autonomously harden** code — independent adversaries + a ground-truth gate, no human.
- Validated on first dogfood: pointed at the (already 6-bug-hardened) job queue, it found a **7th
  real bug** the 25-agent workflow missed (`u64` overflow in `fail()`'s retry deadline), refuted a
  false candidate, fixed it with a regression test, gate PASS. **Adversarial review compounds.**
- Reuses the do-loop's evidence-gate and the `claude -p` delegate — no new execution primitive.

**Negative / gap (the cooperative half is NOT yet hex-native)**
- This is the **adversarial/hardening** half. The **cooperative-design** half (diverge → red-team
  designs → synthesize → build) is still orchestrated by an external workflow, not hex. So hex
  *hardens* on its own; it does not yet *design* on its own. That remains open (a future
  `hex swarm build`-style verb that pipelines design→critique→synthesize→build→review).
- `claude -p` per agent ⇒ real latency/cost; the verb is for deliberate hardening passes, not a
  per-keystroke check.

## Implementation

- `hex-exec/src/adversarial.rs` — `run_review` (hunt/verify/fix), `extract_json` (+ 5 unit tests). *(this ADR)*
- `hex-cli` — `SwarmAction::Review`, `run_adversarial_review`; new `hex-exec` path dependency. *(this ADR)*
- Dogfood committed: `de3a949d` (the overflow bug it found + fixed in the job queue).

## References

- The cooperative+adversarial job-queue workflow (`510e82c1`) — the reference this distills.
- ADR-2606072044 — best-of-N + `claude -p` frontier fallback (the agent backend).
- ADR-2606071500 — the ReAct do-loop (the evidence-gate discipline this reuses).
- `hex swarm run` — the parallel-worker primitive this sits beside.
