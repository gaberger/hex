# Adversarial review of the collapse diff — P9.4

**Workplan:** `wp-2608241500-hex-solo` P9.4
**ADR:** ADR-2608241500
**Date:** 2026-09-11
**Range:** `c61c8520` (the commit before Phase 0) → `HEAD`
**Lens:** the fourth of four — **silently dropped behaviour**. The other three
(surviving daemon connections, provider names outside `hex-infer`, tests deleted
rather than ported) were checked by hand and recorded in
`2608241500-post-collapse-verdict.md`. This one is the one a hand-check is worst
at, because a hand-check reads what changed and dropped behaviour is defined by
what is no longer there.

---

## Method

Not a reading of the diff. Two executable checks.

**1. The command surface, before and after.** Every `#[derive(Subcommand)]`
enum in `hex-cli/src/`, at both revisions, expanded to its variants:

```
before: 404 subcommands    after: 137    removed: 271    added: 4
```

For each removed module, its daemon coupling at the base commit
(`5555|NexusClient|/api/|HEX_NEXUS|spacetime|stdb`). A module with zero
references was not daemon-mediated, so its removal dropped local behaviour and
needs a reason. Fourteen came back with zero.

**2. Reachability, checked by breaking things.** For every source file: is it
declared as a module anywhere? For every declared module: does anything read it?
Both answers were then confirmed the only way that cannot lie — put invalid Rust
in the file and build the workspace. If the build passes, the file is not
compiled.

That second step matters. The grep-based version of this check produced false
positives twice — once because `grep` exits non-zero on a missing path and I
read that as "not declared", once because `mock.rs` is declared in
`ports/inference.rs` rather than a `mod.rs`. **The grep is a heuristic; the
build is the oracle.** Every finding below was confirmed by the build.

---

## Findings

### 1. Two modules compiled into the binary that nothing can reach

| Module | Lines | Its only reader | Removed by |
|---|---:|---|---|
| `hex-cli/src/prompts.rs` | 257 | `hex context list\|show\|agent\|system\|tools\|services` | P6.2 |
| `hex-cli/src/session.rs` | 630 | `hex dev session start\|resume\|list\|status\|clean\|load` | P5.4 |

Both deletions were correct. Both left their machinery behind, declared
`pub mod` in `main.rs` and `lib.rs`, compiling on every build, callable by
nothing. `hex-cli`'s library target has no consumer inside or outside the
workspace, so `pub` bought nothing.

`prompts.rs` also kept its cargo: **13 prompt templates in
`hex-cli/assets/prompts/`, 82 KB, embedded in the shipped binary by
rust-embed.** Six are org-sim persona prompts (`agent-coder`, `agent-reviewer`,
`agent-tester`, `agent-ux`, `agent-fixer`, `agent-documenter`) for a system
retired by ADR-2606061359. No extractor reaches the `prompts/` prefix —
`hex assets` knows only `skills/` — and no other `Assets::get_str` call names
one.

### 2. Three files on disk that were never compiled at all

| File | Lines | Note |
|---|---:|---|
| `hex-cli/src/commands/readme.rs` | 1,166 | No `mod readme;` in `commands/mod.rs`, at the base commit either. Predates the collapse. |
| `hex-core/src/test_validation.rs` | 17 | **Two `#[test]` functions that have never run.** |
| `hex-cli/src/autonomous_demo.rs` | 26 | A comment sketch of a call chain. |

`hex-cli/src/commands/plan/executor.rs` (494 lines) was the fourth and was
deleted earlier the same day — 494 lines that *simulated* inference, sleeping
50 ms and returning a fabricated HTTP 200 with 256 completion tokens.

`test_validation.rs` is the more interesting one. Two tests for `PathRule`,
sitting in the tree, appearing in no run, failing never. It is the same class as
the 30 `worktree_enforcement` tests the collapse deleted: work that looks like
coverage and is not.

### 3. An embedded asset whose only consumer is a test that it is embedded

`hex-cli/assets/schemas/mcp-tools.json`, 17 KB. The `hex mcp` verb it described
was deleted in P6.1. The only code that reads it is
`assets::tests::mcp_tools_is_embedded`, which asserts the file is present.

A test that guards an asset nothing uses passes forever and proves nothing about
shipped behaviour. It is a gate around an empty room.

