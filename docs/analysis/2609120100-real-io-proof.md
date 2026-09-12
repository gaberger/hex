# Does hex reach past pure functions?

**Date:** 2026-09-12
**Question:** Every project hex had built was a pure in-memory library — 6,400
lines across five projects, 180 tests, and **not one HTTP call, database, file
write or UI**. That is the easiest shape for a code generator, and the `hex
build` corpus had been chosen to be gateable, which pure functions are. So the
claim "hex builds working software" was true of a sample that excluded the hard
part.

**Test:** `hex scaffold` a service where the gate must spin something up.

---

## The subject

`examples/linkstore-svc`, from one description:

- **Domain** — a Bookmark and a Tag, URL validation, a normalisation rule
  (strip tracking params, lowercase the host).
- **Ports** — `BookmarkStore`, `Clock`.
- **Secondary adapter** — SQLite via `rusqlite` (bundled), owning an idempotent
  schema migration that runs on open.
- **Primary adapter** — an axum/tokio HTTP API: POST/GET/LIST/DELETE.
- **Gate** — `cargo test`, required to exercise an integration test that opens a
  SQLite file in a temp directory, binds a real OS-assigned TCP port, starts the
  server, issues real `reqwest` calls, and asserts the row is present in the
  `.db` file on disk.

## Result

```
✓ 2 designs → 2 critiques → spec 33505ch → build GREEN
✓ gate re-run: PASS — 27 test(s) ran
✓ architecture grade: A+ — score 100/100 (floor A)
```

27 tests. **12 are genuinely end-to-end**, including `data_survives_a_restart`,
`fifty_concurrent_posts_same_url_different_tags`, `wal_and_foreign_keys_are_
really_on`, and `normalisation_reaches_the_disk`.

## The tests were falsified before being believed

A passing test proves nothing until it can fail. Three sabotages, each reverted:

| Change | Result |
|---|---|
| HTTP `201` → `200` in the primary adapter | **4 tests failed** |
| Domain stops stripping tracking parameters | **3 tests failed**, including the one asserting normalisation reaches disk |
| Restored | **27 passed, 0 failed** |

## What was actually being tested

Not whether a frontier model can write an HTTP service — it can. The question
was whether the **architecture survives real adapters**. With a live database
and a live listener, the shortcut is a use case reaching for `rusqlite` or an
axum type, and no pure-function project ever puts that under pressure.

```
src/usecases/  →  crate::domain, crate::ports, std::sync::Arc
```

No database, no HTTP, no runtime, in the layer that must not have them. The
boundary held.

---

## A real gap this turned up

`src/domain/url.rs` contains `use url::Url;` — a third-party crate inside the
domain layer. hex graded the project **A+ with 0 boundary violations**.

Rule 1, as hex states it in every `CLAUDE.md` it writes, is *"`domain/` imports
only `domain/`"*. **The analyzer does not check that.** It checks layer-to-layer
imports — does `usecases` import an adapter, does an adapter import another
adapter — and is blind to a third-party crate appearing anywhere.

So hex's headline rule is stricter than what hex enforces, and the gap is
invisible from the output: a project can import `tokio` into its domain and
still score A+. This was found by reading the imports by hand, which is exactly
the method the rule set exists to replace.

In this instance it is defensible — `url` is a pure parser with no I/O — but hex
did not make that judgement. It could not see the import.

**Unfixed.** Per ADR-2026-09-11-1900 an unenforced rule is prose with extra
steps, so either the analyzer learns to check domain purity (with an allowlist,
since `serde` in a domain type is normal and `tokio` is not), or rule 1 should
be restated to say what is actually checked. Recorded rather than silently
tolerated.

## What remains unproven

**Brownfield.** All six projects hex has built are greenfield. hex has never
been pointed at a large codebase someone else wrote and asked to change it
safely. That is the remaining claim and it is the harder one: the evidence gate
protects a change, but nothing here has tested whether hex can find the right
change to make in code it did not write.
