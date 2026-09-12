# Refactoring trial: can hex change the shape of code it did not write?

**Date:** 2026-09-12
**Subject:** `brain`, a hexagonal project hex did not write. Its web client at
`web/src/adapters/primary/web-client/src/` had 17 boundary violations, all rule
4: components importing `domain/` directly. The client had no `ports/`.
**Measures:** pre-registered in [`2609120500-refactoring-trial-preregistered.md`](2609120500-refactoring-trial-preregistered.md) before the run, in this
order: diff scope, localisation, suite, grade.
**Verb:** `hex build`. No verb exists for "refactor to a grade", and `hex do`
takes one file. That was finding one, on record before the run.

## Baseline

tsc green. vitest 177 of 177. `hex analyze .`: F, score 0, 17 violations.

## The task

The rule and the count. Not the file list, and not the fix. Constraints: do
not modify `domain/`, do not change behaviour, do not edit or delete a test.

## Result

`hex build` reported GREEN after five commits.

| Measure | Registered as | Result |
|---|---|---|
| 1. Diff scope | no `domain/` change, no test edit | 3 `domain/` files changed; 0 tests edited, 3 added |
| 2. Localisation | all 17 found without the list | **17 of 17**, plus 5 more outside the target |
| 3. Suite | 177 of 177, tsc green | **177 of 177**, tsc green |
| 4. Grade | A or better | **C, score 78** |

Two pass. Two fail on the letter. Both need reading.

## Measure 1, read

The three `domain/` changes are two one-line import redirects and one new
types file. No domain logic changed. The two redirected lines had been
importing from `api/client.ts`, which is an adapter. That is a rule 1
violation, and the analyzer does not flag it, because it checks layer-to-layer
edges by directory name and `api/` is not a layer it knows. hex found it,
declared the wire types inside `domain/`, and cut the edge.

The constraint was broken. Breaking it was correct. The analyzer's blind spot
is already recorded in the real-I/O proof and this is the second time it has
surfaced.

The three test files are additions, not edits. One is a port contract test.
One is `portsBoundary.test.ts`, four cases that assert rule 4 over the
directory by reading the filesystem. The agent wrote a gate for the rule it
was enforcing, in the project's own test runner, without being asked. That is
this project's "rules travel with the project" applied by the model on its
own.

Five files changed outside `--target`, in the outer app's `ports/` and
`adapters/secondary/`. They are the same fix, rule 5, applied to violations
the target-scoped task did not name. The gate runs `hex analyze .` from the
repository root, so the gate forced them. That is the gate working.

## Measure 4, read

The grade is C at 78 with zero boundary violations. The first version of this
section attributed the 22 points to the dead-layers detector. That was an
inference, written as a fact, and it was wrong. `--json` did not expose the
score's components; it does now, and they are:

| Component | Baseline | After | Points |
|---|---:|---:|---:|
| violations | 17 | 0 | 0 |
| dead exports | 21 | 18 | 18 |
| unused ports | 0 | 4 | 4 |
| circular deps | 0 | 0 | 0 |

**The four unused ports are the four the refactor created.** `GraphInsightPort`,
`LinkResolutionPort`, `NoteTreePort`, `EditorTextPort`. The detector looks for
an adapter importing the interface by name. The components import the frozen
value `graphInsightPort`, which is the correct TypeScript pattern, and the
detector does not follow a value to its type. A correct fix was penalised four
points by a name heuristic. That is the detector defect, and it is the one this
section originally described against the wrong detector.

**The 18 dead exports are in the outer app**, under `web/src/domain` and
`web/src/usecases`, not in the client. They were 21 at baseline, masked by the
170-point violation penalty. The refactor reduced them by three. They are
pre-existing debt the grade now shows because nothing larger hides it.

With that detector fixed, the score is 82, a B. Not A, because of debt outside
the target. The task did not ask for that and the gate did not demand it.

**The dead-layers count of 10 is unrelated to the score.** That detector reads
only `.rs` files and parses with the Rust grammar. On a TypeScript tree it sees
no inbound edges anywhere and flags every layer directory. It went from 9 to 10
because one directory was added. It is displayed and it is meaningless on any
project that is not Rust.

The second half of this failure is mine. The gate given to `hex build` was
`hex analyze . --exit-code`, which passes at zero violations. The measure I
registered was grade A. The gate was weaker than the measure, so the harness
reported GREEN at C. A trial's gate must be its measure.

## The refactor itself

`ports/graphInsight.ts`, in full:

```ts
import { backlinksFor } from '../domain/backlinksFor';
import { localSubgraph } from '../domain/localSubgraph';

export type { BacklinkEntry } from '../domain/backlinksFor';

export interface GraphInsightPort {
  readonly backlinksFor: typeof backlinksFor;
  readonly localSubgraph: typeof localSubgraph;
}

export const graphInsightPort: GraphInsightPort = Object.freeze({
  backlinksFor: (...a) => backlinksFor(...a),
  localSubgraph: (...a) => localSubgraph(...a),
});
```

A typed interface, a frozen object, forwarding functions, and the domain type
re-exported through the port. That is the mechanism the README describes for
rule 4, produced from the rule alone. `BacklinksPanel.tsx` changed by two
lines: the import, and one destructure.

## Verdict

**hex refactored code it did not write, correctly, and found every violation
without being told where they were.** The shape it produced is the one the
rules ask for. It edited no logic and no test, and it wrote a boundary test of
its own.

On the pre-registered letter it fails twice. One failure is the analyzer
missing a real violation that hex then fixed. The other is a health detector
penalising the fix, compounded by my giving the build a weaker gate than the
measure.

## Recorded, not fixed

- **Fixed since.** `unused_ports` now treats an import from a port's module
  as use of its ports, which is what use looks like in TypeScript, and the Go
  extractor now records the qualified names a file actually references, so a
  Go port nothing names is reportable. Six fixtures, two per language. The
  `brain` clone re-analysed: 0 unused ports, score 82, B. ADR-2609120600.
- **Guarded, not fixed.** `dead layers` is Rust-only. It now declines on a
  tree with no `.rs` files and the display says `n/a` with the reason, instead
  of a count. The rebuild on the shared file model is ADR-2609120600 step 3.
- The analyzer does not see `domain/` importing from a non-layer directory
  such as `api/`. Second sighting.
- `hex build --gate` accepted a gate weaker than the trial's measure, and
  nothing warned that the grade was below the floor at GREEN.
- There is still no verb for "refactor to a grade". The gate drove this one,
  which shows the mechanism works. The verb would make it repeatable.
