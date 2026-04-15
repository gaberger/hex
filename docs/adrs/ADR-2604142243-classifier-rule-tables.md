# ADR-2604142243 — Classifier rules as data tables, not control flow

**Status:** accepted
**Date:** 2026-04-14
**Relates to:** ADR-2604110227 (T1/T2/T3 task tier routing), ADR-2604131500 (developer steering directives)

## Context

Two recent bugs in hex landed because keyword classifiers expressed
precedence as if-elif ordering rather than data:

1. `routes::steer::classify_directive` — "tests must pass before merge"
   misclassified as `priority_change` because the priority branch
   (matching `"before"`) fired before the constraint branch (matching
   `"must"`). Surfaced as a unit-test failure on main for an unknown
   number of weeks. Fixed in `94f3ee6f` by lifting rules into a
   precedence-ordered table with structural assertions.

2. `hex-cli::hook::classify_work_intent` (T1/T2/T3 task-tier routing per
   ADR-2604110227) — same shape: nested `if score >= … else if …` chain
   with implicit ordering, no test that pins the precedence as a
   structural property. The blast radius is much larger: this classifier
   gates whether a prompt auto-spawns a workplan draft, prints a
   mini-plan, or runs as a `TodoWrite`. A misfire here silently
   degrades the autonomous-loop ergonomics across every session.

## Decision

Every classifier in hex that maps free-form text to a discrete label MUST
be expressed as a precedence-ordered data table (`&[Rule]`-style) rather
than as an if-elif control-flow chain. The table MUST:

- carry a `label` and a `match_fn` (or pattern-match struct) per row
- treat row order as semantic precedence
- be accompanied by a `test_rule_table_invariants`-style structural
  test that pins the most important precedence claims by index
- carry a short `signals` description string per row for debuggability
  and rule-listing endpoints

This ADR retroactively codifies the pattern shipped in `94f3ee6f` and
prescribes its application to:

- `hex-cli::hook::classify_work_intent` (T1/T2/T3 prompt classifier)
- `hex-nexus::orchestration::directive::*` (any other directive
  classification helpers, if present)
- Any future classifier added under any crate

## Consequences

- **Pro:** Precedence becomes assertable as data — future contributors
  cannot silently reorder rules without breaking a structural test.
- **Pro:** Rule tables can be exposed via diagnostic endpoints (e.g.
  `/api/classifier/rules`) for dashboard/operator visibility.
- **Pro:** Tests document the spec in one place (the table + invariants),
  not scattered across N happy-path tests.
- **Con:** Slightly more line-noise vs a 5-line if-elif chain — worth it
  for any classifier with three or more branches.
- **Migration cost:** ~50 LOC per classifier; ~5 classifiers in the tree.

## Alternatives considered

- **Macros (`classify! { … }`):** rejected — adds a layer of indirection
  before the type system can check the rules. The struct table is plain
  Rust and grep-able.
- **External config (TOML/JSON rule files):** rejected — runtime parsing
  cost + loses compile-time guarantees + makes rule precedence less
  obvious in code review. Reconsider only if non-engineers need to edit
  rules.
- **State the precedence in a doc comment only:** rejected — that's
  exactly what we had, and it didn't prevent either bug.
