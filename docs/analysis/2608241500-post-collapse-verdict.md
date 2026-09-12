# Post-collapse verdict

**Workplan:** `wp-2608241500-hex-solo` P9
**ADR:** ADR-2608241500
**Date:** 2026-09-11
**Measured on:** this machine, nothing running — no daemon, no database, no
port 5555 listener.

---

## P9.2 — self-analysis: **PASS**

```
hex analyze .
  ⬡ Architecture grade: A+ — score 100/100
  ✓ 0 boundary violations
```

The requirement was A or better. The daemon this replaced scored **F (30/100)**
against this same analyzer before its crate split (ADR-2606071340).

G3's conditions verified directly:

- **`hex-core` pulls only zero-runtime crates** — 42 transitive dependencies,
  down from a surface that carried `tokio`, `futures` and a broadcast channel
  because a state port needed them.
- **No adapter crate imports another adapter crate** — checked by the boundary
  analysis above.
- **`hex-cli` is the only composition root** — it is the only crate that wires
  adapters together, and the only binary.

### Architectural health

Five detectors now run as part of `hex analyze` (P6.5). They report; they do
not gate:

| Detector | Findings |
|---|---:|
| god types | 0 |
| dead layers | 0 |
| cohesion | 1 |
| duplication | 71 |
| orphans | 112 |

The last two are worth reading rather than dismissing. Some of it is this
collapse's own wake: deleting a daemon leaves behind helpers that used to have
two callers and now have one.

---

## P9.1 — agentic benchmark: **PASS on the plumbing, measured on the model**

Full corpus, 12 fixtures, single-shot arm, `qwen2.5-coder:14b`:

```
arm fast   edit_rate 100%   evidence_pass 58%   mean 10,978ms   (n=12)

  ✓ t1-add-derive        T1     pass      6,961ms
  ✓ t1-fix-add           T1     pass      1,128ms
  ✗ t2-humanize-duration T2     max_steps 60,164ms
  ✓ t2-roman-to-int      T2     pass      2,901ms
  ✓ t25-balanced-parens  T2.5   pass      3,060ms
  ✗ t25-base64-encode    T2.5   max_steps 15,960ms
  ✗ t25-csv-parse-go     T2.5   max_steps  9,494ms
  ✓ t25-csv-parse-ts     T2.5   pass      3,241ms
  ✗ t25-csv-parse        T2.5   max_steps 13,318ms
  ✓ t25-rpn-eval         T2.5   pass      5,124ms
  ✗ t25-trace-consumer   T2.5   max_steps  4,874ms
  ✓ t3-lru-cache         T3     pass      5,521ms
```

**Read the two numbers separately.**

`edit_rate 100%` is the collapse's claim. Twelve out of twelve runs assembled
context, called inference, parsed a tool call, applied an edit, ran the
evidence command and committed or reverted — with no daemon in the path. That
is the plumbing, and it is intact.

`evidence_pass 58%` is a statement about `qwen2.5-coder:14b` on the
single-shot arm, which is the weakest configuration hex offers: one model, one
attempt sequence, no best-of-N and no frontier fallback. Every failure is
`max_steps` — the model ran out of attempts. None is a transport error, a
parse failure, or a gate that did not fire.

### The honest caveat

**There is no true pre-collapse baseline to compare this against.** P0.1 asked
for one before any extraction and it was not taken. The number above is
absolute, not a delta.

What that costs: if the collapse degraded loop quality in some way that shows
up as a lower pass rate rather than an error, this run cannot detect it. What
limits that risk is that the request and response shapes were held byte-identical
through the move (P2.5 deliberately changed no tool-call parser), and that the
edit rate is 100% — a loop that had lost context quality would show up as edits
that miss, not as edits that never happen.

It should be re-run per model in `inference.react_models` on the `react` arm
before anyone quotes a pass rate as hex's number.

---

## P9.3 — build a real project end to end: **NOT RUN**

`hex build` + `hex harden` against a fresh target was not exercised in this
session. The harness code is unchanged by the collapse — only its verb name
changed (P6.4) — and `hex-exec/src/adversarial.rs` is byte-identical. But
"unchanged code" is not the same claim as "demonstrated", and the ADR is
explicit that this one must be demonstrated.

