//! Unit tests for the punch-list linter (wp-workflow-enforcement W1.4).
//!
//! Exercises `extract_punch_list` (W1.1) and `lint_assistant_output` (W1.2)
//! against four canonical shapes mandated by the workplan:
//!
//!   (a) Prose enumeration with no task ids        → violation
//!   (b) Enumeration with each item tagged by id   → clean
//!   (c) Singleton fix statement                   → clean
//!   (d) Table with Status column and no ids       → violation
//!
//! Cases (a) + (d) live in `punch-list-violations.md`; (b) + (c) live in
//! `punch-list-clean.md`. Each test asserts both the extractor count and
//! the linter verdict so regressions in either function are caught.
//!
//! The linter is the authoring-time enforcement of the CLAUDE.md rule
//! "Enqueue, never defer to 'next session'." A prose punch-list with no
//! routing references is exactly the violation this hook must block.

use std::fs;
use std::path::PathBuf;

use hex_cli::commands::hook::punch_list::{
    extract_punch_list, lint_assistant_output, Classification, Reference,
};

fn fixture(name: &str) -> String {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures/hook");
    p.push(name);
    fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {}", p.display(), e))
}

/// An item is routed iff at least one of its references is not `None`.
fn is_routed(refs: &[Reference]) -> bool {
    !refs.is_empty() && refs.iter().any(|r| !matches!(r, Reference::None))
}

// ── (a) Prose enumeration with no task ids — VIOLATION ──────────────────

#[test]
fn prose_enumeration_without_task_ids_is_violation() {
    // Minimal inline fixture: four numbered gap items, zero references.
    let text = "\
Still open before we ship:

1. Need to wire up the subagent-stop hook.
2. Should add a --json flag to hex classify.
3. Fix the escalation report --since filter.
4. Follow-up: per-tier dashboard heatmap.
";

    let items = extract_punch_list(text);
    let enumeration: Vec<_> = items
        .iter()
        .filter(|i| matches!(i.classification, Classification::Enumeration))
        .collect();
    assert!(
        enumeration.len() >= 4,
        "expected at least 4 enumeration items, got {}: {:#?}",
        enumeration.len(),
        items
    );
    assert!(
        enumeration.iter().all(|i| !is_routed(&i.references)),
        "none of the prose enumeration items should carry a routing reference"
    );

    let verdict = lint_assistant_output(text);
    assert!(
        !verdict.violations.is_empty(),
        "unrouted prose enumeration must produce violations"
    );
    assert!(
        verdict.self_correct_note.is_some(),
        "linter must emit a self-correct note when violations exist"
    );
    let note = verdict.self_correct_note.as_deref().unwrap_or_default();
    assert!(
        note.to_lowercase().contains("enqueue")
            || note.to_lowercase().contains("route")
            || note.to_lowercase().contains("gaps"),
        "self-correct note should nudge the agent to enqueue/route the gaps: {:?}",
        note
    );
}

// ── (b) Enumeration with each item tagged by task id — CLEAN ────────────

#[test]
fn enumeration_with_task_ids_is_clean() {
    let text = "\
Routed gaps — every item carries a task id:

1. Wire subagent-stop hook (task a1b2c3d4-e5f6-4789-9abc-def012345678).
2. Add --json flag to hex classify
   (task 11111111-2222-4333-8444-555555555555).
3. Fix escalation report --since filter
   (task ffffffff-eeee-4ddd-8ccc-bbbbbbbbbbbb).
4. Per-tier dashboard heatmap — see
   docs/workplans/drafts/draft-2604241200-dashboard-heatmap.json.
";

    let items = extract_punch_list(text);
    let enumeration: Vec<_> = items
        .iter()
        .filter(|i| matches!(i.classification, Classification::Enumeration))
        .collect();
    assert!(
        enumeration.len() >= 4,
        "expected at least 4 enumeration items, got {}: {:#?}",
        enumeration.len(),
        items
    );
    assert!(
        enumeration.iter().all(|i| is_routed(&i.references)),
        "every routed enumeration item must have a non-None reference: {:#?}",
        enumeration
    );

    let verdict = lint_assistant_output(text);
    assert!(
        verdict.violations.is_empty(),
        "routed enumeration must not produce violations, got: {:#?}",
        verdict.violations
    );
    assert!(
        verdict.self_correct_note.is_none(),
        "no violations → no self-correct note"
    );
}

