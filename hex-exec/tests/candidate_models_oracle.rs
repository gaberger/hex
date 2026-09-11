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
/// Nothing configured means nothing to run, not a model of hex's choosing.
///
/// This used to assert a hardcoded pair of model ids. Founding goal G1 says no
/// non-test file outside `hex-infer` may name a provider or a model, and a
/// hardcoded last resort is the clearest way to break it: the operator cannot
/// re-point it by editing configuration, and cannot see that it happened,
/// because a run on the wrong model looks exactly like a run on the right one.
///
/// An empty list makes the caller say `NO_MODEL_CONFIGURED` and name the key
/// that is missing.
#[test]
fn nothing_configured_yields_no_candidates() {
    assert_eq!(candidate_models(None, &[], None), Vec::<String>::new());
}
