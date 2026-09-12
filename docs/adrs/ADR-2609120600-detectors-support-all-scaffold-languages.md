# ADR-2609120600: Every detector supports every scaffold language, with a fixture per language

**Status:** Proposed
**Date:** 2026-09-12
**Epoch:** solo
**Drivers:** Operator directive: "we need to support 3 languages" and "make sure we have full testing." Backed by an audit taken during the refactoring trial (`docs/analysis/2609120500-refactoring-trial.md`).
**Relates-To:** ADR-2608241500 (the solo collapse), ADR-2026-09-11-1900 (gate-first development)

## Context

`hex init --scaffold` and `hex scaffold` produce Rust, Go and TypeScript. The
README says so. The architecture grade is the second gate, and it is computed
by detectors in `hex-analysis`.

An audit of those detectors on 2026-09-12:

| Detector | Feeds the score | Rust | Go | TypeScript |
|---|---|---|---|---|
| boundary analysis (tree-sitter) | yes | yes | yes | yes |
| `dead_exports` | yes | yes | ? | reads TS exports; correctness unverified |
| `unused_ports` | yes | yes | structural | name-match only; missed the idiomatic value-export pattern |
| `circular_deps` | yes | yes | mentioned | not mentioned |
| `dead_layer` | display | yes | **no** | **no**: parses with the Rust grammar and skips every non-`.rs` file |
| `cohesion` | display | yes | no | no |
| `duplication` | display | yes | no | no |
| `god_types` | display | yes | no | no |
| `orphan` | display | yes | no | no |

No detector test writes a `.ts` or `.go` fixture. All six test files under
`hex-analysis/tests/` are Rust-only.

Two consequences were measured, not predicted. On a real TypeScript project,
`unused_ports` reported four correct ports as unused and took four points off a
refactor that had created them. `dead_layer` reported every layer directory as
dead, ten of them, and that count was displayed as if it meant something.

A gate that is computed in one language and promised in three is a gate that
lies in two.

## Decision

1. **Every detector that feeds the score supports Rust, Go and TypeScript**, or
   declares in its output that it does not apply to the language it was run
   on. Silence is not an option. A detector that cannot read a tree must say
   so rather than report zero or report everything.

2. **Every detector ships with a fixture in each of the three languages**, and
   each fixture has a wired case (no finding) and a broken case (one finding).
   A detector without all six cases is not done.

3. **Detectors build on the shared per-language file model** in
   `treesitter_adapter` (`FileData` with imports and exports already extracted
   for all three languages) rather than parsing on their own. `dead_layer`
   parses with `tree_sitter_rust` directly. That is why it is Rust-only, and it
   is the pattern to remove.

4. **Display-only detectors follow the same rule**, second. They do not move
   the grade, and they are printed next to it, so a wrong count there is read
   as part of the verdict.

## Order of work

| Step | Detector | Why this order |
|---|---|---|
| 1 | `unused_ports` | In the score. The TypeScript miss is understood and small. |
| 2 | `dead_exports` | In the score. Its TypeScript output on `brain` (18) needs verifying against the source before it is trusted. |
| 3 | `dead_layer` | Displayed on every run. Rebuild on `FileData` so all three languages come for free. |
| 4 | `circular_deps` | In the score. Verify TypeScript coverage. |
| 5 | `cohesion`, `duplication`, `god_types`, `orphan` | Display only. Same rebuild pattern as step 3. |

Each step lands with its six fixtures in the same commit.

## Verification

Falsifiable and blocking:

- A test in `hex-analysis/tests/` per detector per language, wired and broken.
  The suite fails if any is missing.
- `hex analyze` on the three shipped scaffolds reports **zero** findings from
  every detector, in all three languages. A fresh scaffold has nothing dead,
  unused, or duplicated by construction. Today, on the TypeScript scaffold,
  `dead_layer` reports every layer.
- The `brain` clone from the refactoring trial, re-analysed after step 1,
  reports 0 unused ports, not 4.

## Consequences

**Positive.** The grade means the same thing in all three languages. The
README's language claim becomes checkable.

**Negative.** Detector rewrites on the shared model may change Rust results
slightly. Each rewrite runs against hex's own tree before and after, and a
change in hex's own grade is a finding, not noise.

**Neutral.** The boundary analyzer, the part that grades rule 1 through 6, was
already three-language. This ADR does not touch it.
