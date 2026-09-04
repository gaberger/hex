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

use crate::adapters::{ClaudeCodeInferenceAdapter, OllamaInferenceAdapter};

/// Which runtime serves a model id.
///
/// Deliberately an exact prefix test, not a heuristic on shape. The same reasoning weave's tier
/// router settled on: ollama tags have no common form, so a rule like "contains a colon" misroutes
/// anything vendor-prefixed. A `claude*` id is Anthropic's, everything else is the local runtime —
/// and an id that is not recognised goes to ollama, which can at least say it has never heard of it.
fn adapter_for(model: &str) -> Box<dyn IInferencePort> {
    if model.to_lowercase().starts_with("claude") {
        Box::new(ClaudeCodeInferenceAdapter::new(None))
    } else {
        Box::new(OllamaInferenceAdapter::new(None))
    }
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

    let tools: Vec<hex_core::domain::tools::ToolDefinition> = req
        .get("tools")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

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

    let response = adapter_for(model)
        .complete(request)
        .await
        .map_err(|e| e.to_string())?;

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
