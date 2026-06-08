//! Oracle for ADR-2606072044 — best-of-N selection: first candidate that passed,
//! else the last attempt. Pure; independent of direct_react.rs.
use hex_exec::direct_exec::DirectResult;
use hex_exec::direct_react::select_best_of_n;

fn pass() -> DirectResult {
    DirectResult {
        ok: true, attempts: 1, edit_applied: true,
        committed: Some("h".into()), evidence_passed: true,
        evidence_output: String::new(), error: None,
    }
}
fn fail() -> DirectResult {
    DirectResult {
        ok: false, attempts: 1, edit_applied: false,
        committed: None, evidence_passed: false,
        evidence_output: String::new(), error: Some("x".into()),
    }
}

#[test]
fn first_passing_wins() {
    let (m, r) = select_best_of_n(vec![("a".into(), fail()), ("b".into(), pass()), ("c".into(), pass())]);
    assert_eq!(m, "b");
    assert!(r.ok);
}
#[test]
fn all_fail_returns_last() {
    let (m, r) = select_best_of_n(vec![("a".into(), fail()), ("b".into(), fail())]);
    assert_eq!(m, "b");
    assert!(!r.ok);
}
#[test]
fn single_candidate() {
    let (m, _) = select_best_of_n(vec![("only".into(), pass())]);
    assert_eq!(m, "only");
}
