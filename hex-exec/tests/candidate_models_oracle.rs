//! Oracle for ADR-2606072044 step 1 — candidate model resolution precedence.
//! Independent of the impl file the agent edits (direct_react.rs).
use hex_exec::direct_react::candidate_models;

fn s(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|x| x.to_string()).collect()
}

#[test]
fn explicit_model_wins() {
    assert_eq!(candidate_models(Some("foo"), &s(&["a", "b"]), Some("bar")), s(&["foo"]));
}
#[test]
fn configured_list_used_when_no_explicit() {
    assert_eq!(candidate_models(None, &s(&["a", "b"]), Some("bar")), s(&["a", "b"]));
}
#[test]
fn single_fallback_when_list_empty() {
    assert_eq!(candidate_models(None, &[], Some("bar")), s(&["bar"]));
}
#[test]
fn default_pair_when_nothing_set() {
    assert_eq!(
        candidate_models(None, &[], None),
        s(&["devstral-small-2:24b", "qwen2.5-coder:14b"])
    );
}
