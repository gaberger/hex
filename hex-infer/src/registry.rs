//! The operator's registered inference backends, read from disk.
//!
//! `~/.hex/inference-servers.json` is the file `hex inference add` writes and
//! `hex inference list` prints. The daemon used to preload SpacetimeDB from it
//! on every startup (ADR-2026-04-08-0813), so the database was always
//! downstream of this file — severing the daemon costs no data and needs no
//! migration. hex reads the same file the daemon read.
//!
//! # Why this exists
//!
//! Without it, [`crate::complete::adapter_for`] can only choose between the
//! local runtime and `claude -p`, by testing whether the model id starts with
//! `claude`. Every other registered backend — an OpenAI-compatible host, an
//! OpenRouter key, a remote GPU box — is unreachable, and a request for one of
//! their models is sent to the local runtime, which 404s on an id it has never
//! heard of.
//!
//! That is a founding-goal violation, not a missing feature: **G1** requires
//! that adding or retiring a provider is a configuration change, not a
//! refactor. Routing on a hardcoded prefix makes the set of reachable
//! providers a property of the source code.

use std::path::PathBuf;

use crate::endpoint::Endpoint;

/// `~/.hex/inference-servers.json`, or a temp path when there is no home.
pub fn registry_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".hex/inference-servers.json")
}

/// Every registered endpoint, in file order.
///
/// A missing or malformed file is not an error: hex must still run with no
/// registry at all, falling back to the local runtime.
pub fn load() -> Vec<Endpoint> {
    load_from(&registry_path())
}

/// Parse a registry file into endpoints.
///
/// Entries carry camelCase keys and a `models` field that is a JSON array
/// *encoded as a string* — an artifact of the SpacetimeDB row shape they were
/// written from. Both quirks are absorbed here so nothing downstream knows.
pub fn load_from(path: &std::path::Path) -> Vec<Endpoint> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(&text) else {
        tracing::warn!(path = ?path, "inference registry is not valid JSON — ignoring");
        return Vec::new();
    };
    let Some(entries) = root.get("endpoints").and_then(|e| e.as_array()) else {
        return Vec::new();
    };
    entries.iter().filter_map(endpoint_from_json).collect()
}

/// The endpoint that serves `model`, with `model` recorded on it.
///
/// Exact element match, never substring: a substring search routes a request
/// for `llama-3` to any provider whose list happens to contain
/// `meta-llama/llama-3.3-70b-instruct`.
pub fn serving(endpoints: &[Endpoint], model: &str) -> Option<Endpoint> {
    endpoints
        .iter()
        .find(|e| e.models.iter().any(|m| m == model) || e.model == model)
        .map(|e| {
            let mut e = e.clone();
            e.model = model.to_string();
            e
        })
}

fn endpoint_from_json(v: &serde_json::Value) -> Option<Endpoint> {
    let id = v.get("id")?.as_str()?.to_string();
    let url = v.get("url").and_then(|u| u.as_str()).unwrap_or_default().to_string();
    let provider = v.get("provider")?.as_str()?.to_string();
    if url.is_empty() {
        return None;
    }
    let models = models_of(v);
    let secret_key =
        v.get("apiKeyRef").and_then(|k| k.as_str()).unwrap_or_default().to_string();
    Some(Endpoint {
        id,
        url,
        provider,
        model: v
            .get("model")
            .and_then(|m| m.as_str())
            .filter(|m| !m.is_empty())
            .map(str::to_string)
            .or_else(|| models.first().cloned())
            .unwrap_or_default(),
        models,
        status: v.get("status").and_then(|s| s.as_str()).unwrap_or("unknown").to_string(),
        requires_auth: v
            .get("requiresAuth")
            .and_then(|b| b.as_bool())
            .unwrap_or(!secret_key.is_empty()),
        secret_key,
        health_checked_at: v
            .get("healthCheckedAt")
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string(),
        quality_score: v.get("qualityScore").and_then(|q| q.as_f64()).unwrap_or(0.0) as f32,
        quantization_level: v
            .get("quantizationLevel")
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string(),
    })
}

