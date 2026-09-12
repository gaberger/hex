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

// ── writing ──────────────────────────────────────────────────────────────────

/// Add or replace an endpoint by id, and write the file.
///
/// `hex config inference add` used to PATCH the daemon, which wrote a
/// SpacetimeDB row, which `config_sync` then preloaded back out of this same
/// file on the next startup (ADR-2026-04-08-0813). The file was always the
/// source; the database was a copy of it.
pub fn upsert(endpoint: Endpoint) -> Result<(), String> {
    let mut all = load();
    match all.iter_mut().find(|e| e.id == endpoint.id) {
        Some(existing) => *existing = endpoint,
        None => all.push(endpoint),
    }
    save(&all)
}

/// Remove an endpoint by id. Returns whether it was there.
pub fn remove(id: &str) -> Result<bool, String> {
    let mut all = load();
    let before = all.len();
    all.retain(|e| e.id != id);
    if all.len() == before {
        return Ok(false);
    }
    save(&all).map(|_| true)
}

/// Write the registry, preserving the on-disk shape.
///
/// `models` goes back out as a string-encoded array and the keys stay
/// camelCase, because an older hex, and the operator's own editor, both read
/// this file. Changing its shape to suit the in-memory type would be the
/// convenience of one process paid for by everything else that opens it.
pub fn save(endpoints: &[Endpoint]) -> Result<(), String> {
    let path = registry_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let rows: Vec<serde_json::Value> = endpoints.iter().map(endpoint_to_json).collect();
    let doc = serde_json::json!({
        "endpoints": rows,
        "updated_at": chrono::Utc::now().to_rfc3339(),
        "version": 1,
    });
    let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    // Write-then-rename: a killed process must not leave a half-written
    // registry, because an unparseable one reads as "no backends at all".
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
}

fn endpoint_to_json(e: &Endpoint) -> serde_json::Value {
    let models = if e.models.is_empty() { vec![e.model.clone()] } else { e.models.clone() };
    serde_json::json!({
        "id": e.id,
        "url": e.url,
        "provider": e.provider,
        "model": e.model,
        "models": serde_json::to_string(&models).unwrap_or_else(|_| "[]".into()),
        "status": e.status,
        "requiresAuth": e.requires_auth,
        "apiKeyRef": if e.secret_key.is_empty() { serde_json::Value::Null }
                     else { serde_json::Value::String(e.secret_key.clone()) },
        "healthCheckedAt": e.health_checked_at,
        "qualityScore": e.quality_score,
        "quantizationLevel": e.quantization_level,
    })
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

    /// Round-tripping must survive the file's quirks, not just our own types.
    #[test]
    fn a_saved_endpoint_reads_back_identically() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", dir.path());
        let ep = Endpoint {
            id: "tt".into(),
            url: "https://x/v1".into(),
            provider: "openai_compat".into(),
            model: "Qwen/Qwen3-32B".into(),
            models: vec!["Qwen/Qwen3-32B".into(), "other".into()],
            status: "healthy".into(),
            requires_auth: true,
            secret_key: "TT_KEY".into(),
            health_checked_at: "2026-09-11T00:00:00Z".into(),
            quality_score: 0.5,
            quantization_level: "cloud".into(),
        };
        save(std::slice::from_ref(&ep)).expect("save");
        let back = load();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0], ep);
    }

    #[test]
    fn upsert_replaces_by_id_and_remove_reports_absence() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", dir.path());
        let mk = |id: &str, model: &str| Endpoint {
            id: id.into(),
            url: "http://x".into(),
            provider: "ollama".into(),
            model: model.into(),
            models: vec![model.into()],
            status: "unknown".into(),
            requires_auth: false,
            secret_key: String::new(),
            health_checked_at: String::new(),
            quality_score: 0.0,
            quantization_level: String::new(),
        };
        upsert(mk("a", "one")).expect("insert");
        upsert(mk("b", "two")).expect("insert");
        upsert(mk("a", "rewritten")).expect("replace");
        let all = load();
        assert_eq!(all.len(), 2, "an id is replaced, not duplicated");
        assert_eq!(all.iter().find(|e| e.id == "a").unwrap().model, "rewritten");

        assert!(remove("a").expect("remove"));
        assert!(!remove("a").expect("remove again"), "already gone");
        assert_eq!(load().len(), 1);
    }

}