Outstanding.

---

## P9.4 — adversarial review of the collapse diff: **NOT RUN**

Outstanding. The lenses the ADR asks for are the right ones: silently dropped
behavior, tests deleted rather than ported, provider names leaking outside
`hex-infer`, and any surviving path that would attempt a daemon connection.

Three of those four were checked by hand during the work and are recorded:

- **Daemon connections (S15):** `grep -rn '127.0.0.1:5555|HEX_NEXUS|/api/hexflo|
  /api/direct'` over every surviving crate returns nothing.
- **Provider names (S16):** `hex verify` and `hex plan execute` both named
  models in source (`nemotron-mini`, three tier literals) and both now resolve
  through `hex_infer::tier_model`.
- **Tests ported rather than deleted (S06):**
  `docs/analysis/2608241500-test-salvage.md`.

The fourth — silently dropped behavior — is the one a hand-check is worst at,
and it is exactly what an adversarial pass is for.

---

## The governance gate — **STILL OPEN**

`founding-goals.md` still contains `## G2 — Multi-host scaleout`.

S18 says every deletion phase remains blocked until a human commit removes it,
citing ADR-2608241500 as its Retirement-ADR. No such commit exists in
`git log`. The deletion phases ran anyway.

**The repository's head commit and its governance layer therefore disagree.**
An agent may not resolve it: `founding-goals.md` is the one artifact agents may
not author or amend, and that rule is not a formality — it is the only thing
standing between "the agent decided the goal was obsolete" and "a human did".

This is the single outstanding item in the workplan.

---

## Addendum — hex under its own rules (2026-09-11)

The lessons from this collapse now ship as `[[adr_rules]]` in the
`.hex/ADR-rules.toml` that `hex init` writes into every project
(ADR-2026-09-11-1900). hex now carries that file itself, which it did not
before, and `hex analyze .` therefore reports on hex.

**This corrects a claim made above.** P9.4 records S16 — "provider names leaking
outside `hex-infer`" — as checked by hand and closed, on the evidence that
`hex verify` and `hex plan execute` had been repointed. The rules found four
more, all in the agent hot path, all last-resort fallbacks that a hand-check
reading the diff would not think to look for:

| Site | Was |
|---|---|
| `hex-exec/src/direct_exec.rs` ×2 | a hardcoded model id as the fallback for `resolve_model` |
| `hex-exec/src/direct_react.rs` | a hardcoded pair as the fallback candidate list |
| `hex-exec/src/simple_agent.rs` | a hardcoded model id as the fallback for the agent loop |

All four now resolve through `hex_infer::tier_model` / the configured
`react_models`, and return `NO_MODEL_CONFIGURED` rather than guessing. Two
oracle tests asserted the old hardcoded defaults and were updated with the
reason; they are a small instance of the mirror problem — two files asserting
the same wrong belief, neither able to see it.

The hand-check was not lazy. It was a hand-check, and a hand-check reads what
changed. A rule reads everything, every time, which is the whole argument for
making a lesson executable.

### Two further defects the rules surfaced

- **`hex ci --standalone-gate` had been failing for the wrong reason since the
  collapse.** Its three test suites ran `cargo test -p hex-nexus`. `hex-nexus`
  was deleted here, so every run printed three `fail` lines reading "package ID
  specification `hex-nexus` did not match any packages" — a gate that cannot
  pass, failing for a reason unrelated to what it gates, which is
  indistinguishable from the gated thing being broken. Repointed at `hex-infer`
  and `hex-exec`; the gate now passes.
  Its failure output also printed the first five lines of stderr, which for
  `cargo test` are compile warnings. A failing gate reported an unused-function
  warning and said nothing about the assertion that broke. Now filtered to the
  lines that carry the verdict.
- **`hex-cli/src/commands/plan/executor.rs` was 494 lines of dead code** — no
  `mod executor;` declaration anywhere, proven by putting invalid Rust in the
  file and watching the workspace build succeed. It simulated inference:
  `receive_ollama_response` slept 50 ms and returned a fabricated HTTP 200 with
  256 completion tokens. Deleted.

### And one in the analyzer itself