### 4. `hex adr review` — the attached obligation, checked

P6.1 authorised deleting `adr_review` (991 lines) with a condition: **"merge its
linting into `adr/`"**. The merge is not recorded anywhere. Checked directly:

| `hex adr review` (deleted) | `hex adr doctor` (present) |
|---|---|
| `duplicate_numbering` | `DuplicateId` ✓ |
| `metadata_validation` | `MissingRequiredField`, `UnparseableStatus` ✓ |
| `supersession_chain` | `SupersededUnlinked` ✓ |
| `stale_reference` | `DanglingDependency` ✓ |
| `scope_conflict` | **no equivalent** |

Four of five survived. The fifth flagged pairs of ADRs sharing more than eight
domain keywords. Its own source records it being tuned down twice — threshold
raised from 3 to 8, severity lowered from Warning to Info with the comment
"every pair of ADRs in the same problem domain will share keywords by design".
An advisory heuristic that had already been argued down to advisory. Recorded as
a known gap, not a regression.

### 5. Three verbs P6.1 listed for deletion that were kept — correctly

Recorded here because P9.4's lens cuts both ways. `interview`, `insight` and
`refresh` appear in P6.1's deletion list under "daemon-mediated verbs". None has
a single daemon reference. They were kept. The plan was wrong; the execution was
right. This is already noted in the workplan task itself.

---

## Action taken

Deleted: 2,096 lines of Rust and 90 KB of embedded assets.

```
hex-cli/src/prompts.rs                 257    compiled, unreachable
hex-cli/src/session.rs                 630    compiled, unreachable
hex-cli/src/commands/readme.rs       1,166    never compiled
hex-cli/src/autonomous_demo.rs          26    never compiled
hex-core/src/test_validation.rs         17    never compiled (2 dead tests)
hex-cli/assets/prompts/            13 files   embedded, unreachable
hex-cli/assets/schemas/mcp-tools.json         embedded, test-only consumer
```

Consumers traced first, across both blind spots the collapse recorded: a
re-export at a crate root (`lib.rs` has no `pub use`) and a `#[cfg(feature)]`
import (none). No integration test referenced any of them.

**43 test functions went with them**, and the exact list was taken rather than
estimated — `cargo test --workspace -- --list` before and after:

```
 24  prompts::tests           (12 tests × lib and bin targets)
 16  session::tests           ( 8 tests × lib and bin targets)
  2  assets::tests            (mcp_tools_is_embedded, × 2)
  1  doc-test on PromptTemplate
```

Every one belonged to a module nothing could reach. **No test of reachable code
was lost.** That claim is checkable: the `comm -23` of the two lists is the four
lines above and nothing else.

**After: 948 tests, 0 failures. `hex analyze .`: A+ 100/100, 0 boundary
violations.**

---

## Verdict

**The collapse dropped no behaviour that anyone can still call.** Every one of
the 271 removed subcommands either had daemon coupling at the base commit or is
accounted for above. The deletions were sound.

What it left behind is residue: five files and two asset bundles that survived
because nothing failed when they stopped being used. A module with no reader
compiles clean. An embedded asset with no extractor ships quietly. A test file
with no `mod` declaration runs zero tests and reports nothing.

That is the honest shape of the finding. The lens was "what was silently
dropped", and the answer is: **nothing was dropped silently, but several things
were silently *kept*.** Dead weight is the mirror image of the failure P9.4 was
looking for, and it is invisible for the same reason — no gate fails when code
stops being reachable.

The generalisable check is the one used here, and it is three lines:

```
# Is this file compiled?      Put invalid Rust in it and build the workspace.
# Does anything read it?      grep for `<module>::` outside its own file.
# Can an asset be reached?    grep every Assets::get for its prefix.
```

The first is the only one that cannot be fooled. The other two were each wrong
once during this review.

## What this review did not cover

- **Behaviour dropped *inside* a surviving verb.** The surface diff sees a verb
  that disappeared; it cannot see a flag, a branch, or an error path that
  quietly stopped existing inside a verb that remained. That needs a per-verb
  behavioural diff, and no such record was kept before the collapse.
- **The 63 open ADR-rule findings** on hex itself, listed in the post-collapse
  verdict. They are a backlog, not part of this review.
