use serde::{Deserialize, Serialize};

/// Definition of a tool available to the agent — sent to LLM APIs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: ToolInputSchema,
}

/// JSON Schema for tool input parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInputSchema {
    #[serde(rename = "type")]
    pub schema_type: String,
    #[serde(default)]
    pub properties: serde_json::Value,
    #[serde(default)]
    pub required: Vec<String>,
}

/// A tool call from the assistant — extracted from tool_use content blocks.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// Result of executing a tool — fed back as tool_result content block.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
}
