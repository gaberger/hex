//! Configuration — where hex learns which models and which endpoints exist.
//!
//! Per ADR-2608241500 P2.4 this replaces `stdb.list_providers()` and the 18
//! `State(...)` reads in the daemon's `inference_complete` handler. Nothing
//! new had to be invented: both files already exist and are already the
//! source of truth.
//!
//! | File | Holds |
//! |---|---|
//! | `.hex/project.json` → `inference` | tier models, react models, timeout |
//! | `~/.hex/inference-servers.json`   | the endpoint registry |
//!
//! # Why the registry file is authoritative, not a cache
//!
//! It reads like a cache — `hex-cli/src/commands/inference.rs:315` calls it
//! one. It is not. `hex-nexus/src/config_sync.rs:151` *preloads SpacetimeDB
//! from this file* on every startup (ADR-2026-04-08-0813). The database was
//! downstream of the file all along, so severing the database costs no data
//! and needs no migration: hex reads the file the daemon was reading.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::endpoint::Endpoint;

/// Fallback request deadline when `.hex/project.json` does not set one.
pub const DEFAULT_TIMEOUT_SECS: u64 = 600;

/// Everything `complete` needs to know, resolved from disk and environment.
#[derive(Debug, Clone, Default)]
pub struct InferenceConfig {
    /// Registered endpoints, in file order.
    pub endpoints: Vec<Endpoint>,
    /// Tier name (`t1`, `t2`, `t2.5`) → model id.
    pub tier_models: std::collections::BTreeMap<String, String>,
    /// Models the ReAct loop may use, in preference order.
    pub react_models: Vec<String>,
    /// Outer deadline for one completion.
    pub timeout_secs: u64,
    /// OpenRouter key, if one is reachable from the environment.
    pub openrouter_api_key: Option<String>,
    /// Anthropic key, if one is reachable from the environment.
    pub anthropic_api_key: Option<String>,
}

impl InferenceConfig {
    /// Load from the standard locations: `.hex/project.json` under `project_root`
    /// and `~/.hex/inference-servers.json`.
    ///
    /// Missing or malformed files are not errors. hex must still run with no
    /// registry at all — the key-based fallbacks in [`crate::complete`] cover
    /// that case, and refusing to start because a config file is absent would
    /// be worse than starting with fewer options.
    pub fn load(project_root: &Path) -> Self {
        let project = ProjectInference::load(&project_root.join(".hex/project.json"));
        Self {
            endpoints: load_registry(&registry_path()),
            tier_models: project.tier_models,
            react_models: project.react_models,
            timeout_secs: project.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS),
            openrouter_api_key: openrouter_key_from_env(),
            anthropic_api_key: anthropic_key_from_env(),
        }
    }

    /// The model configured for a tier, e.g. `"t2"`.
    pub fn tier_model(&self, tier: &str) -> Option<&str> {
        self.tier_models.get(tier).map(String::as_str)
    }

    /// The configured T2 model, but only when it names a *local* model.
    ///
    /// The last-resort Ollama fallback uses this to avoid asking a local
    /// server for a cloud-only id: T2 may be something like `Qwen/Qwen3-32B`,
    /// which Ollama 404s on. A vendor slug (`/`) means cloud.
    pub fn local_t2_model(&self) -> Option<&str> {
        self.tier_model("t2").filter(|m| !m.contains('/'))
    }
}

/// `~/.hex/inference-servers.json`, or a temp path when there is no home dir.
pub fn registry_path() -> PathBuf {
    dirs_home()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".hex/inference-servers.json")
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// The `inference` block of `.hex/project.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ProjectInference {
    #[serde(default)]
    tier_models: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    react_models: Vec<String>,
    #[serde(default)]
    timeout_secs: Option<u64>,
}

impl ProjectInference {
    fn load(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let Ok(root) = serde_json::from_str::<serde_json::Value>(&text) else {
            tracing::warn!(path = ?path, "project.json is not valid JSON — using defaults");
            return Self::default();
        };
        root.get("inference")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    }
}

