//! `workplan_emit` — typed primitive that writes a hex workplan JSON.
//!
//! Closes the ADR → workplan auto-bridge gap. A persona that authored an
//! ADR (or the auto-emitter background task) can call this to produce a
//! schema-conformant workplan in `docs/workplans/wp-<slug>.json`. Output
//! flows through proposed_action(file_write); twin auto-approves
//! `proposed_by="tool:workplan_emit"`; executor lands the file; existing
//! `hex plan list` / `hex plan reconcile` immediately see it.
//!
//! Schema enforced (matches `hex plan schema`):
//!   id (^wp-), feature, adr (^ADR-), status, phases[]
//!   each phase: id (^P\d+$), name, tier 0..5, tasks[]
//!   each task: id, name, layer

use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

use super::{Tool, ToolResult};

const STDB_HOST_DEFAULT: &str = "http://127.0.0.1:3033";
const MAX_BODY: usize = 24_000;

pub struct WorkplanEmit;

#[async_trait]
impl Tool for WorkplanEmit {
    fn name(&self) -> &'static str {
        "workplan_emit"
    }
    fn description(&self) -> &'static str {
        "Emit a hex workplan JSON to docs/workplans/wp-<slug>.json. \
         Use this immediately after authoring an ADR with implementation \
         work to dispatch — turns the ADR's Decision section into a \
         schema-conformant workplan that hex swarm/task can consume. \
         Phases are dependency-ordered; tasks within a phase run parallel."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "slug": {
                    "type": "string",
                    "description": "kebab-case slug, becomes wp-<slug>.json (e.g. 'stdb-payload-cap')"
                },
                "feature": {
                    "type": "string",
                    "description": "Human-readable feature name shown in `hex plan list`"
                },
                "adr": {
                    "type": "string",
                    "description": "Source ADR reference (e.g. 'ADR-2605082600'). Required by ADR→workplan→swarm pipeline."
                },
                "phases": {
                    "type": "array",
                    "description": "Dependency-ordered phases. Each phase has id (P0/P1/...), name, tier (0-5), and tasks[]",
                    "items": {
                        "type": "object",
                        "required": ["id", "name", "tasks"],
                        "properties": {
                            "id":   { "type": "string", "description": "Phase id, e.g. P0" },
                            "name": { "type": "string", "description": "What this phase delivers" },
                            "tier": { "type": "integer", "minimum": 0, "maximum": 5,
                                      "description": "0=domain/ports, 1=secondary, 2=primary, 3=usecases, 4=integration" },
                            "tasks": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "required": ["id", "name", "layer", "files"],
                                    "properties": {
                                        "id":    { "type": "string", "description": "Task id, e.g. P0.1" },
                                        "name":  { "type": "string", "description": "Concrete deliverable" },
                                        "layer": { "type": "string", "enum": ["domain","ports","usecases","primary","secondary","infrastructure","integration"] },
                                        "files": {
                                            "type": "array",
                                            "items": { "type": "string" },
                                            "description": "Repo-relative file paths this task creates or modifies. Required by ADR-2604142200 for hex plan reconcile to verify done-condition. Use forward slashes; no globs."
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            },
            "required": ["slug", "feature", "adr", "phases"]
        })
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let start = Instant::now();
        let slug_raw = match input.get("slug").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return ToolResult::err("missing slug", start.elapsed().as_millis() as u64),
        };
        let slug = sanitise_slug(&slug_raw);
        if slug.is_empty() || slug.len() > 80 {
            return ToolResult::err("slug after sanitisation must be 1-80 chars", start.elapsed().as_millis() as u64);
        }
        let feature = match input.get("feature").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() && s.len() <= 200 => s.to_string(),
            _ => return ToolResult::err("missing feature (1-200 chars)", start.elapsed().as_millis() as u64),
        };
        let adr = match input.get("adr").and_then(|v| v.as_str()) {
            Some(s) if s.starts_with("ADR-") => s.to_string(),
            _ => return ToolResult::err("adr must start with ADR-", start.elapsed().as_millis() as u64),
        };
        let phases_in = match input.get("phases").and_then(|v| v.as_array()) {
            Some(a) if !a.is_empty() => a.clone(),
            _ => return ToolResult::err("phases array required, non-empty", start.elapsed().as_millis() as u64),
        };

        // Validate each phase + tasks; build the on-disk JSON.
        let mut phases_out: Vec<Value> = Vec::new();
        for (pi, p) in phases_in.iter().enumerate() {
            let pid = p.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if !is_valid_phase_id(pid) {
                return ToolResult::err(
                    format!("phase[{}].id '{}' must match ^P\\d+$", pi, pid),
                    start.elapsed().as_millis() as u64,
                );
            }
            let pname = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if pname.is_empty() {
                return ToolResult::err(format!("phase[{}].name required", pi), start.elapsed().as_millis() as u64);
            }
            let tier = p.get("tier").and_then(|v| v.as_u64()).unwrap_or(0);
            if tier > 5 {
                return ToolResult::err(format!("phase[{}].tier {} > 5", pi, tier), start.elapsed().as_millis() as u64);
            }
            let tasks_in = match p.get("tasks").and_then(|v| v.as_array()) {
                Some(a) if !a.is_empty() => a,
                _ => return ToolResult::err(format!("phase[{}].tasks required, non-empty", pi), start.elapsed().as_millis() as u64),
            };
            let mut tasks_out: Vec<Value> = Vec::new();
            for (ti, t) in tasks_in.iter().enumerate() {
                let tid = t.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let tname = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let tlayer = t.get("layer").and_then(|v| v.as_str()).unwrap_or("");
                let tfiles: Vec<String> = t.get("files")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();
                if tid.is_empty() || tname.is_empty() || tlayer.is_empty() {
                    return ToolResult::err(
                        format!("phase[{}].tasks[{}] requires id+name+layer", pi, ti),
                        start.elapsed().as_millis() as u64,
                    );
                }
                if tfiles.is_empty() {
                    return ToolResult::err(
                        format!("phase[{}].tasks[{}] '{}' requires files[] (ADR-2604142200 reconcile evidence)", pi, ti, tid),
                        start.elapsed().as_millis() as u64,
                    );
                }
                tasks_out.push(json!({
                    "id":     tid,
                    "name":   tname,
                    "layer":  tlayer,
                    "files":  tfiles,
                    "status": "pending",
                }));
            }
            phases_out.push(json!({
                "id":     pid,
                "name":   pname,
                "tier":   tier,
                "tasks":  tasks_out,
                "status": "pending",
            }));
        }

        let workplan = json!({
            "id":         format!("wp-{}", slug),
            "feature":    feature,
            "adr":        adr,
            "status":     "planned",
            "priority":   "normal",
            "created_at": chrono::Utc::now().to_rfc3339(),
            "created_by": "hex-coder",
            "phases":     phases_out,
        });

        let pretty = match serde_json::to_string_pretty(&workplan) {
            Ok(s) => s,
            Err(e) => return ToolResult::err(format!("json serialise: {}", e), start.elapsed().as_millis() as u64),
        };
        if pretty.len() > MAX_BODY {
            return ToolResult::err(
                format!("workplan size {} > cap {} (BSATN limit)", pretty.len(), MAX_BODY),
                start.elapsed().as_millis() as u64,
            );
        }

        let target_path = format!("docs/workplans/wp-{}.json", slug);
        let payload = serde_json::json!({
            "path": target_path,
            "content": pretty,
        });

        let host = std::env::var("HEX_SPACETIMEDB_HOST").unwrap_or_else(|_| STDB_HOST_DEFAULT.to_string());
        let db = std::env::var("HEX_STDB_DATABASE")
            .unwrap_or_else(|_| hex_core::stdb_database_for_module("hexflo-coordination").to_string());
        let url = format!("{}/v1/database/{}/call/proposed_action_open", host, db);
        let http = match reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("http: {}", e), start.elapsed().as_millis() as u64),
        };
        let body_call = serde_json::json!([
            "file_write",
            payload.to_string(),
            "tool:workplan_emit",
            0u64,
        ]);
        let resp = match http.post(&url).json(&body_call).send().await {
            Ok(r) => r,
            Err(e) => return ToolResult::err(format!("stdb: {}", e), start.elapsed().as_millis() as u64),
        };
        if !resp.status().is_success() {
            let s = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return ToolResult::err(format!("proposed_action_open HTTP {}: {}", s, body), start.elapsed().as_millis() as u64);
        }
        ToolResult::ok(
            json!({
                "ok": true,
                "target_path": target_path,
                "workplan_id": format!("wp-{}", slug),
                "adr": adr,
                "phases": phases_out.len(),
                "byte_len": pretty.len(),
                "note": "proposed_action queued; twin auto-approves tool:* per ADR-2605082500; once executor writes, run `hex swarm init wp-<slug>` to dispatch",
            }),
            start.elapsed().as_millis() as u64,
        )
    }
}

fn is_valid_phase_id(s: &str) -> bool {
    s.starts_with('P') && s.len() > 1 && s[1..].chars().all(|c| c.is_ascii_digit())
}

fn sanitise_slug(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phase_id_validation() {
        assert!(is_valid_phase_id("P0"));
        assert!(is_valid_phase_id("P12"));
        assert!(!is_valid_phase_id("p0"));
        assert!(!is_valid_phase_id("P"));
        assert!(!is_valid_phase_id("Pa"));
    }
    #[test]
    fn schema_requires_4_fields() {
        let s = WorkplanEmit.input_schema();
        let req: Vec<String> = s.get("required").unwrap().as_array().unwrap()
            .iter().map(|v| v.as_str().unwrap().to_string()).collect();
        for f in ["slug", "feature", "adr", "phases"] {
            assert!(req.contains(&f.to_string()), "missing required: {}", f);
        }
    }
}
