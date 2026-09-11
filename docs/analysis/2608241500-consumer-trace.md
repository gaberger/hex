# Consumer trace for the solo collapse

**Workplan:** `wp-2608241500-hex-solo` P4.1
**ADR:** ADR-2608241500
**Spec:** S17 — *no deletion is performed without a consumer trace*
**Date:** 2026-09-11
**Graph:** `graph-out/graph.json`, rebuilt 2026-09-11 — 20,157 nodes, 24,925 edges, 1,890 communities

ADR-2026-04-05-0900 records `hex-agent` being broken for an entire session because
a workplan deleted code whose feature-gated importers were never traced. This
document is the check that must not be skipped again.

---

## 0. How this was produced, and where it can be wrong

Three independent passes, because each misses something the others catch:

| Pass | Catches | Misses |
|---|---|---|
| `Cargo.toml` dependency map | crate-level edges, exactly | anything inside a crate |
| `hex graph consumers` (AST) | file imports and entity uses | nothing feature-gated out of the AST |
| `grep` over `hex_core::` paths | type-level uses across crates | uses through a re-export alias |

**A caveat on `hex graph consumers`.** Entity consumers are matched by symbol
*name*, not by resolved path, so a common name over-reports. `hex-cli/src/tui`
appears to be used by four `hex-agent` files "using `TaskStatus`" — `hex-agent`
defines its own unrelated `TaskStatus`. Over-reporting is the safe direction for
a deletion oracle and is left uncorrected.

**The graph rebuild itself was blocked.** `hex graph build` required the daemon
(`Cannot reach hex-nexus at http://127.0.0.1:5555`) while `hex graph consumers`
read the file directly. The excision oracle could therefore be run but its input
could never be refreshed without the thing being excised. The graph on disk was
three months stale (10 June). `hex graph build`, `query`, `path`, `explain` and
`context` were moved in-process before this trace was run; the numbers above come
from a graph built with no daemon running.

---

## 1. Crate dependency map

Who names whom in a `Cargo.toml`:

| Crate | LOC | Depended on by | Verdict |
|---|---:|---|---|
| `hex-nexus` | 95,764 | `hex-desktop`, `hex-agent` | both are also deleted → **free after them** |
| `hex-agent` | 23,395 | — | **free** |
| `hex-desktop` | 782 | — | **free** (already dropped from `[workspace] members`, P0.5) |
| `hex-state` | 3,252 | `hex-nexus` | **free after P5.2** |
| `spacetime-modules` | 13,165 | — (separate workspace) | **free** |
| `hex-analyzer` | 4,500 | — | **free**; P6.5 folds its analyzers into `hex-analysis` first |
| `hex-parser` | 1,115 | — | not a deletion target, but nothing depends on it — see §6 |
| `hex-analysis` | 4,683 | **`hex-nexus` only** | ⚠️ **orphaned by P5.2** — see §5 |
| `hex-core` | 8,763 | cli, exec, nexus, agent, infer, state | survives |
| `hex-exec` | 9,613 | `hex-cli`, `hex-nexus` | survives |
| `hex-graph` | 2,397 | cli, exec, nexus | survives |
| `hex-git` | 1,202 | `hex-exec`, `hex-nexus` | survives |
| `hex-infer` | 5,422 | `hex-exec`, `hex-nexus` | survives |
| `hex-cli` | 92,645 | — (the binary) | survives, triaged in P6 |

Deletion order is forced by this table: **hex-agent and hex-desktop before
hex-nexus, hex-nexus before hex-state.** The workplan's P5.1 → P5.2 → P5.3
ordering already satisfies it.

---

## 2. Feature-gated imports — the failure mode that caused ADR-2026-04-05-0900

Every `#[cfg(feature = ...)]` in the workspace, all nine of them, gate on
`spacetimedb`:

| File | Count |
|---|---:|
| `hex-agent/src/adapters/secondary/stdb_inference.rs` | 2 |
| `hex-nexus/src/spacetime_bindings/mod.rs` | 5 |
| `hex-state/src/spacetime_state.rs` | 2 |

Crates declaring a `spacetimedb` feature: `hex-agent`, `hex-nexus`, `hex-state`,
`hex-desktop` (which forwards to `hex-nexus/spacetimedb`).