// ── (c) Singleton fix statement — CLEAN ─────────────────────────────────

#[test]
fn singleton_fix_statement_is_clean() {
    // A one-off fix mention is not a punch-list — even without a reference
    // the linter must not flag it. An enumeration requires >= 2 items.
    let text = "\
Looked into the daemon race condition today. Fix: guard the heartbeat
write with a mutex inside `hex brain daemon`. That's the entire change.
";

    let items = extract_punch_list(text);
    // If the extractor recognises it at all it must be classified as
    // Singleton or InlineFix, NOT Enumeration.
    for item in &items {
        assert!(
            !matches!(item.classification, Classification::Enumeration),
            "singleton prose must not be classified as Enumeration: {:#?}",
            item
        );
    }

    let verdict = lint_assistant_output(text);
    assert!(
        verdict.violations.is_empty(),
        "singleton fix statement must not trigger the linter, got: {:#?}",
        verdict.violations
    );
    assert!(
        verdict.self_correct_note.is_none(),
        "no violations → no self-correct note"
    );
}

// ── (d) Table with Status column and no ids — VIOLATION ─────────────────

#[test]
fn status_table_without_ids_is_violation() {
    let text = "\
Subsystem health — nothing is routed yet:

| Subsystem      | Status   | Notes                                    |
|----------------|----------|------------------------------------------|
| inbox watcher  | pending  | Needs RFC-3339 timestamps on ack events  |
| autoscaler     | broken   | Backs off too aggressively under load    |
| reconcile loop | pending  | Ignores worktree-local edits             |
";

    let items = extract_punch_list(text);
    assert!(
        !items.is_empty(),
        "status-column table should produce at least one PunchItem"
    );
    assert!(
        items.iter().all(|i| !is_routed(&i.references)),
        "no row carries a task id, every extracted item must be unrouted: {:#?}",
        items
    );

    let verdict = lint_assistant_output(text);
    assert!(
        !verdict.violations.is_empty(),
        "status-column table with no ids must produce violations"
    );
    assert!(
        verdict.self_correct_note.is_some(),
        "violations must come with a self-correct note"
    );
}

// ── Fixture-file round-trips ────────────────────────────────────────────

#[test]
fn violations_fixture_produces_violations() {
    let text = fixture("punch-list-violations.md");
    let items = extract_punch_list(&text);
    assert!(
        items.len() >= 4,
        "violations fixture bundles a numbered list + a status table; expected >= 4 items, got {}",
        items.len()
    );
    assert!(
        items.iter().all(|i| !is_routed(&i.references)),
        "every item in the violations fixture is deliberately unrouted: {:#?}",
        items
    );

    let verdict = lint_assistant_output(&text);
    // Both the numbered-list case (a) and the status-table case (d) should
    // contribute. We require at least the numbered items to be flagged.
    assert!(
        verdict.violations.len() >= 4,
        "expected at least 4 violations (4 prose items + table rows), got {}: {:#?}",
        verdict.violations.len(),
        verdict.violations
    );
    assert!(verdict.self_correct_note.is_some());
}

#[test]
fn clean_fixture_produces_no_violations() {
    let text = fixture("punch-list-clean.md");
    let items = extract_punch_list(&text);

    // Every enumeration item must be routed. A singleton in the fixture is
    // allowed to have no references because Classification::Singleton /
    // InlineFix are never treated as gap enumerations by the linter.
    let enumeration_items: Vec<_> = items
        .iter()
        .filter(|i| matches!(i.classification, Classification::Enumeration))
        .collect();
    assert!(
        enumeration_items.len() >= 4,
        "clean fixture has a routed numbered list (>=4 items), got {} enumeration items",
        enumeration_items.len()
    );
    assert!(
        enumeration_items.iter().all(|i| is_routed(&i.references)),
        "every enumeration item in the clean fixture must be routed: {:#?}",
        enumeration_items
    );

    let verdict = lint_assistant_output(&text);
    assert!(
        verdict.violations.is_empty(),
        "clean fixture must produce zero violations, got: {:#?}",
        verdict.violations
    );
    assert!(verdict.self_correct_note.is_none());
}
