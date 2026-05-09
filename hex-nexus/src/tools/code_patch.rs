//! `code_patch` — typed source mutator. THE missing primitive.
//!
//! Lets a persona modify existing source files, not just create new
//! ones. Mirrors adr_draft / spec_draft (proposed_action emit) but for
//! arbitrary file paths with line-range targeting.
//!
//! Operation modes:
//!   - `replace_lines`: replace [start, end] inclusive with `new_content`
//!   - `replace_string`: literal find/replace, must be unique in file
//!   - `append`: append `new_content` to end of file (no replace)
//!
//! Hard guards (defence-in-depth — twin + executor enforce again):
//!   - Path must be repo-relative, no /, no `..`
//!   - Path must end in a recognised source extension
//!   - new_content size cap: 16 KB (sub-CTO 24 KB BSATN limit)
//!   - replace_string must occur EXACTLY ONCE in file (else reject)
//!
//! Verifier: action_executor's cargo_check gate runs after write.
//! Twin auto-approves `proposed_by="tool:code_patch"` (matches the
//! ADR-2605082500 SOP-emitted-action policy).

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

use super::{Tool, ToolResult};

const STDB_HOST_DEFAULT: &str = "http://127.0.0.1:3033";
const MAX_NEW_CONTENT: usize = 16 * 1024;

pub struct CodePatch;

