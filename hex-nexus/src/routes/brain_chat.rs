//! POST /api/brain/chat — operator chat dispatch for the Brain dashboard.
//!
//! wp-brain-dashboard M3.
//!
//! Operator types `@<role> <message>` in the Brain chat pane. Frontend POSTs
//! { role, message } here. We:
//!   1. Load the role's YAML persona from embedded AgentTemplates
//!   2. Build a system prompt from persona.description + persona.constraints
//!   3. Call inference_complete with the persona's preferred model
//!   4. Return { content, model, role } for the frontend to render
//!
//! No swarm/task creation, no worker spawn — this is the lightweight chat path
//! for "I want to ask my agent something". For full workflow execution
//! (multi-iteration, gates, commits) the operator still uses
//! `hex agent worker --role <name> --once`.

use axum::{extract::State, Json};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::state::SharedState;
use crate::templates::AgentTemplates;
use crate::routes::inference::{inference_complete, InferenceCompleteRequest};

#[derive(Debug, Deserialize)]
pub struct BrainChatRequest {
    /// Persona role name (e.g. "pm-agent", "adversarial-red"). Must have a
    /// matching YAML at hex-cli/assets/agents/hex/hex/<role>.yml.
    pub role: String,
    /// Operator message — free text, the agent's task.
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct BrainChatResponse {
    pub role: String,
    pub model: String,
    pub content: String,
}

/// Minimal persona shape — we only need the few fields used in prompt construction.
/// Avoids depending on hex-cli's AgentDefinition (which lives in another crate).
#[derive(Debug, Deserialize, Default)]
struct PersonaSnippet {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    constraints: Vec<serde_yaml::Value>,
    #[serde(default)]
    model: PersonaModel,
}

#[derive(Debug, Deserialize, Default)]
struct PersonaModel {
    #[serde(default)]
    preferred: Option<String>,
}

fn load_persona(role: &str) -> Option<PersonaSnippet> {
    let path = format!("agents/hex/hex/{}.yml", role);
    let bytes = AgentTemplates::get(&path)?;
    let content = std::str::from_utf8(&bytes.data).ok()?;
    serde_yaml::from_str::<PersonaSnippet>(content).ok()
}

fn constraints_as_strings(values: &[serde_yaml::Value]) -> Vec<String> {
    values.iter().filter_map(|v| match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        // Some YAMLs nest constraints as { rule: ..., why: ... } maps.
        // Render the first scalar found.
        serde_yaml::Value::Mapping(m) => m.values().find_map(|val| match val {
            serde_yaml::Value::String(s) => Some(s.clone()),
            _ => None,
        }),
        _ => None,
    }).collect()
}

/// Map a YAML model name to the inference-gateway model id. Keep this in sync
/// with hex-cli/src/pipeline/agent_def.rs::ModelConfig::resolve_model_id.
/// Falls through to passthrough for Ollama-style "<name>:<tag>" identifiers.
fn resolve_model_id(name: &str) -> String {
    match name {
        "sonnet" | "claude-sonnet" => "claude-sonnet-4-6".to_string(),
        "haiku" | "claude-haiku" => "claude-haiku-4-5-20251001".to_string(),
        "opus" | "claude-opus" => "claude-opus-4-6".to_string(),
        "gpt-4o" => "openai/gpt-4o".to_string(),
        "gpt-4o-mini" => "openai/gpt-4o-mini".to_string(),
        n if n.contains(':') => n.to_string(),
        _ => "openrouter/free".to_string(),
    }
}

pub async fn dispatch_brain_chat(
    state: State<SharedState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<BrainChatRequest>,
) -> (StatusCode, Json<Value>) {
    if req.role.is_empty() || req.message.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "role and message are required" })),
        );
    }

    let Some(persona) = load_persona(&req.role) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": format!(
                    "no YAML persona found for role '{}'. Backfill hex-cli/assets/agents/hex/hex/{}.yml.",
                    req.role, req.role
                ),
            })),
        );
    };

    let mut system_lines = vec![
        format!("ROLE: {}", if persona.name.is_empty() { req.role.clone() } else { persona.name.clone() }),
    ];
    if !persona.description.is_empty() {
        system_lines.push(persona.description.trim().to_string());
    }
    let constraints = constraints_as_strings(&persona.constraints);
    if !constraints.is_empty() {
        system_lines.push("\nCONSTRAINTS:".to_string());
        for c in &constraints {
            system_lines.push(format!("- {}", c));
        }
    }
    let system_prompt = system_lines.join("\n");

    let model_id = persona
        .model
        .preferred
        .as_deref()
        .map(resolve_model_id)
        .unwrap_or_else(|| "openrouter/free".to_string());

    let inference_req = InferenceCompleteRequest {
        model: Some(model_id.clone()),
        messages: vec![json!({ "role": "user", "content": req.message })],
        system: Some(system_prompt),
        max_tokens: 4096,
        tools: None,
    };

    let (status, resp) = inference_complete(state, headers, Json(inference_req)).await;
    if status != StatusCode::OK {
        return (status, resp);
    }

    let content = resp.0
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let final_model = resp.0
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or(model_id);

    (
        StatusCode::OK,
        Json(json!({
            "role": req.role,
            "model": final_model,
            "content": content,
        })),
    )
}
