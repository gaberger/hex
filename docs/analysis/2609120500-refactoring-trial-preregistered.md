# Refactoring trial: pre-registered measures

Written before the run. Not edited after.

## Subject
`brain`, a hexagonal project hex did not write. Its web client at
`web/src/adapters/primary/web-client/src/` has 17 boundary violations, all
rule 4: components import `domain/` directly. The client has no `ports/`.

## Baseline
- tsc: green
- vitest: 177 of 177
- `hex analyze .`: F, score 0, 17 boundary violations

## The task given to hex
The rule and the count, not the file list. Localisation is being measured.

## The gate
tsc green AND vitest green AND `hex analyze . --exit-code` zero.
The gate is red before the run because of the third clause only.

## Verb
`hex build`. `hex do` takes one file and this change needs a new `ports/`
directory plus edits to six components. No verb exists for "refactor to a
grade". That is finding one, recorded before the run.

## Measures, in order of weight
1. Diff scope. Files touched. Whether any file under `domain/` changed. A
   refactor that rewrites domain logic to satisfy the grade is a failure
   regardless of the other three.
2. Localisation. Whether every one of the 17 violations was found without
   the list.
3. Suite. 177 of 177 after, and tsc green.
4. Grade. A or better after.

## What counts as a pass
All four. Anything less is reported as what it is.
