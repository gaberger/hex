# Test salvage audit

**Workplan:** `wp-2608241500-hex-solo` P4.2
**Spec:** S06 — *test coverage that exercises surviving crates is ported forward
before the daemon is deleted, and the deleted count is recorded rather than
letting it vanish silently*
**Date:** 2026-09-11

---

## 1. The inventory

| Crate | Test attributes | Fate |
|---|---:|---|
| `hex-nexus` | 946 | deleted (P5.2) |
| `hex-cli` | 690 | mostly survives; the deleted verbs take theirs with them (P6.1) |
| `hex-agent` | 238 | deleted (P5.1) |
| `hex-state` | 13 | deleted (P5.3) |
| `hex-core` | 122 | survives |
| `hex-exec` | 121 | survives |
| `hex-infer` | 108 → **131** | survives, and grew here |
| `hex-analysis` | 89 | survives |
| `hex-graph` | 15 | survives |
| `hex-git` | 6 | survives |

**1,197 test attributes sit inside crates scheduled for deletion.** This audit
decides, for each cluster, whether it asserts something about a surviving crate.

---

## 2. What was ported forward

### 2.1 Tier classification → `hex-infer/src/routing.rs` (+11 tests)

`hex-nexus/src/orchestration/workplan_executor.rs::classify_task_tier` and the
nine classifier cases in `hex-nexus/tests/tier_routing.rs`.

The daemon classified tasks into inference tiers on behalf of `hex plan
execute`. That verb survives and will run in-process, so it still needs the
classification — and tier routing is what `hex-infer` is for. The function moved
as `hex_infer::classify_tier`, over a `TaskShape` the caller projects onto,
rather than the daemon's `WorkplanTask`: this crate places inference calls and
must not take a position on the workplan schema.

The salvaged behavior includes three rules a rewrite would have dropped:

- explicit `tier` beats every heuristic;
- the front-end design escalation to T2.5, which exists because of a recorded
  lesson (`lesson:tier-routing-for-ui`, 2026-05-31) — standard codegen produces
  unstyled output;
- the conservative default. Under-classifying costs a retry; over-classifying
  spends frontier budget on a rename.

Ported as 11 tests rather than 9: the strategy-hint and agent-role cases were
table-driven into one test each, and two were added for `Tier`'s JSON round
trip, since `T2.5` is the one variant whose wire spelling differs from its
name.

### 2.2 Provider adapter tests → `hex-infer/tests/` (12 tests)

`hex-nexus/tests/ollama_adapter.rs` and `claude_code_adapter.rs`, moved with
`git mv` and repointed from `hex_nexus::adapters::inference::` to
`hex_infer::providers::`. They were already hermetic — all network traffic goes
through an in-process `httpmock` server — so they needed no other change.

These exercise the real production code P2.2 relocated. The adapters' *in-file*
unit tests came across with the files in P2.2; these integration tests were left
behind in `hex-nexus/tests/` and would have been deleted with it.

`httpmock` added to `hex-infer`'s dev-dependencies.

---

## 3. What is deleted, and why it costs no coverage

### 3.1 Tests with no production code under them (57)

| File | Tests | Finding |
|---|---:|---|
| `hex-nexus/tests/worktree_enforcement.rs` | 30 | Imports nothing but `std`. `SessionState` and the hook logic are **redefined inside the test file**. |
| `hex-nexus/tests/merge_gate_reducers.rs` | 27 | Imports only `std`, `serde_json`, `uuid`. Drives a live SpacetimeDB over HTTP. |

`worktree_enforcement.rs` is worth naming explicitly. Thirty tests across "all 8
phases" of ADR-2026-03-23-1700, asserting against a copy of the design rather
than the shipped code — the exact failure CLAUDE.md warns about under *"tests
can mirror bugs"*. Deleting them removes no coverage of anything that runs,
because they never covered anything that runs.

`hex worktree` survives as a verb. If its behavior deserves tests, they belong
in `hex-git` against the real implementation, and they do not exist today. That
is a coverage gap the collapse **reveals**, not one it creates — recorded here
rather than fixed, because writing them is not this workplan's job.

### 3.2 Tests whose subject is deleted (~1,100)

| Cluster | Tests | Subject |
|---|---:|---|
| `hex-nexus/src/orchestration/` | 332 | SOP loop, workplan conductor, adversarial swarm, auto-repair, classifiers, shadow router |
| `hex-nexus/src/research/` | 111 | the research analyst fleet |
| `hex-nexus/src/adapters/` | 96 | SpacetimeDB state adapters |
| `hex-nexus/src/*.rs` (top level) | 90 | sched service, quant router, task-type classifier |
| `hex-nexus/src/routes/` | 70 | axum handlers |
| `hex-nexus/src/coordination/` | 12 | HexFlo swarm coordination |
| `hex-agent` | 238 | the second agent loop, superseded by `hex-exec` |
| `hex-state` | 13 | the STDB state adapter |

Every one asserts daemon routing, SpacetimeDB reducers, fleet coordination, or
the retired SOP pipeline. There is no surviving code for them to describe.

### 3.3 Coverage replaced rather than ported (19)

| File | Tests | Replaced by |
|---|---:|---|
| `hex-nexus/tests/hexflo_memory_adapter.rs` | 14 | `hex-exec/src/store` — 16 tests over `FileStore` |
| `hex-nexus/tests/hexflo_memory_e2e.rs` | 5 | same |

The memory *store* changed substrate (P3.3), so the old tests describe an
adapter that no longer exists. The guarantees they asserted — a write reads
back, a rewrite does not duplicate, a missing store reads empty — are covered by
the new suite, which additionally covers cases the STDB adapter could not have:
project scope shadowing global, two keys that sanitize to the same filename, and
a log truncated mid-write.

### 3.4 Tests for features being removed (~50)

`quant_routing.rs` (19) and `routing_integration.rs` (6) exercise the
quantization router and task-type classifier; `hex-core/src/quantization.rs`
goes in P7.3. `docker_sandbox_e2e.rs` (15), `q_report_route.rs` (11),
`dashboard_visibility.rs` (5), `remote_agent_integration.rs` (5),
`lora_enforcement_external.rs` (2) and the `send_task_to_agent` /
`dispatch_ui_task` / `complex_app_task` singles all target capabilities the ADR
retires.

---

## 4. Recorded count

| Outcome | Tests |
|---|---:|
| Ported into a surviving crate | **23** (11 classification + 12 adapter) |
| Coverage replaced by new tests in a surviving crate | 19 |
| Deleted — subject deleted | ~1,100 |
| Deleted — never exercised production code | 57 |

Deleting roughly 1,150 tests looks alarming written down, so state the ratio
plainly: after this audit, 23 tests of real behavior were at risk of vanishing,
and all 23 were ported. The rest describe a daemon, a fleet, and a database that
will not exist.

---

## 5. Open item

`hex worktree` survives with no tests against its real implementation (§3.1).
Not fixed here. Worth a task of its own once the deletion phases land and the
verb's surviving surface is settled.
