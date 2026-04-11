# README drift baseline (2026-04-11)

**Context**: user asked "how can we validate that everything in README.md is true"
on branch `claude/review-codebase-7qKsD`. This document captures the drift that
existed at baseline, the validation tool built to catch it, and the policy for
keeping the README honest going forward.

## Baseline drift

On 2026-04-11, running the new `hex readme validate` command against the
pre-fix README produced **5 errors and 1 warning**:

| Category | README claimed | Actual | Severity | Fix |
|---|---|---|---|---|
| ADR badge | `107_Accepted` | 145 | err | Updated badge to `145_Accepted` |
| ADR body | "107 Architecture Decision Records" | 145 | err | Updated inline count |
| Agent definitions table | "14 agent definitions" | 18 | err | Updated |
| Agent Roles prose | "14 specialized agent definitions" | 18 | err | Updated |
| Port traits (file tree) | "9 port traits" | 10 | err | Updated |
| Port traits (crate table) | "9 port traits" | 10 | err | Same edit |
| Skills | "21+ skills" | 20 | warn | Changed to "20 skills" + deleted stale duplicate |

### What stayed accurate

The README's structural claims were all correct:

- **7 WASM modules** ✓ (exact match)
- **6 workspace crates** ✓
- **All 5 SVG assets** under `.github/assets/` exist (`banner`, `comparison`,
  `architecture`, `swarm`, `workflow`)
- **All 6 internal documentation links** resolve (`docs/adrs/`, `docs/guides/`,
  `docs/specs/`, `docs/workplans/`, `docs/analysis/`, `LICENSE`)
- **All 7 WASM module names** in the table exist as directories under
  `spacetime-modules/`
- **All 6 crate names** referenced in the System Architecture table exist
  as directories under the repo root
- **All 8 agent names** referenced in the Agent Roles table (`hex-coder`,
  `planner`, `integrator`, `swarm-coordinator`, `validation-judge`,
  `behavioral-spec-writer`, `adversarial-reviewer`, `rust-refactorer`) exist
  as YAML files under `hex-cli/assets/agents/hex/hex/`
- **All 17 `hex <cmd>` invocations** in bash code blocks run successfully with
  `--help` — no broken or renamed commands

The drift was concentrated in **counts that grow over time** (ADRs, agents,
skills, port traits). This is the expected failure mode: humans add new artifacts
but forget to update the tallies in the README. The same entity could be claimed
in two places (badge + prose) and both would need to be kept in sync by hand —
exactly the pattern that guarantees silent drift.

## The validator

`hex readme validate` checks the README against the filesystem across six bands:

1. **Numeric counts** — extracts `(digits)` immediately before a target phrase
   ("Architecture Decision Records", "agent definitions", "port traits",
   "reducers", "WASM modules", "_Accepted"), counts the corresponding filesystem
   artifact, and errors on mismatch. `~` prefixes like "~130 reducers" allow a
   tolerance of ±20.
2. **SVG asset existence** — for each of the 5 referenced images, verify the
   file exists at the claimed path.
3. **Internal link resolution** — parse every `[text](path)` markdown link
   where `path` is local (not `http://`, `https://`, `#`, or `mailto:`), and
   verify the path exists.
4. **Module name references** — each of the 7 WASM module names referenced in
   the SpacetimeDB Microkernel table must exist as a directory with a
   `Cargo.toml`.
5. **Crate / agent name references** — each of the 6 crate names and the 8
   agent names referenced in the System Architecture and Agent Roles tables
   must exist as directories or YAML files.
6. **CLI command existence** — parse every `hex <subcmd>` invocation inside
   fenced ```` ```bash ```` blocks, shell out to `hex <subcmd> --help`, and
   assert exit code 0.

Implementation: `hex-cli/src/commands/readme.rs::validate_readme`. Pure stdlib,
no regex dep.

## How to run it

```bash
hex readme validate                  # report drift, exit 0 unless errors
hex readme validate --strict         # exit 1 on warnings too (CI mode)
./scripts/validate-readme.sh         # wrapper that builds the binary if needed
cargo test -p hex-cli commands::readme::tests::repo_readme_is_accurate
                                      # unit test guard for CI (gates PRs)
```

## CI enforcement

- **`.github/workflows/ci.yml` → `boundary-check` job** runs
  `./target/release/hex readme validate --strict` after the release binary is
  built. Any drift blocks the PR.
- **`rust-check` job** runs `cargo test --workspace --exclude hex-desktop`,
  which transitively runs `repo_readme_is_accurate`. Same gate, different path.

Either job failing is enough to block a merge. The two independent paths exist
because the Rust unit test gives a tight inner-loop signal (`cargo test`
locally) while the CLI invocation in boundary-check exercises the real
packaged binary — if `hex readme validate` itself breaks, the unit test wouldn't
catch that.

## Policy

When you add a new ADR, agent, skill, WASM module, or port trait:

1. Add the artifact (new file, new trait, new module).
2. Run `./scripts/validate-readme.sh` locally — it will tell you exactly which
   lines of the README claim a stale count.
3. Update the README to match. Usually this is one or two places (e.g. the
   badge plus the inline prose count).
4. Commit both the artifact and the README update in the same commit.

Do not try to keep the README "ahead" of reality by inflating counts — the
validator will fail and the CI will block your merge. Always match reality.

## Claims the validator does NOT check

The following claims still require human review or out-of-band benchmarks:

- **Quantitative performance claims** — "<1ms coordination", "90% cost
  reduction", "~200ms to <1ms latency drop", "~200 tokens for full adapter
  context". These need a reproducible benchmark harness. See the README's
  Layer 4 section for strategy.
- **Comparative claims** about BAML / SpecKit / HUD / LangChain / CrewAI /
  AutoGen / Claude Agent SDK. These are opinionated and change over time.
- **Prose architectural claims** like "enforcement at kernel level",
  "stateless and horizontally scalable". These need architectural review.
- **External links** (`https://...`) — not checked because they rot
  asynchronously and break CI for reasons unrelated to this repo.

Claims in these categories should be reviewed manually at release time.
If a claim becomes machine-verifiable in the future (e.g., you add a
benchmark script that measures coordination latency), extend
`validate_readme()` with a new check band.

## References

- `hex-cli/src/commands/readme.rs::validate_readme` — the implementation
- `hex-cli/src/commands/readme.rs::tests::repo_readme_is_accurate` — the CI
  unit test guard
- `scripts/validate-readme.sh` — the interactive wrapper
- `.github/workflows/ci.yml` → `boundary-check` — the CI invocation
- ADR-2604110227 — the ADR that introduced this whole line of work