**Finding: zero feature-gated imports exist in any surviving crate.** Every one
is inside a crate the workplan deletes outright. The specific failure this trace
exists to prevent cannot occur for P5. It can still occur for P6 and P7, which
cut *inside* surviving crates — §4 covers those.

---

## 3. Module-level targets inside surviving crates

| Target | LOC | Importers | Entity consumers | Verdict |
|---|---:|---|---|---|
| `hex-cli/src/pipeline/` | 16,850 | `hex-cli/src/lib.rs`, `hex-cli/src/main.rs` | — | **BLOCKED** — cut both declarations (P5.4) |
| `hex-cli/src/tui/` | 6,589 | `hex-cli/src/lib.rs`, `hex-cli/src/main.rs` | `commands/dev.rs` (`TuiApp`), `commands/swarm/mod.rs` and `commands/task.rs` (`extract_task_title`) | **BLOCKED** — `extract_task_title` must move or be inlined before the cut; the `hex-agent`/`hex-core` hits are name collisions on `TaskStatus`, not real |
| `hex-exec/src/tools/delegate.rs` | 218 | `hex-exec/src/tools/mod.rs` | its own registry entry | **BLOCKED by one line** — see §6 |
| `hex-cli/assets/wasm/` | 8 blobs | `hex-cli/src/assets.rs` rust-embed | — | see §7 |

---

## 4. `hex-core` trim (P7) — two blocking conflicts

Consumers **outside** the crates being deleted. Self-references inside `hex-core`
(a port naming its own domain type, e.g. `ports/brain.rs` ↔ `domain/brain.rs`)
are listed but are not blockers, because both sides are removed together.

### 4.1 Ports (P7.1) — all clear

| Port | External consumers | Verdict |
|---|---|---|
| `state.rs` (1,088) | none | SAFE |
| `coordination.rs` | `domain/agents.rs` (also deleted) | SAFE |
| `experiment.rs` | `domain/experiment/` (also deleted) | SAFE |
| `consolidation_memory.rs` | `domain/consolidation.rs` (also deleted) | SAFE |
| `agent_comm.rs`, `heartbeat.rs`, `dead_letter.rs`, `worker_pool.rs`, `brain.rs` | none | SAFE |
| `secret.rs` | `domain/secret_grant.rs` (also deleted) | SAFE |
| `sandbox.rs`, `agent_runtime.rs` | `hex-core/src/lib.rs` re-exports | SAFE — drop the re-export in the same commit |

All twelve ports are removable. Both pairs that reference each other
(`sandbox`/`agent_runtime`, `coordination`/`agents`) are on the same delete list.

### 4.2 Domain types (P7.2 / P7.3) — **two must be kept**

| Type | External consumers | Verdict |
|---|---|---|
| **`messages.rs` (157)** | `hex-core/src/ports/inference.rs`, `ports/inference/mock.rs`, and **all five** `hex-infer` providers | 🔴 **MUST KEEP** |
| **`api_optimization.rs` (305)** | `hex-infer/src/providers/anthropic.rs` (`RateLimitHeaders`) | 🔴 **MUST KEEP** |
| `tools.rs` | `ports/inference.rs`, `hex-infer` (`ToolDefinition`, `ToolInputSchema`) | 🟡 **MUST KEEP** — not on any list, but absent from P7.2's keep list too |
| `composition.rs` (633) | `hex-core/tests/substrate_p5_integration.rs` | remove the test with it — §4.3 |
| `brain.rs`, `capability.rs`, `experiment/`, `consolidation.rs`, `heartbeat.rs`, `sandbox.rs`, `swarm_task.rs`, `secret_grant.rs`, `agents.rs`, `corpus.rs`, `quantization.rs`, `research_finding.rs` | none outside their own doomed ports | SAFE |

**This is a genuine defect in the workplan, not a nuance.** P7.1 keeps
`ports/inference.rs` in as many words — *"KEEP inference.rs and its mock (492 —
the G1 contract)"* — while P7.2 deletes `messages.rs`, which that port is written
in terms of. Executing P7.2 as written breaks the one contract the whole ADR is
organised around, and takes `hex-infer` down with it. **P7.2 must be amended to
keep `messages.rs`, `api_optimization.rs` and `tools.rs`.**

### 4.3 Fix task

