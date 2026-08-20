# ADR-2607131500: Scaffold git hygiene, polyglot architecture analysis, and swarm-build retry resilience

**Status:** Accepted
**Date:** 2026-07-13
**Drivers:** Building two downstream projects (a Python CLI, then a TypeScript web UI) via `hex swarm build` surfaced three related trust gaps in hex's own tooling, discovered live in the same working session.
**Supersedes:**
**Superseded-By:**

## Context

Using hex as the development agent for a personal-knowledge-base project (`brain`, Python; `brain/web`, TypeScript) exposed three concrete gaps, each verified against real command output before being classified as a bug rather than a misunderstanding:

1. **No version control on scaffold.** `hex dev new`/`hex init` never `git init` a new project, unlike every example under `hex/examples/` (each its own repo). Worse, `hex swarm build`/`hex swarm review` (`hex-exec/src/adversarial.rs`) never committed their own successful, gate-passing results either — confirmed via `grep -c git hex-exec/src/adversarial.rs` returning zero hits before this change. Two builds in one session required manual `git init && git add -A && git commit` afterward.

2. **Vacuous architecture analysis for non-TS/Rust/Go projects.** `hex analyze` on the Python `brain` project printed a section literally labeled "Hex layers (TypeScript)" and reported **0 source files scanned** with a "Grade A+ 100/100" — not because the architecture was clean, but because nothing was checked. Root-caused precisely: `hex-cli/src/commands/analyze.rs`'s `collect_source_files_recursive` whitelisted only `rs|ts|js|go` extensions (line 966, excluding `.py` entirely), and `hex-cli/src/commands/init.rs`'s `create_adr_rules_toml` wrote **anchored** layer patterns (`"src/domain"`) that never match a real nested package layout (e.g. `src/kb/core/domain/models.py`). Crucially, the underlying matcher — `hex_core::rules::boundary::detect_layer` (`hex-core/src/rules/boundary.rs`) — was **already language-agnostic**, using unanchored substring matching (e.g. `s.contains("/domain/") || s.ends_with("/domain")`). The bug was narrower than "hex doesn't support Python": two callers weren't using the generic matcher they already had.

3. **No resilience in `hex swarm build`'s synthesize step.** A hardcoded 600s timeout with zero retry (`hex-exec/src/adversarial.rs:294`, pre-fix) meant one slow inference call failed the entire build outright — reproduced live: the TypeScript build's first attempt failed with `synthesize failed: claude -p timed out` and an empty (0-char) spec, purely from inference-latency variance (a retry of the identical command succeeded).

## Decision

Fix all three, minimally and without inventing new abstractions where an existing one already worked correctly:

1. **Git hygiene.** `hex-cli/src/commands/init.rs` gained `ensure_git_initialized_and_committed()`, called after scaffold creation (step 6a), producing an initial commit under the commit-local `hex-factory <factory@hex.local>` identity — the same convention already established in `hex-exec/src/direct_exec.rs` (ADR-2606071323 §4: attributable, never masquerading as the operator). `hex-exec/src/adversarial.rs` gained the analogous `commit_result()`, called from `run_build`/`run_review` only after the gate passes. Both are entirely non-fatal (missing git binary, nothing to commit, or an already-initialized repo are silent no-ops) — scaffolding/build success is never contingent on the commit succeeding.

   Left deliberately untouched: `hex-cli/src/pipeline/code_phase.rs`'s existing `ensure_git_isolated()` (used by the *example-generation* pipeline, which `git init`s *before* any files exist — a different timing contract than "commit after files exist"). Reusing it under changed semantics risked altering unrelated pipeline behavior for no benefit; a small, separate implementation in `init.rs` was the lower-risk choice.

2. **Polyglot analysis.** `analyze.rs`: added `"py"` to the extension whitelist; added `pyproject.toml`/`setup.py`/`requirements.txt` as project markers alongside `package.json`/`Cargo.toml`/`go.mod`; replaced the local hardcoded `LAYER_DIRS` table (which used yet a *third*, mutually inconsistent path convention, `core/domain`) with a walk over every file under `src/` classified via the existing `hex_core::rules::boundary::detect_layer` — the same function `enforce.rs` already uses, collapsing two independent hardcoded systems into one. Dropped the hardcoded "(TypeScript)" label since detection no longer implies a language. `init.rs::create_adr_rules_toml`'s written template switched from anchored (`"src/domain"`) to unanchored, trailing-slash-terminated patterns (`"/domain/"`, etc.) matching `boundary.rs`'s own convention exactly — language-independent by construction, so no `--lang` flag or `ProjectLanguage` threading was needed (the already-captured-but-discarded `ProjectLanguage::Python` interview value became moot).

