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
#[test]
fn default_pair_when_empty() {
    let cfg = json!({});
    assert_eq!(
        react_models_from_config_value(&cfg, None),
        s(&["devstral-small-2:24b", "qwen2.5-coder:14b"])
    );
}