/// Every model an entry advertises.
///
/// `models` is a JSON array encoded as a string; a bare string, and a
/// hand-edited file, are both tolerated.
pub fn models_of(v: &serde_json::Value) -> Vec<String> {
    let Some(raw) = v.get("models").and_then(|m| m.as_str()) else {
        return v
            .get("model")
            .and_then(|m| m.as_str())
            .map(|m| vec![m.to_string()])
            .unwrap_or_default();
    };
    if let Ok(list) = serde_json::from_str::<Vec<String>>(raw) {
        return list;
    }
    raw.trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(json: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("inference-servers.json");
        std::fs::write(&path, json).expect("write");
        (dir, path)
    }

    #[test]
    fn a_missing_registry_is_empty_not_an_error() {
        assert!(load_from(std::path::Path::new("/nonexistent/registry.json")).is_empty());
    }

    #[test]
    fn a_malformed_registry_is_empty_not_a_panic() {
        let (_d, p) = write("{ not json");
        assert!(load_from(&p).is_empty());
        let (_d2, p2) = write(r#"{"no_endpoints_key": true}"#);
        assert!(load_from(&p2).is_empty());
    }

    #[test]
    fn the_models_field_parses_as_a_string_encoded_array() {
        let (_d, p) = write(
            r#"{"endpoints":[{"id":"tt","url":"https://x/v1","provider":"openai_compat",
                "models":"[\"Qwen/Qwen3-32B\",\"deepseek-ai/DeepSeek-R1-0528\"]"}]}"#,
        );
        let eps = load_from(&p);
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].models, ["Qwen/Qwen3-32B", "deepseek-ai/DeepSeek-R1-0528"]);
        // The first advertised model becomes the default.
        assert_eq!(eps[0].model, "Qwen/Qwen3-32B");
    }

    #[test]
    fn a_hand_edited_models_field_still_parses() {
        let (_d, p) = write(
            r#"{"endpoints":[{"id":"a","url":"http://x","provider":"ollama",
                "models":"[gemma4-12b, qwen3:4b]"}]}"#,
        );
        assert_eq!(load_from(&p)[0].models, ["gemma4-12b", "qwen3:4b"]);
    }

    #[test]
    fn an_entry_with_no_url_is_dropped() {
        let (_d, p) = write(r#"{"endpoints":[{"id":"a","provider":"ollama","models":"[\"m\"]"}]}"#);
        assert!(load_from(&p).is_empty());
    }

    #[test]
    fn serving_matches_an_advertised_model_exactly() {
        let (_d, p) = write(
            r#"{"endpoints":[
                {"id":"tt","url":"https://x/v1","provider":"openai_compat","models":"[\"Qwen/Qwen3-32B\"]"},
                {"id":"local","url":"http://127.0.0.1:11434","provider":"ollama","models":"[\"qwen3:4b\"]"}]}"#,
        );
        let eps = load_from(&p);
        let hit = serving(&eps, "Qwen/Qwen3-32B").expect("matched");
        assert_eq!(hit.id, "tt");
        assert_eq!(hit.model, "Qwen/Qwen3-32B");
        assert_eq!(serving(&eps, "qwen3:4b").unwrap().id, "local");
    }

    #[test]
    fn serving_never_matches_on_a_substring() {
        let (_d, p) = write(
            r#"{"endpoints":[{"id":"or","url":"https://openrouter.ai/api/v1","provider":"openrouter",
                "models":"[\"meta-llama/llama-3.3-70b-instruct\"]"}]}"#,
        );
        assert!(serving(&load_from(&p), "llama-3").is_none());
    }

    #[test]
    fn the_api_key_reference_is_carried_as_a_variable_name() {
        let (_d, p) = write(
            r#"{"endpoints":[{"id":"tt","url":"https://x/v1","provider":"openai_compat",
                "apiKeyRef":"TENSTORRENT_API_KEY","models":"[\"m\"]"}]}"#,
        );
        let ep = &load_from(&p)[0];
        assert_eq!(ep.secret_key, "TENSTORRENT_API_KEY");
        assert!(ep.requires_auth, "a key reference implies auth is required");
    }
}