3. **Swarm-build resilience.** Added `claude_run_retry()` in `adversarial.rs` — a bounded-attempt loop (clamped 1-6, no sleep/backoff) mirroring the existing pattern in `direct_exec.rs` (`max_attempts.unwrap_or(3).clamp(1, 6)`), rather than introducing time-based backoff where none of the codebase's conventions use it. `SwarmAction::Build` (`hex-cli/src/commands/swarm/mod.rs`) gained `--timeout` (default 600, unchanged) and `--retries` (default 3) flags, threaded through `run_build`; the build phase's timeout scales `timeout_secs * 4` to preserve today's exact default (600→2400) while remaining proportionally adjustable. `run_review`'s three `claude_run` call sites also moved to `claude_run_retry` for consistency, using a new `DEFAULT_RETRIES` constant, without adding new CLI surface to `hex swarm review` (out of scope for this ADR).

## Consequences

**Positive:**
- Every `hex dev new`/`hex swarm build` project now has real version history from creation, matching `hex/examples/`'s own convention.
- `hex analyze`/`hex enforce check-file` give a real signal for Python (and any future language whose layer directories follow the domain/ports/usecases/adapters convention) instead of a silent, vacuous pass.
- Transient inference-latency timeouts in `hex swarm build` no longer fail an entire multi-minute cooperative build outright.

**Negative:**
- The unanchored substring patterns are looser than a fully-anchored match — a directory literally named e.g. `not-domain-related/domain-cache/` would still register as the `domain` layer. Accepted: this exact tradeoff already existed in `hex_core::rules::boundary` for every consumer before this change; `analyze.rs`/`init.rs` now merely stop disagreeing with it.
- Auto-commit after `swarm build`/`review` means a caller who wanted to inspect an uncommitted diff before it lands must now do so via `git diff HEAD~1` instead of `git diff` — a minor workflow change, not a data-loss risk (the prior state is still a commit, not force-pushed or squashed).
- **Known incomplete generalization, explicitly out of scope here:** a repo-wide grep for anchored `"src/domain"`-style patterns during this ADR's impact analysis found the same convention hardcoded independently in `hex-analyzer`, `hex-analysis`, `hex-nexus/src/research/architecture_analyst.rs`, and `hex-agent`'s validator/MCP server — none of which call the functions changed here, so none are broken by this change, but none benefit from it either. Also unchanged: `hex-cli/src/commands/plan/mod.rs::layers_check` (`hex plan layers`), a third, separate hardcoded `is_ts`/`is_rust` boolean system unrelated to `analyze.rs`/`enforce.rs`.

**Mitigations:**
- The looser matching is bounded by a trailing slash (`"/domain/"`, not bare `"domain"`), verified by a new regression test (`test_unanchored_layer_pattern_matches_nested_package_path`, `enforce.rs`) that specifically asserts `"src/domainxyz/foo.py"` does *not* falsely match.
- Follow-on ADR candidate: extend the same unanchored-pattern convention to `hex-analyzer`/`hex-analysis`/`hex plan layers` if/when those surfaces hit the same Python-support gap in practice.

## Implementation

| Phase | Description | Status | Verification |
|-------|------------|--------|--------------|
| P1 | Git init + commit on scaffold (`init.rs`) and on gate-pass (`adversarial.rs`) | Done | code:hex-cli/src/commands/init.rs, code:hex-exec/src/adversarial.rs, test:live scaffold at /tmp/hex-scaffold-test produced one commit |
| P2 | `.py` extension + `pyproject.toml`/`setup.py`/`requirements.txt` markers + generic layer walk (`analyze.rs`) | Done | code:hex-cli/src/commands/analyze.rs, test:`hex analyze /home/gary/development/brain` — 29 files scanned, 5 layers detected |
| P3 | Unanchored `ADR-rules.toml` template (`init.rs`) | Done | code:hex-cli/src/commands/init.rs, test:hex-cli/src/commands/enforce.rs::test_unanchored_layer_pattern_matches_nested_package_path, test:`hex enforce check-file` against real nested `.py` files |
| P4 | `claude_run_retry` + `--timeout`/`--retries` on `hex swarm build` | Done | code:hex-exec/src/adversarial.rs, code:hex-cli/src/commands/swarm/mod.rs, test:live retry of the TypeScript `web/` build with `--timeout 900 --retries 3` |
| P5 | Full workspace verification | Done | test:`cargo build --release -p hex-cli -p hex-exec -p hex-core`, test:`cargo test -p hex-core -p hex-exec -p hex-cli` (570+ passed; one pre-existing unrelated flaky live-service test, `standalone_gate_smoke`, confirmed via stash-and-rerun to fail identically without this patch) |

## References

- ADR-2606071323 — commit-local factory identity convention (`hex-factory`/`factory@hex.local`), reused here for both new commit sites.
- `hex-core/src/rules/boundary.rs` — the pre-existing, already-language-agnostic layer matcher this ADR routes two more callers through.
