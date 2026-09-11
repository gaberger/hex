//! One text completion, without a daemon in the path.
//!
//! `hex-exec` used to POST `{model, messages, max_tokens}` to
//! `http://127.0.0.1:$HEX_NEXUS_PORT/api/inference/complete` and read `{content}` back. The daemon
//! then called the provider. It added routing and logging to a call that already knew its model —
//! and it made the agent loop unable to run at all unless a control plane was up.
//!
//! This is that call as a function. Same inputs, same output, one process.

use hex_core::domain::messages::{ContentBlock, Message, Role};
use hex_core::ports::inference::{IInferencePort, InferenceRequest, Priority};

use crate::adapters::{
    AnthropicAdapter, ClaudeCodeInferenceAdapter, OllamaInferenceAdapter, OpenAiCompatAdapter,
};
use crate::endpoint::Endpoint;
use crate::registry;

/// Which backend serves a model id.
///
/// **The registry decides.** `~/.hex/inference-servers.json` is asked first,
/// and the matched entry's provider family picks the adapter. Only when no
/// registered endpoint advertises the model does the prefix test below apply.
///
/// It used to be the prefix test alone: `claude*` to `claude -p`, everything
/// else to the local runtime. That made the set of reachable providers a
/// property of the source code — an operator could register an
/// OpenAI-compatible host or an OpenRouter key with `hex inference add`, see
/// it in `hex inference list`, and still have every request to it sent to
/// local ollama, which 404s on an id it has never heard of. Founding goal G1
/// requires that adding or retiring a provider is a configuration change, not
/// a refactor.
///
/// The prefix fallback is kept for the no-registry case, and it is the same
/// reasoning weave's tier router settled on: ollama tags have no common form,
/// so a rule like "contains a colon" misroutes anything vendor-prefixed.
fn adapter_for(model: &str) -> Box<dyn IInferencePort> {
    if let Some(endpoint) = registry::serving(&registry::load(), model) {
        return adapter_for_endpoint(&endpoint);
    }
    if model.to_lowercase().starts_with("claude") {
        Box::new(ClaudeCodeInferenceAdapter::new(None))
    } else {
        Box::new(OllamaInferenceAdapter::new(None))
    }
}

/// The adapter for one registered endpoint.
///
/// The API key is the *name* of an environment variable, resolved here. The
/// daemon kept keys in a SpacetimeDB vault and resolved references at dispatch
/// time under a 3-second timeout — a distributed system standing in for
/// `std::env::var`, for a single-user tool on one machine.
fn adapter_for_endpoint(endpoint: &Endpoint) -> Box<dyn IInferencePort> {
    let key = resolve_key(endpoint);
    match endpoint.provider.to_ascii_lowercase().as_str() {
        // The local runtime streams NDJSON of its own; its adapter speaks that.
        "ollama" => Box::new(OllamaInferenceAdapter::new(Some(endpoint.url.clone()))),
        "anthropic" => Box::new(AnthropicAdapter::new(key, endpoint.model.clone())),
        "claude-code" | "claude_code" => Box::new(ClaudeCodeInferenceAdapter::new(None)),
        // openrouter, openai, openai_compat, vllm, llama-cpp and anything else
        // registered: all of them speak the OpenAI chat-completions shape.
        _ => Box::new(OpenAiCompatAdapter::new(
            key,
            endpoint.url.clone(),
            endpoint.model.clone(),
        )),
    }
}

/// Read an endpoint's key out of the environment variable it names.
fn resolve_key(endpoint: &Endpoint) -> String {
    if endpoint.secret_key.is_empty() {
        return String::new();
    }
    match std::env::var(&endpoint.secret_key) {
        Ok(v) if !v.is_empty() => v,
        // A literal key in the field rather than a variable name: tolerated,
        // because a hand-edited registry is a real thing operators produce.
        _ if endpoint.secret_key.starts_with("sk-") => endpoint.secret_key.clone(),
        _ => {
            tracing::warn!(
                endpoint = %endpoint.id,
                variable = %endpoint.secret_key,
                "no environment value for this endpoint's key reference"
            );
            String::new()
        }
    }
}


