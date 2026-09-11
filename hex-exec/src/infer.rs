//! In-process inference (ADR-2608241500 P2.5).
//!
//! Every agent loop in `hex-exec` used to reach inference by POSTing to
//! `http://127.0.0.1:5555/api/inference/complete` — a localhost hop into
//! `hex-nexus`, which then called the provider adapters. The daemon
//! contributed the hop and nothing else: after P2.4 lifted
//! `inference_complete` out of its axum handler, no `State` read survived.
//!
//! `hex-infer` owns the provider adapters, tier routing, the fallback chain
//! and the frontier escape hatch as a plain library, so the loop calls it
//! directly. Request and response shapes are unchanged — [`complete_json`]
//! takes the JSON body the HTTP endpoint took and returns the JSON object it
//! returned — which is why this task touches no tool-call parser.
//!
//! # Founding goal G1 — model independence
//!
//! No provider name appears in this module, and none may. Whether the answer
//! comes from Ollama, an OpenAI-compatible endpoint, Anthropic or a `claude -p`
//! subprocess is resolved inside `hex-infer` from `.hex/project.json` and
//! `~/.hex/inference-servers.json`.

use hex_infer::{CompleteRequest, Completion, InferenceConfig};
use serde_json::Value;

/// Resolve inference configuration for the current repository.
///
/// Deliberately re-read per call rather than cached in a `OnceLock`. Both
/// files are small, a ReAct loop makes tens of calls rather than thousands,
/// and re-reading means `hex inference add` takes effect in a running loop
/// instead of at the next process start.
pub fn config() -> InferenceConfig {
    InferenceConfig::load(&crate::direct_exec::repo_root())
}

/// Run one completion against the repository's configured providers.
///
/// `caller` names the loop and `intent` what it was doing; both are recorded
/// for the cost meter. Usage recording is best-effort — losing an accounting
/// row must never lose a completion.
pub async fn complete(
    caller: &str,
    intent: &str,
    request: CompleteRequest,
) -> Result<Completion, String> {
    let answer = hex_infer::complete(request, &config()).await.map_err(|e| e.to_string())?;
    record_usage(caller, intent, &answer);
    Ok(answer)
}

/// Run one completion, speaking the JSON shapes the old HTTP endpoint spoke.
///
/// `body` is `{model, messages, system?, max_tokens?, tools?}`; the success
/// value is `{content, model, input_tokens, output_tokens, tool_calls,
/// provider}`. Unknown fields are ignored, as the endpoint ignored them.
pub async fn complete_json(caller: &str, intent: &str, body: Value) -> Result<Value, String> {
    let request: CompleteRequest =
        serde_json::from_value(body).map_err(|e| format!("malformed inference request: {e}"))?;
    complete(caller, intent, request).await.map(|c| c.to_json())
}

/// Longest `intent` kept in an accounting row. The cost meter groups on it;
/// a whole instruction would make every group unique and the grouping useless.
const INTENT_LEN: usize = 120;

/// Append one accounting row for a completion that succeeded.
///
/// This is new ground, not a port: the cost meter used to read a table the
/// daemon wrote on the agent's behalf. In-process, the caller that placed the
/// request is the only thing that knows the answer, so it records its own.
fn record_usage(caller: &str, intent: &str, answer: &Completion) {
    use hex_core::ports::local_store::{ILocalStore, UsageRecord};
    let record = UsageRecord {
        at: crate::store::now_rfc3339(),
        model: answer.model.clone(),
        role: caller.to_string(),
        intent: intent.chars().take(INTENT_LEN).collect(),
        input_tokens: answer.input_tokens,
        output_tokens: answer.output_tokens,
        cost_usd: answer
            .openrouter_cost_usd
            .as_deref()
            .and_then(|c| c.parse::<f64>().ok()),
    };
    if let Err(e) = crate::store::FileStore::current().append_usage(&record) {
        tracing::debug!(error = %e, "usage accounting failed (non-fatal)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The request bodies the loops build must still deserialize into a
    /// `CompleteRequest`. This is the contract P2.5 relies on to leave every
    /// call site's `json!` block untouched.
    #[test]
    fn react_request_body_deserializes() {
        let body = json!({
            "model": "some-model",
            "max_tokens": 8192,
            "system": "you are a code editor",
            "tools": [{"name": "read_file", "input_schema": {}}],
            "messages": [{"role": "user", "content": "hello"}],
        });
        let req: CompleteRequest = serde_json::from_value(body).expect("deserializes");
        assert_eq!(req.model.as_deref(), Some("some-model"));
        assert_eq!(req.max_tokens, 8192);
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.tools.as_ref().map(Vec::len), Some(1));
    }

    /// The single-shot executor sends no `system`, no `tools`, and relies on
    /// the default generation cap when it omits `max_tokens`.
    #[test]
    fn exec_request_body_defaults() {
        let body = json!({
            "messages": [{"role": "user", "content": "edit this"}],
        });
        let req: CompleteRequest = serde_json::from_value(body).expect("deserializes");
        assert!(req.model.is_none());
        assert!(req.tools.is_none());
        assert_eq!(req.max_tokens, hex_infer::complete::DEFAULT_MAX_TOKENS);
    }

    /// Loading config must never fail, even with no registry and no project
    /// file — the fallback chain in `hex-infer` covers that case, and refusing
    /// to start because a config file is absent would be worse.
    #[test]
    fn config_loads_without_files() {
        let empty = tempfile::tempdir().expect("tempdir");
        let cfg = InferenceConfig::load(empty.path());
        assert!(cfg.timeout_secs > 0);
    }
}