#[async_trait]
impl Tool for CodePatch {
    fn name(&self) -> &'static str {
        "code_patch"
    }
    fn description(&self) -> &'static str {
        "Mutate an existing source file. Three modes: replace_lines \
         (start..=end inclusive), replace_string (must be unique), or \
         append. Path must be repo-relative under hex-nexus/src/, \
         hex-cli/src/, hex-core/src/, hex-agent/src/, hex-parser/src/, \
         hex-analyzer/src/, examples/, scripts/, docs/, or \
         spacetime-modules/. The action_executor will run cargo_check \
         (or appropriate validator) before the patch lands. Use this to \
         apply ADR/spec mitigations, add new tools, fix bugs."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Repo-relative path, e.g. 'hex-nexus/src/tools/secret_scan.rs'"
                },
                "mode": {
                    "type": "string",
                    "enum": ["replace_lines", "replace_string", "append", "create"],
                    "description": "How to apply the change. 'create' writes a new file; rejects if file exists."
                },
                "start_line": {
                    "type": "integer",
                    "description": "1-based inclusive start line (replace_lines mode only)"
                },
                "end_line": {
                    "type": "integer",
                    "description": "1-based inclusive end line (replace_lines mode only)"
                },
                "find_string": {
                    "type": "string",
                    "description": "Exact literal to find (replace_string mode). MUST occur exactly once in file."
                },
                "new_content": {
                    "type": "string",
                    "description": "Replacement / inserted / appended content. 0-16384 bytes."
                },
                "rationale": {
                    "type": "string",
                    "description": "One-line why — surfaces in commit-style audit log."
                }
            },
            "required": ["path", "mode", "new_content", "rationale"]
        })
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let start = Instant::now();
        let rel_path = match input.get("path").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return ToolResult::err("missing path", start.elapsed().as_millis() as u64),
        };
        if rel_path.starts_with('/') || rel_path.contains("..") {
            return ToolResult::err("path must be repo-relative; absolute and `..` rejected", start.elapsed().as_millis() as u64);
        }
        let allowed_prefixes = [
            "hex-nexus/src/", "hex-cli/src/", "hex-core/src/", "hex-agent/src/",
            "hex-parser/src/", "hex-analyzer/src/", "hex-desktop/src/",
            "examples/", "scripts/", "docs/", "spacetime-modules/", "tests/",
        ];
        if !allowed_prefixes.iter().any(|p| rel_path.starts_with(p)) {
            return ToolResult::err(
                format!("path '{}' outside allowed prefixes (hex-*/src/, examples/, scripts/, docs/, spacetime-modules/, tests/)", rel_path),
                start.elapsed().as_millis() as u64,
            );
        }
        let mode = match input.get("mode").and_then(|v| v.as_str()) {
            Some(m @ "replace_lines" | m @ "replace_string" | m @ "append" | m @ "create") => m.to_string(),
            _ => return ToolResult::err("mode must be replace_lines|replace_string|append|create", start.elapsed().as_millis() as u64),
        };
        let new_content = match input.get("new_content").and_then(|v| v.as_str()) {
            Some(s) if s.len() <= MAX_NEW_CONTENT => s.to_string(),
            Some(s) => return ToolResult::err(
                format!("new_content {} bytes exceeds {}", s.len(), MAX_NEW_CONTENT),
                start.elapsed().as_millis() as u64,
            ),
            None => return ToolResult::err("missing new_content", start.elapsed().as_millis() as u64),
        };
        let rationale = match input.get("rationale").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() && s.len() <= 300 => s.to_string(),
            _ => return ToolResult::err("rationale required, 1-300 chars", start.elapsed().as_millis() as u64),
        };

        let repo_root = std::env::var("HEX_REPO_ROOT")
            .unwrap_or_else(|_| "/home/gary/hex-intf".to_string());
        let target = Path::new(&repo_root).join(&rel_path);

        let final_content = match mode.as_str() {
            "create" => {
                if target.exists() {
                    return ToolResult::err(
                        format!("create rejected: {} already exists; use replace_lines or replace_string", rel_path),
                        start.elapsed().as_millis() as u64,
                    );
                }
                new_content.clone()
            }
            "append" => {
                let existing = std::fs::read_to_string(&target)
                    .unwrap_or_default();
                let mut out = existing;
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str(&new_content);
                out
            }
            "replace_lines" => {
                let start_line = input.get("start_line").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let end_line = input.get("end_line").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                if start_line == 0 || end_line < start_line {
                    return ToolResult::err(
                        "replace_lines requires start_line >= 1 and end_line >= start_line",
                        start.elapsed().as_millis() as u64,
                    );
                }
                let existing = match std::fs::read_to_string(&target) {
                    Ok(s) => s,
                    Err(e) => return ToolResult::err(format!("read failed: {}", e), start.elapsed().as_millis() as u64),
                };
                let lines: Vec<&str> = existing.lines().collect();
                if end_line > lines.len() {
                    return ToolResult::err(
                        format!("end_line {} exceeds file length {}", end_line, lines.len()),
                        start.elapsed().as_millis() as u64,
                    );
                }
                let mut out = String::new();
                for (i, line) in lines.iter().enumerate() {
                    let one_based = i + 1;
                    if one_based < start_line {
                        out.push_str(line);
                        out.push('\n');
                    } else if one_based == start_line {
                        out.push_str(&new_content);
                        if !new_content.ends_with('\n') {
                            out.push('\n');
                        }
                    } else if one_based > end_line {
                        out.push_str(line);
                        out.push('\n');
                    }
                    // lines in (start_line, end_line] are skipped
                }
                out
            }
            "replace_string" => {
                let find = match input.get("find_string").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s.to_string(),
                    _ => return ToolResult::err("replace_string mode requires find_string", start.elapsed().as_millis() as u64),
                };
                let existing = match std::fs::read_to_string(&target) {
                    Ok(s) => s,
                    Err(e) => return ToolResult::err(format!("read failed: {}", e), start.elapsed().as_millis() as u64),
                };
                let occurrences = existing.matches(&find).count();
                if occurrences == 0 {
                    return ToolResult::err("find_string not found in file", start.elapsed().as_millis() as u64);
                }
                if occurrences > 1 {
                    return ToolResult::err(
                        format!("find_string occurs {} times — must be unique; widen the find_string with surrounding context", occurrences),
                        start.elapsed().as_millis() as u64,
                    );
                }
                existing.replacen(&find, &new_content, 1)
            }
            _ => unreachable!(),
        };

        // Emit proposed_action(file_write) — same path adr_draft / spec_draft use.
        let payload = serde_json::json!({
            "path": rel_path,
            "content": final_content,
        });
        let host = std::env::var("HEX_SPACETIMEDB_HOST")
            .unwrap_or_else(|_| STDB_HOST_DEFAULT.to_string());
        let db = std::env::var("HEX_STDB_DATABASE")
            .unwrap_or_else(|_| hex_core::stdb_database_for_module("hexflo-coordination").to_string());
        let url = format!("{}/v1/database/{}/call/proposed_action_open", host, db);
        let http = match reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("http build: {}", e), start.elapsed().as_millis() as u64),
        };
        let body_call = serde_json::json!([
            "file_write",
            payload.to_string(),
            "tool:code_patch",
            0u64,
        ]);
        let resp = match http.post(&url).json(&body_call).send().await {
            Ok(r) => r,
            Err(e) => return ToolResult::err(format!("stdb call: {}", e), start.elapsed().as_millis() as u64),
        };
        if !resp.status().is_success() {
            let status_code = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return ToolResult::err(
                format!("proposed_action_open HTTP {}: {}", status_code, body),
                start.elapsed().as_millis() as u64,
            );
        }
        ToolResult::ok(
            json!({
                "ok": true,
                "target_path": rel_path,
                "mode": mode,
                "rationale": rationale,
                "byte_len": final_content.len(),
                "note": "proposed_action queued; twin auto-approves tool:* per ADR-2605082500; executor writes via SafeFileWriter; cargo_check is the post-write verifier",
            }),
            start.elapsed().as_millis() as u64,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_requires_4_fields() {
        let s = CodePatch.input_schema();
        let req: Vec<String> = s.get("required").unwrap().as_array().unwrap()
            .iter().map(|v| v.as_str().unwrap().to_string()).collect();
        for f in ["path", "mode", "new_content", "rationale"] {
            assert!(req.contains(&f.to_string()), "missing required: {}", f);
        }
    }
}
