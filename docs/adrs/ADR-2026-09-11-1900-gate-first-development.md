# ADR-2026-09-11-1900: Gate-first development — retire specs-first

**Status:** Proposed
**Date:** 2026-09-11
**Epoch:** solo
**Drivers:** Operator directive: "we want a better solution than spec-driven development." Backed by measurement taken during the solo collapse (ADR-2608241500), where a 36-task spec-driven workplan and its 18 behavioural specs were executed end to end and their failure modes recorded as they happened.
**Relates-To:** ADR-2608241500 (collapse to a solo agent), ADR-2026-06-04-1740 (the evidence gate), ADR-2026-05-19-0720 (evidence-gate semantics), ADR-2606071500 (ReAct tool-use loop)

<!-- LIFECYCLE: Proposed → Accepted → Completed. Change Status only via adr_status_set / the adr-steward. -->

## Context

`CLAUDE.md` prescribes a **specs-first** pipeline:

> 1. Decide — an ADR. 2. **Specify — behavioral specs before code.** 3. Build.
> 4. Test. 5. Validate. 6. Ship.

The solo collapse ran that pipeline at full scale — one ADR, 18 behavioural
specs (`docs/specs/hex-solo.json`), a 36-task workplan — against ~200k lines of
deletion. That is the largest single exercise of the method this project has
performed, and it produced evidence rather than opinion.

### What the specs got wrong, specifically

Four of thirty-six tasks were not merely imprecise. They were **actively
harmful**, and each was caught by the compiler or by a consumer trace, never by
a spec:

- **P7.2 would have broken the build.** It deletes `hex-core/src/domain/
  messages.rs` and `api_optimization.rs`. `ports/inference.rs` — which the
  *same workplan*, one task earlier, keeps in as many words as "the G1
  contract" — is written in terms of both, as are all six `hex-infer` adapters.
  Two adjacent tasks in one document contradicted each other and nothing
  noticed, because nothing executed the document.
- **P6.1 lists three verbs as daemon-mediated that have zero daemon
  references.** `interview`, `insight` and `refresh`. Deleting them would have
  removed working features for a reason that does not apply to them.
- **P6.3 says delete `reconcile`** because it "resyncs workplan state with
  daemon-run agents". What it does is check a workplan's claimed status against
  git evidence — which matters *more* after the collapse, not less.
- **S09 undercounts its own subject.** "7 committed `.wasm` blobs, 1.5 MB." It
  is 8 blobs and 3.43 MB. The acceptance threshold derived from that number was
  therefore wrong by 1.9 MB.

### What the specs did to themselves

**44 of 110 specs described features that no longer existed**, and nothing ever
failed. `dashboard-session-cleanup.json`, `cli-chat-tui.json`,
`coo-observability-baseline.md` — subjects deleted months earlier by
ADR-2606061359. The specs sat there being confidently wrong.

Of the 17 JSON specs that survived the audit, **1 names a command that can be
run.** The other 16 are `given` / `when` / `then` prose.

That ratio is the whole finding. A spec that cannot be executed cannot fail,
and a document that cannot fail cannot be trusted, because nothing distinguishes
it from a document that is wrong.

### What a spec does to its own tests

`hex-nexus/tests/worktree_enforcement.rs` was 30 tests across "all 8 phases" of
ADR-2026-03-23-1700 — and imported nothing but `std`. `SessionState` and the
hook logic were **redefined inside the test file**. Every assertion held against
a copy of the design rather than the shipped code.

This is the "tests can mirror bugs" lesson in its worst form. Deriving tests
from a spec, by the same process that wrote the spec, does not produce an
oracle. It produces a second copy of the same belief. Deleting those 30 tests
cost no coverage because they had never covered anything that ran.

### What worked instead, measured in the same session

Given a one-line challenge and one shell command, with **no human-written
spec**:

```bash
hex build "A thread-safe token-bucket rate limiter…" \
  --target examples/ratelimiter-proof \
  --gate "cargo test --manifest-path examples/ratelimiter-proof/Cargo.toml"
```

→ 2 designs, 2 critiques, a 16,986-character spec *synthesized as a disposable
intermediate*, 777 lines, gate GREEN, 14 tests passing on an independent run.

Then `hex harden` against the same gate found **3 real bugs the build's own 14
passing tests missed** — every candidate surviving skeptical verification. The
most valuable was a confidently-stated false premise in a comment:

> `u128` cannot overflow on a product of two `u64` values, so this does the job
> of `checked_mul`.

