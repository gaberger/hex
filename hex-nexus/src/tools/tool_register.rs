//! `tool_register` — meta-tool that scaffolds new typed tools.
//!
//! Lets a persona create a new tool from a one-line spec by emitting three
//! `code_patch` operations internally:
//!   1. CREATE hex-nexus/src/tools/<tool_name>.rs with stub Tool impl
//!   2. UPDATE mod.rs adding `pub mod <tool_name>;`
//!   3. UPDATE mod.rs adding registration line in `ToolRegistry::default()`
//!
//! Input:
//!   - tool_name: lowercase_with_underscores (^[a-z][a-z0-9_]*$)
//!   - description: one-line summary for LLM function-calling
//!   - behavior_summary: what the tool should do (used in stub comment)
//!
//! Output:
//!   - tool_name: confirmed
//!   - files_emitted: paths of the three patched files
//!   - next_steps: reminder to implement the stub

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::sync::LazyLock;
use std::time::Instant;

use super::{code_patch::CodePatch, Tool, ToolResult};

static TOOL_NAME_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9_]*$").unwrap());

const STUB_TEMPLATE: &str = r#"//! `{tool_name}` — {description}
//!
//! {behavior_summary}

use async_trait::async_trait;
use serde_json::{{json, Value}};
use std::time::Instant;

use super::{{Tool, ToolResult}};

pub struct {struct_name};

#[async_trait]
impl Tool for {struct_name} {{
    fn name(&self) -> &'static str {{
        "{tool_name}"
    }}
    fn description(&self) -> &'static str {{
        "{description}"
    }}
    fn input_schema(&self) -> Value {{
        json!({{
            "type": "object",
            "properties": {{}},
            "required": []
        }})
    }}
    async fn execute(&self, _input: Value) -> ToolResult {{
        let start = Instant::now();
        ToolResult::err(
            "not yet implemented — replace this stub in hex-nexus/src/tools/{tool_name}.rs",
            start.elapsed().as_millis() as u64,
        )
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;
    #[test]
    fn schema_shape() {{
        let s = {struct_name}.input_schema();
        assert_eq!(s.get("type").and_then(|v| v.as_str()), Some("object"));
    }}
}}
"#;

pub struct ToolRegister;