/// Parse `~/.hex/inference-servers.json` into endpoints.
///
/// The file's `endpoints[]` entries carry camelCase keys and a `models` field
/// that is a JSON array *encoded as a string* — an artifact of the
/// SpacetimeDB row shape it was written from. Both quirks are handled here so
/// nothing downstream has to know about them.
pub fn load_registry(path: &Path) -> Vec<Endpoint> {
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

fn endpoint_from_json(v: &serde_json::Value) -> Option<Endpoint> {
    let id = v.get("id")?.as_str()?.to_string();
    let url = v.get("url")?.as_str()?.to_string();
    let provider = v.get("provider")?.as_str()?.to_string();
    if url.is_empty() {
        return None;
    }
    let model = v
        .get("model")
        .and_then(|m| m.as_str())
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .or_else(|| first_model(v))
        .unwrap_or_default();
    let secret_key = v
        .get("apiKeyRef")
        .and_then(|k| k.as_str())
        .unwrap_or_default()
        .to_string();
    Some(Endpoint {
        id,
        url,
        provider,
        model,
        status: v
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown")
            .to_string(),
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
        quality_score: v
            .get("qualityScore")
            .and_then(|q| q.as_f64())
            .unwrap_or(0.0) as f32,
        quantization_level: v
            .get("quantizationLevel")
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string(),
    })
}

/// Every model an entry advertises. `models` is a JSON array encoded as a
/// string; a bare string is tolerated too.
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
    // Defensive parse for a hand-edited file: strip brackets and quotes.
    raw.trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn first_model(v: &serde_json::Value) -> Option<String> {
    models_of(v).into_iter().next()
}

/// OpenRouter key, from `OPENROUTER_API_KEY` or from `ANTHROPIC_API_KEY` when
/// an OpenRouter key was placed there (the `sk-or-` prefix gives it away —
/// a real configuration this repo has used).
fn openrouter_key_from_env() -> Option<String> {
    std::env::var("OPENROUTER_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
        .or_else(|| {
            std::env::var("ANTHROPIC_API_KEY")
                .ok()
                .filter(|k| k.starts_with("sk-or-"))
        })
}

/// Anthropic key — only when it really is one. A `sk-or-` value in
/// `ANTHROPIC_API_KEY` is an OpenRouter key and must not reach Anthropic.
fn anthropic_key_from_env() -> Option<String> {
    std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|k| k.starts_with("sk-ant-"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(dir: &Path, rel: &str, body: &str) -> PathBuf {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        p
    }

    #[test]
    fn registry_parses_the_real_on_disk_shape() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "inference-servers.json",
            r#"{"version":1,"endpoints":[
              {"id":"ollama","provider":"ollama","url":"http://localhost:11434",
               "model":"gemma4-12b","models":"[\"gemma4-12b\"]","requiresAuth":false,
               "apiKeyRef":null,"status":"unknown","healthCheckedAt":""},
              {"id":"tenstorrent-qwen3-32b","provider":"openai_compat",
               "url":"https://console.tenstorrent.com","model":"Qwen/Qwen3-32B",
               "models":"[\"Qwen/Qwen3-32B\"]","requiresAuth":true,
               "apiKeyRef":"TENSTORRENT","status":"unknown","healthCheckedAt":""}]}"#,
        );
        let eps = load_registry(&p);
        assert_eq!(eps.len(), 2);
        assert_eq!(eps[0].provider, "ollama");
        assert!(!eps[0].requires_auth);
        assert_eq!(eps[1].secret_key, "TENSTORRENT");
        assert!(eps[1].requires_auth);
    }

    #[test]
    fn models_field_is_a_json_array_encoded_as_a_string() {
        let v = serde_json::json!({"models": "[\"a\",\"b\",\"c\"]"});
        assert_eq!(models_of(&v), vec!["a", "b", "c"]);
    }

    #[test]
    fn a_hand_edited_models_field_still_parses() {
        let v = serde_json::json!({"models": "[a, b]"});
        assert_eq!(models_of(&v), vec!["a", "b"]);
    }

    #[test]
    fn an_entry_with_no_url_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "r.json",
            r#"{"endpoints":[{"id":"x","provider":"ollama","url":""}]}"#,
        );
        assert!(load_registry(&p).is_empty());
    }

    #[test]
    fn a_missing_registry_is_not_an_error() {
        assert!(load_registry(Path::new("/nonexistent/nope.json")).is_empty());
    }

    #[test]
    fn malformed_registry_json_yields_no_endpoints_rather_than_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "r.json", "{not json");
        assert!(load_registry(&p).is_empty());
    }

    #[test]
    fn project_json_supplies_tier_models_and_react_models() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".hex/project.json",
            r#"{"inference":{"tier_models":{"t1":"qwen3:4b","t2":"gemma4-12b",
                "t2.5":"devstral-small-2:24b"},
                "react_models":["devstral-small-2:24b","claude-code"],
                "timeout_secs":600}}"#,
        );
        let cfg = InferenceConfig::load(dir.path());
        assert_eq!(cfg.tier_model("t1"), Some("qwen3:4b"));
        assert_eq!(cfg.tier_model("t2.5"), Some("devstral-small-2:24b"));
        assert_eq!(cfg.react_models.len(), 2);
        assert_eq!(cfg.timeout_secs, 600);
    }

    #[test]
    fn a_missing_project_json_falls_back_to_the_default_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = InferenceConfig::load(dir.path());
        assert_eq!(cfg.timeout_secs, DEFAULT_TIMEOUT_SECS);
        assert!(cfg.tier_models.is_empty());
    }

    #[test]
    fn a_cloud_t2_model_is_not_offered_to_the_local_fallback() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".hex/project.json",
            r#"{"inference":{"tier_models":{"t2":"Qwen/Qwen3-32B"}}}"#,
        );
        let cfg = InferenceConfig::load(dir.path());
        assert_eq!(cfg.tier_model("t2"), Some("Qwen/Qwen3-32B"));
        assert_eq!(cfg.local_t2_model(), None);
    }

    #[test]
    fn a_local_t2_model_is_offered_to_the_local_fallback() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".hex/project.json",
            r#"{"inference":{"tier_models":{"t2":"qwen2.5-coder:32b"}}}"#,
        );
        let cfg = InferenceConfig::load(dir.path());
        assert_eq!(cfg.local_t2_model(), Some("qwen2.5-coder:32b"));
    }
}
