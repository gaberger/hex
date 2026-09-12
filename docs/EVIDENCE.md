# Evidence

Every claim on the README maps to a command here. Each command was run before
this page was written, and the expected output is what it printed. A claim with
no command on this page is not yet validated.

## Setup

```bash
cargo build --workspace
```

The hermetic reproducers need nothing else. The live ones need a built `hex` on
`PATH` and, where noted, a toolchain for the example's language.

## The scaffold is deterministic and runs

**Claim.** `hex init --scaffold` writes byte-identical output for the same name,
and the result passes its gate with no edits, in Rust, Go and TypeScript.

```bash
cargo test -p hex-cli --test scaffold_is_executable
```

Expected: `8 passed`. The tests scaffold twice and diff the bytes, run each
language's gate, plant a violation and assert each shipped rule fires, and
assert a clean scaffold trips none of them.

## The rules that ship with a scaffold fire

**Claim.** `.hex/ADR-rules.toml` is enforced by `hex analyze`, not advisory.

```bash
hex init /tmp/probe --scaffold --lang rust
printf 'pub fn bad() { let _p = "/home/x"; let _m = "qwen3:4b"; }\n' > /tmp/probe/src/offender.rs
hex analyze /tmp/probe
```

Expected: two violations named with file and line. `Hardcoded absolute path` and
`A model or provider name outside the inference boundary`.

## A vacuous pass is rejected

**Claim.** A test command that exits 0 having run zero tests does not satisfy
the gate.

```bash
cargo test -p hex-exec --lib vacuous
```

Expected: `5 passed`. One test is the regression that motivated the counter: a
multi-binary cargo run with one empty binary and 39 tests elsewhere is not
vacuous. The old substring check called it vacuous and would have reverted a
correct commit.

## The architecture grade cannot rise as violations are added

**Claim.** The health score is monotonic in the violation count.

```bash
cargo test -p hex-analysis --lib health_score
```

Expected: `8 passed`, including `the_score_never_climbs_as_violations_are_added`
over 0 to 60 violations. Before the fix, 26 violations scored 96 because the
penalty wrapped in a `u8`.

## hex passes its own analyzer

```bash
hex analyze .
```

Expected: `Architecture grade: A+ — score 100/100`, `0 boundary violations`.

## A commit failure does not delete the work

**Claim.** When the gate passes and `git commit` fails, the change is kept in
the working tree and the error names the failed step.

```bash
cargo test -p hex-exec --lib commit_failure
```

Expected: `2 passed`. Before the fix, a fresh clone with no git identity
reverted every correct repair and reported it as a failed test.

## The shipped documents name only real verbs

**Claim.** Every `hex …` command in the README, in `ARCHITECTURE.md`, in every
document under `docs/`, and in the `CLAUDE.md` section written into scaffolded
projects resolves against the binary. Every relative link resolves. Every
diagram image exists.

```bash
cargo test -p hex-cli --test shipped_docs_name_real_verbs
```

Expected: `9 passed`. Planting a dead verb or a dead link fails it.

## A workplan task is done only with evidence

**Claim.** A task marked done without commits in its scope is not accepted.

```bash
cargo test -p hex-cli --test reconcile_evidence
```

Expected: `3 passed`.

## The provider is named in one place

**Claim.** The local inference server's identity lives in `hex-infer` only.

```bash
cargo test -p hex-infer --lib local_provider
```

Expected: `5 passed`, including that an unconfigured project yields no tiers
rather than a default model, and that `OLLAMA_HOST` normalises whether given as
`host:port` or a URL.

## The examples pass their gates

Each was built by `hex scaffold` or `hex build` from one sentence.

```bash
(cd examples/linkstore-svc    && cargo test)   # 27, of which 12 end-to-end
(cd examples/url-shortener-rs && cargo test)   # 39
(cd examples/game-life-rs     && cargo test)   # 36
(cd examples/ratelimiter-proof && cargo test)  # 18
(cd examples/game-ttt-go      && go test ./...)
(cd examples/game-2048-ts     && npm install && npm test)   # 88
```

`linkstore-svc` binds a real port and writes a real SQLite file. Breaking its
HTTP status fails 4 tests. Breaking a domain rule fails 3 more.

## The standalone gate

```bash
hex ci --standalone-gate
```

Expected: three `pass` lines and `Standalone gate passed`. It runs the
inference adapters' tests and the agent loop's tests. Before the fix it ran
tests in a deleted crate and could never pass.

## Not yet validated

- **Localisation.** Given only a failing test name, hex found the right file in
  1 of 10 trials. There is no command for it. The trial is in
  [`analysis/2609120300-brownfield-trial.md`](analysis/2609120300-brownfield-trial.md).
- **Third-party imports in `domain/`.** The analyzer does not check them. A
  project can import a runtime into its domain and score A+.
- **Repair beyond single-token bugs.** The brownfield trial injected one wrong
  token per file. Multi-file changes are untested.
