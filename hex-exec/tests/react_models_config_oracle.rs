//! Oracle for ADR-2606072044 step 2 — parse the candidate list out of config.
//! Composes candidate_models (step 1). Independent of direct_react.rs.
use hex_exec::direct_react::react_models_from_config_value;
use serde_json::json;

fn s(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|x| x.to_string()).collect()
}

#[test]
fn list_from_config() {
    let cfg = json!({ "inference": { "react_models": ["a", "b"] } });
    assert_eq!(react_models_from_config_value(&cfg, None), s(&["a", "b"]));
}
#[test]
fn single_from_config() {
    let cfg = json!({ "inference": { "react_model": "x" } });
    assert_eq!(react_models_from_config_value(&cfg, None), s(&["x"]));
}
#[test]
fn explicit_overrides_config() {
    let cfg = json!({ "inference": { "react_models": ["a", "b"] } });
    assert_eq!(react_models_from_config_value(&cfg, Some("z")), s(&["z"]));
}
/// An empty config yields no candidates. See the sibling oracle in
/// `candidate_models_oracle.rs`: a hardcoded last-resort model is founding
/// goal G1's failure case, because configuration can no longer re-point it.
#[test]
fn empty_config_yields_no_candidates() {
    let cfg = json!({});
    assert_eq!(react_models_from_config_value(&cfg, None), Vec::<String>::new());
}