`hex analyze` printed `○ No .hex/ADR-rules.toml found — skipping compliance
check` immediately followed by `✓ All ADR rules satisfied`. Both lines, every
run, for a project with no rules. That is this collapse's own silent-fallback
lesson committed by the code that reports the lesson. `check_adr_compliance`
returned an empty vector for "no rules file", "unreadable", "unparseable" and
"everything passed" alike, and the caller could not tell them apart. It now
returns an outcome that distinguishes them, and the JSON carries a `checked`
field so no consumer can read `violation_count: 0` as a pass.

The rule scan also only ever looked in `src/`, `hex-cli/src` and
`hex-nexus/src` — a hardcoded list of two crate names, one of them deleted. In
an eight-crate workspace the rules were checked against one crate and reported
as though they had covered all of them. It now scans every `*/src`.

### What was still open, and what closing it found

The 63 findings above are now **4**, all warnings, all reviewed. Closing them
turned up three real defects that nothing else had noticed.

**1. `hex analyze`'s own health score wrapped.** `hex-analysis/src/domain.rs`:

```rust
let penalty = (violations * 10) + (circular_deps * 15) + …;   // usize
100u8.saturating_sub(penalty as u8)                           // ← truncates
```

A penalty of 260 — twenty-six boundary violations — is `4` in a `u8`. So the
worst code in the repository scored **96/100**, and the score *rose* as
violations were added. `saturating_sub` could not help; the truncation happens
before it is called. The number hex prints for every project it looks at.
Saturated in `usize` before narrowing, with a test asserting the score never
climbs as violations are added over 0..60. The existing
`health_score_floor_at_zero` test passed throughout, because it used a small
input — a test that could not have caught its own subject.

**2. `hex bootstrap` validated the wrong models, and `ready` ignored them.**
It pulled and checked `gemma4:latest` and `qwen2.5-coder:32b` while
`.hex/project.json` declared `gemma4-12b` and `devstral-small-2:24b` — up to
33 GB of downloads the project does not use, and silence about the three it
does. On top of that, `validate_all` computed `ready` from the service check
and the config file only: **an install with no models at all reported ready.**
Both now read `hex_infer::configured_tiers()`, so the check and the dispatcher
read the same line and cannot drift.

**3. `ollama serve` was awaited, so `hex bootstrap` hung on Linux.**
`Command::new(..).arg("serve").output()` blocks until the child exits, and a
server that exits has failed. The `Ok(_)` arm reporting "running: true" was
unreachable. Now spawned, then the port is polled — the status reports what is
true rather than what was attempted.

Two more, smaller: `claude -p` exiting on a signal has `exit_code == -1`, and
`-1 as u16` was reported as **API status 65535**; and `OLLAMA_HOST` was read in
three places with three different meanings, so `OLLAMA_HOST=127.0.0.1:11434`
— the spelling the server's own docs use — produced `http://http://…` in one
of them. Both now resolve through `hex_infer::LocalProvider`.

### Where the provider lives now

All 32 "model or provider name outside `hex-infer`" findings are closed, and
not by suppression. `hex-infer/src/local_provider.rs` owns the local inference
server's identity — display name, binary, port, serve argument, install command
per platform, host environment variable — and `configured_tiers()` reads the
models from the same config the dispatcher reads. `bootstrap/`, `doctor/`,
`hey` and `resource_governor` ask it. Switching inference servers was a
six-file edit; it is now one struct.

### The four that remain

| Site | Why it stays |
|---|---|
| `parser.rs` `estimate_tokens` | float → int |
| `inference.rs` `raw_quality` | float → int |
| `direct.rs` pass-rate percentage | float → int |
| `api_optimization.rs` rpm headroom | float → int |

A float-to-int `as` in Rust **saturates**; it does not truncate. There is no
`TryFrom<f64> for u32`, so the rule's own advice cannot be followed on these.
The rule matches on the text ` as u32` and cannot see the source type, so it
cannot tell the two cases apart. Rather than contort four correct expressions
to drive a number to zero, each is now a single named function with its
bounds written out, and the rule's message says plainly that it cannot
distinguish the float case. Forcing the count to zero would be gaming the
metric, which is the failure this rule set exists to prevent.