/// Tools, or a real error — never a silent empty list.
///
/// This was `.ok().unwrap_or_default()`, which turned any shape mismatch into "no tools". A model
/// given no tools does not fail: it invents a `{"tool": ...}` JSON-in-text convention, which
/// `extract_tool_uses` cannot see, so the loop reports "ended with no edit" and the MODEL takes the
/// blame for a bridge that dropped its arguments on the floor.
fn parse_tools(v: &serde_json::Value) -> Result<Vec<hex_core::domain::tools::ToolDefinition>, String> {
    if v.is_null() {
        return Ok(Vec::new());
    }
    serde_json::from_value(v.clone())
        .map_err(|e| format!("inference: tools did not match ToolDefinition ({e})"))
}

/// Send one system+user turn and return the text of the reply.
///
/// The return is `Result<String, String>` rather than a typed error because every caller in
/// `hex-exec` already funnels failures into a string it shows the operator, and widening that here
/// would be a change to the loop rather than to inference.
pub async fn complete_text(
    model: &str,
    system: &str,
    user: &str,
    max_tokens: u32,
) -> Result<String, String> {
    let request = InferenceRequest {
        model: model.to_string(),
        system_prompt: system.to_string(),
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: user.to_string() }],
        }],
        tools: Vec::new(),
        max_tokens,
        // 0.0: this path asks for a precise edit or a structured reply, never for variety. It is
        // what the daemon route sent, kept rather than re-decided.
        temperature: 0.0,
        thinking_budget: None,
        cache_control: false,
        priority: Priority::default(),
        grammar: None,
    };

    let response = adapter_for(model)
        .complete(request)
        .await
        .map_err(|e| e.to_string())?;

    crate::spend::record(model, response.input_tokens, response.output_tokens);

    // The daemon returned a flat `content` string. Concatenating the text blocks reproduces that
    // exactly for a reply with no tool use, which is all this path ever asks for.
    let text: String = response
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");

    Ok(text)
}

#[cfg(test)]
mod tests {
    /// The classification rule, pinned.
    ///
    /// An exact PREFIX, not a substring: a local model called `my-claude-clone` is not Anthropic's,
    /// and routing it there fails with a message about a model that does exist elsewhere — the
    /// confusing kind of wrong.
    fn is_claude(model: &str) -> bool {
        model.to_lowercase().starts_with("claude")
    }

    #[test]
    fn claude_ids_route_to_claude() {
        assert!(is_claude("claude-sonnet-4-6"));
        assert!(is_claude("Claude-Haiku-4-5"));
    }

    #[test]
    fn local_ids_do_not_and_neither_does_a_lookalike() {
        assert!(!is_claude("qwen2.5-coder:14b"));
        assert!(!is_claude("gemma4-12b:latest"));
        assert!(!is_claude("my-claude-clone"), "substring matching would send this to Anthropic");
    }
}

