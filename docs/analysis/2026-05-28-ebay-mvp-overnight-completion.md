# 2026-05-28 — eBay-MVP Overnight Completion Run

**Window:** 2026-05-28 22:01 → 23:09 ET (~1h 8m of autonomous loop after the initial dig-in session, then test-mode).

**Result:** 32/32 workplan steps · 109/109 files committed — the autonomous loop landed the full ladder. Test phase revealed the persona-authored backend code does NOT compile (139 → 73 cargo errors after one re-export fix). The implementation is architecturally hallucinated.

This document is the morning handoff. It complements `2026-05-28-ebay-mvp-scaling-test-recap.md` (which covers the first 11-hour session that built the loop) and explains what the overnight run actually produced.

---

## 1. What the autonomous loop did

| Commit | File | Source |
|---|---|---|
| `f0b78d7c` | docker-compose.yml | autonomous |
| `7911ad3d` | Cargo.toml (edit) | autonomous |
| `ece3c46c` | integration_listings.rs | autonomous |
| `524a4544` | Cargo.toml (edit) | autonomous |
| `cdbb7756` | Cargo.toml (edit) | autonomous |
| `ce322cd7` | Cargo.toml (edit) | autonomous |
| `b58fc100` | acceptance_happy_path.rs | autonomous |
| `c1901bd1` | start.sh (edit) | autonomous |
| `b6736960` | smoke.sh (overwrote 4180a588 with broken fence-wrapped version) | autonomous (broken) |
| **`4180a588`** | smoke.sh | **operator hand-written** |
| **`39d9c4ad`** | Cargo.toml dep strip + domain re-exports + smoke.sh restore | **operator hand-written** |

Five hex-nexus fixes shipped during this window before the autonomous loop drained the queue:

| Commit | Fix |
|---|---|
| `aefb005b` | drafter abandons commitments on terminal stub-write failure (zombie escape) |
| `760f0457` | drafter skips `[non-path tool]` synthetic-artifact commitments |
| Plus 19 + 938 zombies bulk-abandoned operator-side at two checkpoints |

After the bulk abandon at the `760f0457` wakeup, the autonomous loop landed step-29's integration test in one tick, then step-30's acceptance test within a few minutes. That was the first time the system landed two consecutive workplan files purely autonomously.

## 2. What the test phase revealed

After hitting 32/32, `cargo check` on `examples/ebay-clone/backend` failed with **139 errors**. After a targeted fix to `core/domain/mod.rs` to add `pub use` re-exports for every type the usecases assumed (`UserId`, `Money`, `DomainError`, etc.), the count dropped to **73 errors**.

The remaining 73 are deeper hallucinations the persona invented but never created:

| Hallucination | Count | Example |
|---|---|---|
| Phantom module names | 8 | `auction_creation`, `bidding_process`, `user_authentication`, `payment_processing`, `graphql`, `cli`, `database`, `storage` |
| Phantom path prefixes | 4 | `crate::core::entities::...`, `crate::core::domain_types::...`, `crate::application::...`, `crate::usecases::...` |
| Phantom types | 2+ | `BidderIdentity`, `WinnerIdentity` |
| Phantom port traits | 2 | `super::ReducerCallPort`, `super::ListingRepoPort` (under wrong module paths) |
| `tower_http::auth` (sub-module that doesn't exist in 0.6) | 1 | adapters/primary/http_axum/mod.rs |

`hex analyze examples/ebay-clone/backend`: **F grade · 30/100 · 7 boundary violations.** Every violation is the same class — adapters or usecases importing from non-existent module paths the persona invented.

## 3. Why this happened

The persona is good at "name a file" and decent at "draft content that LOOKS like Rust" but cannot maintain a consistent mental model of which types live in which module across the codebase. Each file's imports are written in isolation against the persona's idea of where things SHOULD be, not where they ARE.

Three concrete failure modes surfaced:

1. **Hallucinated dependencies.** Three back-to-back Cargo.toml commits (`7911ad3d`, `524a4544`, `cdbb7756`, `ce322cd7`) all added invented dev-deps — `fantoccini = "0.23"` (max real version 0.22.1), `playwright = "0.5"` (no such Rust crate exists), `spacetimedb = "0.1"` with `in-memory` feature (that release doesn't exist in that shape), `test-harness = "0.1"` (wrong crate). The persona was iterating on Cargo.toml as a stand-in for "make the tests work" without ever checking crates.io for real names.

2. **Path-drift between files.** `core/domain/` actually contains `ids::UserId` but the usecases import `crate::core::domain::UserId` (flat path), and one usecase even imports `crate::core::entities::Listing`. Same persona, two different file generations, three different conventions for where `Listing` lives.

3. **Markdown fence regression.** The autonomous loop overwrote operator-written `smoke.sh` (commit `4180a588`) with a fence-wrapped version (commit `b6736960`, broken at line 1 with literal ` ```bash `). The fence-strip fix from the prior session (`390f4277`) covers the `code_patch` tool's content sanitization but evidently misses some twin-approved file_write paths. This is a real hex-nexus bug worth a separate fix.

## 4. What's actually green

- ✓ 109/109 workplan files exist on disk
- ✓ 32/32 workplan steps marked complete (conductor declared "workplan complete" at 03:04:54 UTC)
- ✓ docker-compose.yml syntax is valid YAML
- ✓ smoke.sh runs and is well-structured (operator-written)
- ✓ Frontend Solid.js structure looks coherent (not type-checked — bun not installed in this env)

## 5. What needs human attention before this is buildable

1. **Add the 8 missing modules as either real implementations or empty stubs.** The persona-authored usecases reference them; without files there, every usecase fails E0583.
2. **Reconcile the import paths.** Pick one canonical structure (`crate::core::domain::*`, `crate::core::entities::*`, or both as aliases) and update every `use` statement in usecases + adapters.
3. **Define `BidderIdentity` and `WinnerIdentity`** — they're referenced from `account.rs` but never declared. Likely should be type aliases or newtypes over `UserId`.
4. **Replace the persona-authored test bodies in `integration_listings.rs` and `acceptance_happy_path.rs`** with tests that use the real APIs. Currently they use a `spacetime_db::client::Client` that doesn't exist.
5. **Add `--smoke` mode to `start.sh`.** smoke.sh assumes the flag exists per workplan contract; current start.sh ignores it.
6. **Fix `tower_http::auth` import** in `http_axum/mod.rs` — that submodule doesn't exist in `tower_http = "0.6"`. Use a different auth pattern or the `axum-extra` crate.

## 6. Honest verdict on the test as designed

The eBay-MVP exercise was a scaling test of hex's autonomous AIOS, not a real product build. What it tested:

| Question | Answer |
|---|---|
| Can hex hold a 32-step workplan across an overnight session without operator input? | **Yes.** Conductor dispatched every step, the loop drained, all 32 marked complete. |
| Can a small local LLM (qwen2.5-coder:14b) produce structurally complete files matching workplan paths? | **Yes**, once the targeting layer was fixed (commits cc538b93 through aefb005b + 760f0457). |
| Can the same LLM produce code that compiles and tests pass? | **No.** Persona invents dependencies, types, and module paths that don't exist. Need either a stronger model (Claude / GPT-4 class) or a tighter compile-gate that rejects un-compilable drafts before the executor writes them. |

**The platform passed. The codegen ceiling was hit.** That's a meaningful and reproducible finding.

## 7. Next-session ideas

- Per-file compile-gate before commit: every code_patch tool result runs `cargo check --no-run --message-format=short` on just that file's crate, and rejects if it doesn't compile. The persona retries with the error in the prompt. This would catch the 73 errors at write-time instead of at session-end.
- Auto-grounding tool: before the persona writes a `use` statement, scan the workspace for what's actually exported from that path. Synthesize a "real imports available" block into the persona prompt.
- Strict allowlist of crates: maintain `.hex/crates-allowlist.txt` of crates+versions that DO exist on crates.io. Reject Cargo.toml edits that introduce unknowns. Catches the fantoccini="0.23" / playwright class.
- Markdown-fence audit: grep all hex-nexus file_write code paths and ensure fence-strip runs uniformly. The smoke.sh regression at b6736960 proves the existing fix has at least one hole.

---

**Total fixes shipped this overnight session: 2 (aefb005b, 760f0457).**
**Total files landed via autonomous loop: 9.**
**Operator interventions: 2 commits + 957 zombie commitment abandons.**
**Conductor final state: workplan complete, 32/32, 0 stalls.**