#[async_trait]
impl Tool for ToolRegister {
    fn name(&self) -> &'static str {
        "tool_register"
    }
    fn description(&self) -> &'static str {
        "Scaffold a new typed tool via code_patch. Input: tool_name \
         (lowercase_with_underscores), description, behavior_summary. \
         Output: tool_name + files_emitted + next_steps. Validates name \
         regex ^[a-z][a-z0-9_]*$, checks not already registered, then \
         internally emits three code_patch ops: (a) CREATE tool stub .rs, \
         (b) UPDATE mod.rs pub mod line, (c) UPDATE mod.rs registration."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tool_name": {
                    "type": "string",
                    "description": "Tool name, lowercase_with_underscores (^[a-z][a-z0-9_]*$)"
                },
                "description": {
                    "type": "string",
                    "description": "One-line summary for LLM function-calling schema"
                },
                "behavior_summary": {
                    "type": "string",
                    "description": "What the tool should do (used in stub header comment)"
                }
            },
            "required": ["tool_name", "description", "behavior_summary"]
        })
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let start = Instant::now();
        
        let tool_name = match input.get("tool_name").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return ToolResult::err("missing tool_name", start.elapsed().as_millis() as u64),
        };
        let description = match input.get("description").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() && s.len() <= 200 => s.to_string(),
            _ => return ToolResult::err("description required, 1-200 chars", start.elapsed().as_millis() as u64),
        };
        let behavior_summary = match input.get("behavior_summary").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() && s.len() <= 500 => s.to_string(),
            _ => return ToolResult::err("behavior_summary required, 1-500 chars", start.elapsed().as_millis() as u64),
        };

        // Validate tool_name regex
        if !TOOL_NAME_REGEX.is_match(&tool_name) {
            return ToolResult::err(
                format!("tool_name '{}' must match ^[a-z][a-z0-9_]*$", tool_name),
                start.elapsed().as_millis() as u64,
            );
        }

        // Check not already registered by looking at mod.rs
        let repo_root = std::env::var("HEX_REPO_ROOT")
            .unwrap_or_else(|_| "/home/jack/hex-intf".to_string());
        let mod_path = std::path::Path::new(&repo_root).join("hex-nexus/src/tools/mod.rs");
        let mod_content = match std::fs::read_to_string(&mod_path) {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("read mod.rs: {}", e), start.elapsed().as_millis() as u64),
        };
        let mod_decl = format!("pub mod {};", tool_name);
        if mod_content.contains(&mod_decl) {
            return ToolResult::err(
                format!("tool '{}' already declared in mod.rs", tool_name),
                start.elapsed().as_millis() as u64,
            );
        }

        // Build stub content
        let struct_name = tool_name
            .split('_')
            .map(|part| {
                let mut c = part.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            })
            .collect::<String>();
        let stub = STUB_TEMPLATE
            .replace("{tool_name}", &tool_name)
            .replace("{struct_name}", &struct_name)
            .replace("{description}", &description)
            .replace("{behavior_summary}", &behavior_summary);

        if stub.len() > 2048 {
            return ToolResult::err("generated stub exceeds 2KB cap", start.elapsed().as_millis() as u64);
        }

        let code_patch = CodePatch;

        // (a) CREATE tool stub
        let create_result = code_patch
            .execute(json!({
                "path": format!("hex-nexus/src/tools/{}.rs", tool_name),
                "mode": "create",
                "new_content": stub,
                "rationale": format!("tool_register: scaffold {}", tool_name),
            }))
            .await;
        if !create_result.ok {
            return ToolResult::err(
                format!("CREATE stub failed: {}", create_result.error.unwrap_or_default()),
                start.elapsed().as_millis() as u64,
            );
        }

        // (b) UPDATE mod.rs: add pub mod line after last existing pub mod
        let last_pub_mod_line = mod_content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.trim_start().starts_with("pub mod "))
            .map(|(i, _)| i)
            .last();
        
        let mod_patch_result = if let Some(last_idx) = last_pub_mod_line {
            let lines: Vec<&str> = mod_content.lines().collect();
            let anchor = lines[last_idx];
            code_patch
                .execute(json!({
                    "path": "hex-nexus/src/tools/mod.rs",
                    "mode": "replace_string",
                    "find_string": format!("{}\n", anchor),
                    "new_content": format!("{}\npub mod {};\n", anchor, tool_name),
                    "rationale": format!("tool_register: add pub mod {}", tool_name),
                }))
                .await
        } else {
            return ToolResult::err("no pub mod lines found in mod.rs", start.elapsed().as_millis() as u64);
        };
        if !mod_patch_result.ok {
            return ToolResult::err(
                format!("UPDATE mod.rs (pub mod) failed: {}", mod_patch_result.error.unwrap_or_default()),
                start.elapsed().as_millis() as u64,
            );
        }

        // (c) UPDATE mod.rs: add registration line in ToolRegistry::default()
        // Find the last `reg.register(Arc::new(...));` line
        let last_register_line = mod_content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.trim_start().starts_with("reg.register(Arc::new("))
            .map(|(i, _)| i)
            .last();

        let reg_patch_result = if let Some(last_idx) = last_register_line {
            let lines: Vec<&str> = mod_content.lines().collect();
            let anchor = lines[last_idx];
            let new_reg = format!("        reg.register(Arc::new({}::{}));", tool_name, struct_name);
            code_patch
                .execute(json!({
                    "path": "hex-nexus/src/tools/mod.rs",
                    "mode": "replace_string",
                    "find_string": format!("{}\n", anchor),
                    "new_content": format!("{}\n{}\n", anchor, new_reg),
                    "rationale": format!("tool_register: register {}", tool_name),
                }))
                .await
        } else {
            return ToolResult::err("no reg.register lines found in ToolRegistry::default()", start.elapsed().as_millis() as u64);
        };
        if !reg_patch_result.ok {
            return ToolResult::err(
                format!("UPDATE mod.rs (register) failed: {}", reg_patch_result.error.unwrap_or_default()),
                start.elapsed().as_millis() as u64,
            );
        }

        ToolResult::ok(
            json!({
                "ok": true,
                "tool_name": tool_name,
                "files_emitted": [
                    format!("hex-nexus/src/tools/{}.rs", tool_name),
                    "hex-nexus/src/tools/mod.rs",
                ],
                "next_steps": format!(
                    "Stub created. Open hex-nexus/src/tools/{}.rs and replace the execute() body with real logic. \
                     Update input_schema() to match your tool's parameters.",
                    tool_name
                ),
            }),
            start.elapsed().as_millis() as u64,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_requires_three_fields() {
        let s = ToolRegister.input_schema();
        let req = s.get("required").and_then(|v| v.as_array()).unwrap();
        assert_eq!(req.len(), 3);
        assert!(req.iter().any(|v| v.as_str() == Some("tool_name")));
        assert!(req.iter().any(|v| v.as_str() == Some("description")));
        assert!(req.iter().any(|v| v.as_str() == Some("behavior_summary")));
    }
}