`hex-core/tests/substrate_p5_integration.rs` exercises `composition`,
`telemetry` and `messages`. `composition.rs` goes in P7.3, so this test must be
trimmed to its surviving assertions or deleted with a recorded count (S06).

---

## 5. `hex-analysis` is orphaned by P5.2

`hex-analysis` (4,683 LOC — tree-sitter boundary checking, the layer classifier,
dead-export detection, ADR conformance) is depended on by **`hex-nexus` and
nothing else**. `hex-cli` reaches analysis through the daemon over HTTP.

The moment P5.2 lands, the crate that enforces hexagonal rules has no consumer —
and `hex analyze`, the verb S03 and P9.2 are measured on, stops working.

**Fix task (P6.2, blocking before P5.2 is considered complete):** add
`hex-analysis` to `hex-cli`'s dependencies and repoint `hex-cli/src/commands/
analyze.rs` from `NexusClient` to direct library calls. P6.5 then folds
`hex-analyzer` in behind the same entry point.

The same applies, less urgently, to `hex-git` and `hex-infer`: `hex-cli` depends
on neither today and needs both once the daemon is gone.

---

## 6. `hex-exec/src/tools/delegate.rs` — org-sim residue

`delegate` POSTs to `/api/org/send-message` to hand work to another *persona*.
That endpoint and the persona roster were retired by ADR-2606071340 P0; the
daemon serving it is deleted in P5.2. Its only consumer is its own registration
in `hex-exec/src/tools/mod.rs`.

Left in place, it is a tool in the model's catalogue that always fails —
precisely what S15 forbids ("a verb that cannot function without a daemon must
have been deleted, not left to fail at runtime").

**Fix task: delete `delegate.rs` and its `ToolRegistry` entry in P5.5**, with the
rest of the org-sim. It was not on any list.

---

## 7. Embedded WASM — the workplan undercounts

S09 and P5.5 describe "7 committed `.wasm` blobs, 1.5 MB". The directory holds
**8 blobs totalling 3.43 MB**:

| Blob | Bytes |
|---|---:|
| `hexflo_coordination.wasm` | 1,478,053 |
| `inference_gateway.wasm` | 488,022 |
| `neural_lab.wasm` | 398,508 |
| `chat_relay.wasm` | 248,785 |
| `rl_engine.wasm` | 239,815 |
| `secret_grant.wasm` | 213,615 |
| `agent_registry.wasm` | 181,896 |
| `agent_comms.wasm` | 181,064 |
| **Total** | **3,429,758** |

`secret_grant.wasm` was missed and `hexflo_coordination.wasm` grew since the
survey. S09's acceptance threshold — "the release binary is at least 1.4 MB
smaller" — should read **at least 3.4 MB smaller**.

---

## 8. CLI surface

**50 of 91** command files under `hex-cli/src/commands` reference `nexus_client`,
`NexusClient`, `:5555` or `NEXUS_URL`. The survey recorded 52 of 91; the two
closed since are `graph.rs` and `memory/mod.rs`, repointed in this session.

Every one of the 50 is either deleted in P6.1 or repointed in P6.2. None may be
left to fail at runtime (S15).

---

## 9. Deletion order, as forced by this trace

1. **P6.2 partial (pull forward):** `hex-cli` takes direct dependencies on
   `hex-analysis`, `hex-git`, `hex-infer`; `analyze` repointed. *Without this,
   P5.2 orphans the analyzer and P9.2 cannot run.*
2. **P5.4 prerequisite:** move `extract_task_title` out of `hex-cli/src/tui/`.
3. **P5.1** `hex-agent` → **P5.2** `hex-nexus` → **P5.3** `hex-desktop`,
   `hex-state`, `spacetime-modules`. Forced by §1.
4. **P5.5** assets, org-sim YAMLs, `delegate.rs` (§6), 8 WASM blobs (§7).
5. **P7** with the amendment in §4.2: keep `messages.rs`,
   `api_optimization.rs`, `tools.rs`.

Each step ends with a blocking `cargo check --workspace` (S17).

---

## 10. Governance gate

S18: no deletion phase may begin until a human commit removing founding goal
**G2 — Multi-host scaleout** from `founding-goals.md`, citing ADR-2608241500 as
its Retirement-ADR, is present in `git log`. At the time of writing, G2 is still
in force and no such commit exists. Phases P5, P6 and P7 are blocked.