`Duration::as_nanos` returns a `u128`, not a `u64`. The product overflows, an
`as` cast truncates it, and the result is a rate limiter that silently limits
nothing. **No spec would have caught that**, because a spec describes intent and
the intent here was correct. Only an adversary reading the code finds it.

The same shape held in Rust, and the benchmark corpus exercises it in Go and
TypeScript too.

## Decision

**The executable gate replaces the written spec. A spec that cannot be run is
an ADR.**

### 1. The pipeline

| | Was | Is |
|---|---|---|
| 1 | Decide — ADR | **Decide** — ADR. Unchanged. |
| 2 | **Specify** — behavioural specs before code | **Gate** — write the command that must exit 0, before the code. |
| 3 | Build | **Diverge** — N designs, each red-teamed. The spec is synthesized here, and is disposable. |
| 4 | Test | **Build to the gate.** |
| 5 | Validate | **Harden** — adversarial hunt, default-refute, each fix gated. |
| 6 | Ship | **Ship.** |

Steps 3–5 are `hex build --harden`. Step 2 is `--gate` / `--evidence`. hex has
already built this; only the documentation lagged.

### 2. Three rules

- **A spec that cannot be run does not exist.** Either it becomes a gate, or it
  becomes ADR prose — a recorded decision with a rationale, which is history and
  is allowed to be unexecutable because it never claims to describe the present.
- **The gate is written before the code and is not derived from it.** A gate
  generated from the implementation is the mirror-test failure wearing a new
  hat.
- **A vacuous gate is a failed gate.** `evidence_is_vacuous` already rejects
  "running 0 tests" and "0 passed; 0 failed". That guard is what makes the
  method safe to rely on, and it must extend to every new gate shape.

### 3. `docs/specs/` shrinks to the ones that execute

The 16 prose specs become ADR sections or gates. `docs/specs/` keeps only
artifacts that name a runnable command.

### Alternatives considered

1. **Keep specs, enforce freshness with a linter.** Rejected. The linter can
   check that a spec's referenced paths exist; it cannot check that a spec's
   *claims* are still true. P7.2 referenced real files and was still wrong.
2. **Property-based testing as the oracle.** Partially adopted, not sufficient.
   Property tests are an excellent gate shape and `CLAUDE.md` already
   recommends them. They do not find a false premise in a comment.
3. **Keep specs for design communication only, drop their authority.**
   Rejected as ambiguous: a document with no authority still gets read and
   believed. Better to move it to an ADR, where "this was decided then" is the
   explicit contract.

## Consequences

**Positive:**
- Rot becomes impossible to ignore. A stale gate fails; a stale spec sits.
- The oracle becomes independent of the author. The gate is a command; it does
  not share the implementation's assumptions.
- Intent stops being duplicated in two places that can disagree — which is
  exactly how P7.1 and P7.2 came to contradict each other.
- One fewer artifact to keep current.

**Negative and honest:**
- **A gate says when you are done, not what to build.** Intent still needs a
  home. It lives in the ADR and in the one-line challenge, and both are prose,
  and prose can be wrong. This ADR narrows where unexecutable text has
  authority; it does not eliminate it.
- **Non-functional properties resist gating.** "Fast", "secure", "readable".
  Some gate cleanly (`cargo bench`, `cargo audit`, `clippy -D warnings`); some
  do not. `hex harden`'s lenses are the oracle where a gate cannot reach, and
  they are a weaker oracle: an adversary that misses something leaves no trace,
  where a failing gate is loud.
- **Writing a good gate is harder than writing a good spec**, and the failure is
  quieter — a gate that passes for the wrong reason looks exactly like success.
  `evidence_is_vacuous` covers the crudest case and nothing covers the subtle
  one.
- **Discoverability drops.** "Read the specs" told a newcomer what the system
  promises. "Read the gates" tells them what it is checked for, which is a
  narrower and less welcoming answer. The ADR ledger has to carry more of that
  weight.

**Neutral:**
- All 263 ADRs are untouched. This ADR *increases* their role.
- `hex spec` survives for the executable remainder.

## Verification

Falsifiable, and both gates are blocking:

1. **No surviving artifact in `docs/specs/` lacks a runnable command.** Checked
   by a `hex ci` gate that greps every spec for one and exits non-zero
   otherwise. If a spec cannot get one, it belongs in an ADR.
2. **A deliberately wrong gate must fail loudly.** Add a fixture whose gate
   passes vacuously and confirm `evidence_is_vacuous` rejects it — the guard
   this decision leans on has to be tested, not assumed.

The strongest disconfirming evidence would be a defect class that a written
spec catches and a gate plus adversarial pass does not. None appeared in the
solo collapse, but one exercise is one exercise, and this ADR should be
revisited if one shows up.
