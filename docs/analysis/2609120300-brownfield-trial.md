# Brownfield: can hex change code it did not write?

**Date:** 2026-09-12
**Subject:** [`weave`](https://git.local/gary/weave) — 1,685 files, 549 TypeScript
sources, 495 test files, 1,313 commits. Written by a different project entirely.
**Method:** mutation repair, in a throwaway clone. Criteria fixed before the run.

Every project hex had built was greenfield. The evidence gate protects a change;
nothing had tested whether hex can find and make the right change in unfamiliar
code. This is that test.

## Setup

1. Clone, `npm install`, run the suite. **Baseline was not green** — 6–7 failures,
   all in one socket suite, environmental and flaky. Recorded, and per-test gates
   used instead of a whole-suite gate, which isolates each trial anyway.
2. Inject **10 realistic single-token bugs** into 10 different source files:
   `>=` → `>`, `!==` → `===`, `??` → `||`. Each was **verified to break at least
   one existing test** before being accepted — a mutation that breaks nothing
   cannot test repair.
3. Commit the injected state so HEAD carries the bugs.

No contamination risk: the bugs are mine, so no model has seen them.

## Part A — localisation: **1/10**

Given only the failing test's name, can hex find the file?

| Measure | Result |
|---|---:|
| Truth file ranked 1st | **0/10** |
| Truth file in the top 3 | **0/10** |
| Truth file anywhere in the results | **1/10** |

`hex graph query` built a 5,558-node graph of the repository and then returned
`src/cli.ts` — the hub node — or ADR prose for nearly every query. Ranking by
graph centrality surfaces what everything touches, which is the opposite of what
a bug report needs.

And structurally: **`hex do run` requires `--file`.** No verb accepts "this test
fails, find why". The gap is not a tuning problem; the capability does not exist.

## Part B — repair, given the file: **10/10**

```
gate passed:              10/10
exact original restored:  10/10
test files edited:         0/10
```

Every repair restored the original line exactly. Not one run passed its gate by
editing the test — the failure mode most worth fearing, and the one `--file`
structurally prevents.

The full suite afterwards: **2,141 of 2,160 passing**, with the only failures the
same flaky socket suite present before any injection. No repair broke anything
outside its own file.

The resource governor routed every task to the frontier path (`devstral` would not
fit in available memory), so this measures hex's full capability, not the local
ceiling.

### The honest size of that number

This is **repair given localisation**, on single-token bugs, with the failing test
named in the prompt. A frontier model reading a 130-line file, told which test
fails, will find a `>` that should be `>=`. 10/10 is the expected result, and it
is the *easier* half of the problem — Part A is the hard half and it failed.

Untested: multi-file changes, bugs requiring intent to be understood across
modules, and anything where the failing test does not name the concept.

## What the trial found in hex

**A correct, gate-passing repair was deleted.** On the first run — a fresh clone
with no `git config user.email`, which is the default state of any clone or CI
box — the sequence was:

1. hex made the correct fix.
2. Evidence passed: 9 tests, 0 failures.
3. `git commit` failed: *"unable to auto-detect email address"*.
4. hex **reverted the fix** and reported **"direct run did not pass evidence"**.

Evidence passed. The commit failed. Three defects in one path:

| Defect | Consequence |
|---|---|
| Commit failure returned as `ApplyFailed("commit: …")` | wrong category, so the caller treated correct work as a bad edit |
| The change was reverted | the only valuable output of the run, destroyed |
| CLI always printed "did not pass evidence" | blamed the tests for a git problem |

It also left the repository in a corrupt state: the fix staged in the index, the
bug back in the working tree (`MM`).

All three commit paths — ReAct, single-shot, and the `claude -p` delegate — had
the same shape. Two of the three reverted.

**Fixed.** `EditOutcome::CommitFailed` is now its own variant; the change is kept
and unstaged; the message names which half failed and, for the identity case,
what to run. Verified by reproducing the original conditions with the fixed
binary: fix kept, index clean, test green, message correct.

This is the fifth instance today of one defect class: **a gate reporting a verdict
for a reason unrelated to what it gates.** The others were `hex ci` pointing at a
deleted crate, `hex analyze` printing a green tick for a skipped check, the
vacuous guard rejecting a 39-test pass, and `create_scaffold` reporting "no assets
embedded" when every file already existed.

## Verdict

**Repair in unfamiliar code: demonstrated**, within the stated limits.
**Localisation: absent.** hex cannot yet be handed a failing test and asked to
find the cause — and that, not repair, is what "point it at a codebase" means.

The next thing to build is a verb that takes a failing command and produces a
ranked list of candidate files, gated by actually repairing one of them.
