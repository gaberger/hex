//! `module_register` — eliminate mod.rs registration tax
//!
//! Meta-tool that automates the two-step registration pattern every
//! multi-file SOP build requires: (a) pub mod line, (b) optional registration
//! in ToolRegistry::default() for tools or just pub mod for adapters.
//!
//! Input:
//!   - path: String (relative to repo root, e.g. hex-nexus/src/tools/foo.rs)
//!   - struct_name?: String (PascalCase, defaults to capitalized snake-to-pascal of file basename)
//!   - kind?: 'tool'|'adapter' (defaults: inferred from path)
//!
//! Output:
//!   - registered: bool (false if already present)
//!   - mod_rs_path: String
//!   - pub_mod_line: String
//!   - registration_line: Option<String>
//!   - note?: String (when idempotent skip)

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::sync::LazyLock;
use std::time::Instant;

use super::{code_patch::CodePatch, Tool, ToolResult};

static BASENAME_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9_]*$").unwrap());

pub struct ModuleRegister;

#[async_trait]
impl Tool for ModuleRegister {
    fn name(&self) -> &'static str {
        "module_register"
    }
    fn description(&self) -> &'static str {
        "Automate mod.rs registration for new tools or adapters. Input: path \
         (relative to repo root), optional struct_name (PascalCase), optional \
         kind ('tool'|'adapter', inferred from path). Output: registered (bool), \
         mod_rs_path, pub_mod_line, registration_line. Idempotent: skips if \
         already present. Emits code_patch ops internally."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative to repo root, e.g. hex-nexus/src/tools/foo.rs"
                },
                "struct_name": {
                    "type": "string",
                    "description": "Optional PascalCase struct name; defaults to snake_to_pascal of basename"
                },
                "kind": {
                    "type": "string",
                    "enum": ["tool", "adapter"],
                    "description": "Optional; inferred from path if omitted"
                }
            },
            "required": ["path"]
        })
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let start = Instant::now();

        // Parse inputs
        let path = match input.get("path").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("missing or empty path", start.elapsed().as_millis() as u64),
        };

        if !path.ends_with(".rs") {
            return ToolResult::err("path must end with .rs", start.elapsed().as_millis() as u64);
        }

        // Derive basename (filename without .rs)
        let basename = match path.rsplit('/').next() {
            Some(fname) if fname.ends_with(".rs") => &fname[..fname.len() - 3],
            _ => return ToolResult::err("invalid path format", start.elapsed().as_millis() as u64),
        };

        if !BASENAME_REGEX.is_match(basename) {
            return ToolResult::err(
                format!("basename '{}' must match ^[a-z][a-z0-9_]*$", basename),
                start.elapsed().as_millis() as u64,
            );
        }

        // Infer kind from path
        let kind = match input.get("kind").and_then(|v| v.as_str()) {
            Some("tool") => "tool",
            Some("adapter") => "adapter",
            Some(other) => {
                return ToolResult::err(
                    format!("kind must be 'tool' or 'adapter', got '{}'", other),
                    start.elapsed().as_millis() as u64,
                )
            }
            None => {
                if path.contains("/tools/") {
                    "tool"
                } else if path.contains("/adapters/") {
                    "adapter"
                } else {
                    return ToolResult::err(
                        "cannot infer kind; path must contain /tools/ or /adapters/, or specify kind explicitly",
                        start.elapsed().as_millis() as u64,
                    );
                }
            }
        };

        // Build struct_name (PascalCase from basename)
        let struct_name = match input.get("struct_name").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => basename
                .split('_')
                .map(|part| {
                    let mut c = part.chars();
                    match c.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    }
                })
                .collect::<String>(),
        };

        // Determine mod.rs path
        let mod_rs_path = if kind == "tool" {
            "hex-nexus/src/tools/mod.rs"
        } else {
            "hex-nexus/src/adapters/mod.rs"
        };

        // Validate that the target file exists
        let repo_root = std::env::var("HEX_REPO_ROOT")
            .unwrap_or_else(|_| "/home/user/hex-intf".to_string());
        let target_file = std::path::Path::new(&repo_root).join(path);
        if !target_file.exists() {
            return ToolResult::err(
                format!("target file does not exist: {}", path),
                start.elapsed().as_millis() as u64,
            );
        }

        // Read mod.rs
        let mod_rs_full = std::path::Path::new(&repo_root).join(mod_rs_path);
        let mod_content = match std::fs::read_to_string(&mod_rs_full) {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::err(
                    format!("read {}: {}", mod_rs_path, e),
                    start.elapsed().as_millis() as u64,
                )
            }
        };

        let pub_mod_line = format!("pub mod {};", basename);

        // Check idempotency: already registered?
        if mod_content.contains(&pub_mod_line) {
            return ToolResult::ok(
                json!({
                    "registered": false,
                    "mod_rs_path": mod_rs_path,
                    "pub_mod_line": pub_mod_line,
                    "registration_line": null,
                    "note": format!("already registered: {} present in {}", pub_mod_line, mod_rs_path),
                }),
                start.elapsed().as_millis() as u64,
            );
        }

        let code_patch = CodePatch;

        // (a) Add pub mod line alphabetically
        // Find the last pub mod line for alphabetical insertion
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
                    "path": mod_rs_path,
                    "mode": "replace_string",
                    "find_string": format!("{}\n", anchor),
                    "new_content": format!("{}\n{}\n", anchor, pub_mod_line),
                    "rationale": format!("module_register: add pub mod {}", basename),
                }))
                .await
        } else {
            return ToolResult::err(
                format!("no pub mod lines found in {}", mod_rs_path),
                start.elapsed().as_millis() as u64,
            );
        };

        if !mod_patch_result.ok {
            return ToolResult::err(
                format!(
                    "UPDATE {} (pub mod) failed: {}",
                    mod_rs_path,
                    mod_patch_result.error.unwrap_or_default()
                ),
                start.elapsed().as_millis() as u64,
            );
        }

        // (b) If kind=tool, add registration line in ToolRegistry::default()
        let registration_line = if kind == "tool" {
            let reg_line = format!("        reg.register(Arc::new({}::{}));", basename, struct_name);

            let last_register_line = mod_content
                .lines()
                .enumerate()
                .filter(|(_, line)| line.trim_start().starts_with("reg.register(Arc::new("))
                .map(|(i, _)| i)
                .last();

            let reg_patch_result = if let Some(last_idx) = last_register_line {
                let lines: Vec<&str> = mod_content.lines().collect();
                let anchor = lines[last_idx];
                code_patch
                    .execute(json!({
                        "path": mod_rs_path,
                        "mode": "replace_string",
                        "find_string": format!("{}\n", anchor),
                        "new_content": format!("{}\n{}\n", anchor, reg_line),
                        "rationale": format!("module_register: register {}", basename),
                    }))
                    .await
            } else {
                return ToolResult::err(
                    "no reg.register lines found in ToolRegistry::default()",
                    start.elapsed().as_millis() as u64,
                );
            };

            if !reg_patch_result.ok {
                return ToolResult::err(
                    format!(
                        "UPDATE {} (register) failed: {}",
                        mod_rs_path,
                        reg_patch_result.error.unwrap_or_default()
                    ),
                    start.elapsed().as_millis() as u64,
                );
            }

            Some(reg_line)
        } else {
            None
        };

        ToolResult::ok(
            json!({
                "registered": true,
                "mod_rs_path": mod_rs_path,
                "pub_mod_line": pub_mod_line,
                "registration_line": registration_line,
            }),
            start.elapsed().as_millis() as u64,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_requires_path() {
        let s = ModuleRegister.input_schema();
        let req = s.get("required").and_then(|v| v.as_array()).unwrap();
        assert_eq!(req.len(), 1);
        assert!(req.iter().any(|v| v.as_str() == Some("path")));
    }
}
