//! Typed tool library (ADR-2026-05-08-2500).
//!
//! Provides typed primitives the LLM can compose deterministically via
//! Anthropic function-calling. Each tool wraps an existing hex capability
//! (cargo, ripgrep, ADR write, inbox) behind a typed schema. The
//! `ToolRegistry` exports the schema set the inference path attaches to
//! Phase 3 REASON calls.
//!
//! Add a new tool: implement `Tool` + register in `ToolRegistry::default()`.

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

pub mod adr_draft;
pub mod adr_status_set;
pub mod cargo_check;
pub mod code_patch;
pub mod cost_meter;
pub mod dep_audit;
pub mod escalate_to_operator;
pub mod repo_grep;
pub mod repo_read;
pub mod secret_scan;
pub mod spec_draft;
pub mod tool_register;
pub mod typescript_check;
pub mod web_search;
pub mod workplan_emit;
pub mod module_register;
pub mod workspace_boundary_check;

/// Output envelope for every tool call. JSON shape preserved across all
/// tools so the SOP executor can handle errors uniformly without per-tool
/// downcasting.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolResult {
    pub ok: bool,
    pub output: Value,
    pub error: Option<String>,
    pub elapsed_ms: u64,
    /// True when output was truncated due to size cap.
    pub truncated: bool,
}

impl ToolResult {
    pub fn ok(output: Value, elapsed_ms: u64) -> Self {
        Self { ok: true, output, error: None, elapsed_ms, truncated: false }
    }
    pub fn ok_truncated(output: Value, elapsed_ms: u64) -> Self {
        Self { ok: true, output, error: None, elapsed_ms, truncated: true }
    }
    pub fn err(error: impl Into<String>, elapsed_ms: u64) -> Self {
        Self {
            ok: false,
            output: Value::Null,
            error: Some(error.into()),
            elapsed_ms,
            truncated: false,
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// JSON schema for `input_schema` field of an Anthropic function-calling tool.
    /// Must be a JSON object with `type: "object"`, `properties: {...}`, `required: [...]`.
    fn input_schema(&self) -> Value;
    async fn execute(&self, input: Value) -> ToolResult;
}

pub struct ToolRegistry {
    tools: HashMap<&'static str, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: HashMap::new() }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name(), tool);
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.tools.keys().copied().collect()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Build the `tools` array Anthropic expects in a /messages request.
    /// Each entry: { name, description, input_schema }.
    pub fn anthropic_schema(&self) -> Value {
        let arr: Vec<Value> = self
            .tools
            .values()
            .map(|t| {
                serde_json::json!({
                    "name": t.name(),
                    "description": t.description(),
                    "input_schema": t.input_schema(),
                })
            })
            .collect();
        Value::Array(arr)
    }

    pub async fn execute(&self, name: &str, input: Value) -> ToolResult {
        let start = Instant::now();
        let tool = match self.get(name) {
            Some(t) => t,
            None => {
                return ToolResult::err(
                    format!("unknown tool: {}", name),
                    start.elapsed().as_millis() as u64,
                );
            }
        };
        tool.execute(input).await
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        let mut reg = Self::new();
        reg.register(Arc::new(cargo_check::CargoCheck));
        reg.register(Arc::new(dep_audit::DepAudit));
        reg.register(Arc::new(repo_grep::RepoGrep));
        reg.register(Arc::new(repo_read::RepoRead));
        reg.register(Arc::new(secret_scan::SecretScan));
        reg.register(Arc::new(web_search::WebSearch));
        reg.register(Arc::new(adr_draft::AdrDraft));
        reg.register(Arc::new(spec_draft::SpecDraft));
        reg.register(Arc::new(code_patch::CodePatch));
        reg.register(Arc::new(cost_meter::CostMeter));
        reg.register(Arc::new(workplan_emit::WorkplanEmit));
        reg.register(Arc::new(adr_status_set::AdrStatusSet));
        reg.register(Arc::new(tool_register::ToolRegister));
        reg.register(Arc::new(module_register::ModuleRegister));
        reg.register(Arc::new(workspace_boundary_check::WorkspaceBoundaryCheck));
        reg.register(Arc::new(escalate_to_operator::EscalateToOperator));
        reg.register(Arc::new(typescript_check::TypescriptCheck));
        reg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_first_wave() {
        let r = ToolRegistry::default();
        let names = r.names();
        assert!(names.contains(&"cargo_check"), "cargo_check missing");
        assert!(names.contains(&"repo_grep"), "repo_grep missing");
        assert!(names.contains(&"repo_read"), "repo_read missing");
        assert!(names.contains(&"web_search"), "web_search missing");
        assert!(names.contains(&"adr_draft"), "adr_draft missing");
        assert!(names.contains(&"spec_draft"), "spec_draft missing");
        assert!(names.contains(&"escalate_to_operator"), "escalate_to_operator missing");
    }

    #[test]
    fn anthropic_schema_shape() {
        let r = ToolRegistry::default();
        let s = r.anthropic_schema();
        let arr = s.as_array().expect("must be array");
        assert!(!arr.is_empty(), "schema array empty");
        for entry in arr {
            assert!(entry.get("name").is_some(), "missing name");
            assert!(entry.get("description").is_some(), "missing description");
            let schema = entry.get("input_schema").expect("missing input_schema");
            assert_eq!(
                schema.get("type").and_then(|v| v.as_str()),
                Some("object"),
                "input_schema must be object type"
            );
        }
    }
}