/// The daemon's `/api/inference/complete` contract, as a function.
///
/// Takes and returns the same JSON the HTTP route did, so the ReAct loop's request building and
/// its `extract_tool_uses` parsing are untouched. That is deliberate: a phase should move the
/// boundary, not rewrite what sits either side of it. `hex-exec` is 9,000 lines of loop that
/// works; the only thing wrong with it was the hop.
///
/// Tolerant about message content on the way in — the loop sends `"content": "text"` for the seed
/// and block arrays for tool results, and the route accepted both.
pub async fn complete_raw(req: &serde_json::Value) -> Result<serde_json::Value, String> {
    let model = req.get("model").and_then(|v| v.as_str()).unwrap_or_default();
    if model.is_empty() {
        return Err("inference: request has no model".into());
    }

    let messages = req
        .get("messages")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().map(message_from_json).collect::<Vec<_>>())
        .unwrap_or_default();

    let tools = parse_tools(req.get("tools").unwrap_or(&serde_json::Value::Null))?;

    let request = InferenceRequest {
        model: model.to_string(),
        system_prompt: req.get("system").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        messages,
        tools,
        max_tokens: req.get("max_tokens").and_then(|v| v.as_u64()).unwrap_or(4096) as u32,
        temperature: req.get("temperature").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
        thinking_budget: None,
        cache_control: false,
        priority: Priority::default(),
        grammar: None,
    };

    // TOOLS MEAN /api/chat. The OllamaInferenceAdapter posts to /api/generate, which has no
    // `tools` parameter and whose `collapse_prompt` sends only the LAST USER MESSAGE — so a
    // multi-step loop got neither its tools nor its transcript. It did not fail loudly: the model
    // invented a JSON-in-text convention that `extract_tool_uses` cannot see, and the loop
    // reported "ended with no edit" as though the model were incapable.
    let response = if !request.tools.is_empty() && !model.to_lowercase().starts_with("claude") {
        crate::adapters::ollama_chat::chat(
            &std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://127.0.0.1:11434".to_string()),
            std::time::Duration::from_secs(600),
            request,
        )
        .await
        .map_err(|e| e.to_string())?
    } else {
        adapter_for(model)
            .complete(request)
            .await
            .map_err(|e| e.to_string())?
    };

    // ContentBlock's serde renames already produce Anthropic's {type: text|tool_use} shape, which
    // is exactly what extract_tool_uses reads. No hand-rolled mapping to drift.
    crate::spend::record(model, response.input_tokens, response.output_tokens);

    Ok(serde_json::json!({
        "content": serde_json::to_value(&response.content).map_err(|e| e.to_string())?,
        "model": response.model_used,
        "usage": {
            "input_tokens": response.input_tokens,
            "output_tokens": response.output_tokens,
        },
    }))
}

/// One message, from the loop's JSON. Content may be a bare string or a block array.
fn message_from_json(v: &serde_json::Value) -> Message {
    let role = match v.get("role").and_then(|r| r.as_str()) {
        Some("assistant") => Role::Assistant,
        _ => Role::User,
    };
    let content = match v.get("content") {
        Some(serde_json::Value::String(s)) => vec![ContentBlock::Text { text: s.clone() }],
        Some(arr @ serde_json::Value::Array(_)) => serde_json::from_value(arr.clone())
            .unwrap_or_else(|_| vec![ContentBlock::Text { text: arr.to_string() }]),
        _ => Vec::new(),
    };
    Message { role, content }
}

#[cfg(test)]
mod tool_tests {
    use super::*;

    /// The shape `direct_react::curated_schema` actually emits.
    fn curated_like() -> serde_json::Value {
        serde_json::json!([
            { "name": "repo_read", "description": "Read a file",
              "input_schema": { "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] } },
            { "name": "propose_edit", "description": "Apply the edit and run evidence",
              "input_schema": { "type": "object", "properties": { "mode": { "type": "string" } }, "required": [] } }
        ])
    }

    #[test]
    fn the_curated_schema_survives_the_bridge() {
        let tools = parse_tools(&curated_like()).expect("curated schema must deserialize");
        assert_eq!(tools.len(), 2, "both tools must reach the model");
        assert_eq!(tools[0].name, "repo_read");
    }

    /// The bug this replaced: `.ok().unwrap_or_default()` turned a shape mismatch into an EMPTY
    /// tool list. The model then has no tools, invents a `{"tool": …}` JSON-in-text convention,
    /// and `extract_tool_uses` finds nothing — so the loop reports "ended with no edit" and it
    /// reads as the model being incapable. Observed with devstral-small-2:24b, which emits a
    /// perfectly good structured tool_call when it is actually given tools.
    #[test]
    fn a_malformed_tool_is_an_error_not_a_silent_empty_list() {
        // `input_schema` missing its required `type`.
        let bad = serde_json::json!([{ "name": "x", "description": "d", "input_schema": { "properties": {} } }]);
        assert!(parse_tools(&bad).is_err(), "a shape mismatch must be reported, never swallowed");
    }

    #[test]
    fn absent_tools_are_legitimately_empty() {
        assert_eq!(parse_tools(&serde_json::Value::Null).unwrap().len(), 0);
    }
}
