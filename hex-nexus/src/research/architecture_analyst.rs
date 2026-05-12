//! Architecture analyst (workplan `wp-idle-research-swarm`, P2.1).
//!
//! Shells out to `hex analyze . --json` and folds the resulting violation
//! sets into [`Finding`] records with `domain = architecture`. Severity is
//! taken straight from the analyze output where one is provided
//! (`adr_compliance.violations[].severity`); for boundary violations that
//! ship without a severity field we default to `High` because hex layer
//! crossings are by construction architecture-breaking.
//!
//! The IO layer is a thin wrapper around [`std::process::Command`] so the
//! parser ([`parse_findings`]) stays a pure `&serde_json::Value -> Vec<Finding>`
//! function — trivial to unit-test against synthetic fixtures with no `hex`
//! binary or workspace required.

use std::path::Path;
use std::process::Command;

use hex_core::{ActionKind, Domain, Finding, Severity, SuggestedAction};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Errors produced by [`analyze_architecture`].
#[derive(Debug)]
pub enum AnalystError {
    /// Failed to spawn or wait on the `hex analyze` invocation.
    Spawn(std::io::Error),
    /// `hex analyze` produced no parseable JSON object on stdout.
    NoJson { stdout: String, stderr: String },
    /// `hex analyze` produced output that wasn't valid JSON.
    InvalidJson(serde_json::Error),
}

impl std::fmt::Display for AnalystError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalystError::Spawn(e) => write!(f, "failed to spawn `hex analyze . --json`: {e}"),
            AnalystError::NoJson { stderr, .. } => {
                write!(f, "`hex analyze . --json` produced no JSON object on stdout (stderr: {stderr})")
            }
            AnalystError::InvalidJson(e) => write!(f, "`hex analyze . --json` produced invalid JSON: {e}"),
        }
    }
}

impl std::error::Error for AnalystError {}

/// Run the deterministic architecture analyst against `project_root`.
///
/// Spawns `hex analyze . --json` in `project_root` and parses the result.
/// `hex analyze` exits non-zero in `--strict` mode when violations exist;
/// here we ignore status and parse stdout regardless — a non-zero exit with
/// JSON on stdout is the *expected* path when the analyst has work to do.
pub fn analyze_architecture(project_root: &Path) -> Result<Vec<Finding>, AnalystError> {
    let output = Command::new("hex")
        .args(["analyze", ".", "--json"])
        .current_dir(project_root)
        .output()
        .map_err(AnalystError::Spawn)?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    let json_start = stdout
        .find('{')
        .ok_or_else(|| AnalystError::NoJson { stdout: stdout.clone(), stderr: stderr.clone() })?;
    let payload: Value = serde_json::from_str(&stdout[json_start..]).map_err(AnalystError::InvalidJson)?;

    Ok(parse_findings(&payload))
}

/// Parse a `hex analyze --json` payload into architecture [`Finding`]s.
///
/// Recognised shapes (all optional; missing/empty arrays just produce no
/// findings of that kind):
///
/// * `rust_violations[]` — `{ file, line, message }` from
///   `scan_rust_boundary_violations`. No severity in the payload — hex
///   layer rules are non-negotiable, so we score these `High`.
/// * `violations[]` — `{ source_file, imported_path, rule }` from the
///   TypeScript adapter scan. Same rationale → `High`.
/// * `boundary_errors[]` — opaque counter records from the nexus
///   `/api/<pid>/health` rollup. We surface the count as a single `Medium`
///   finding so the swarm sees nexus disagreed with the local scan.
/// * `adr_compliance.violations[]` — `{ adr, file, line, message, severity }`.
///   Severity is read directly from the payload (`error` → High,
///   `warning` → Medium, anything else → Info).
/// * `dead_code[]` — `{ file, line, symbol?, message? }`. Forward-compat:
///   the `hex analyze` JSON does not currently emit this key, but the
///   workplan calls dead-code out explicitly so we parse it if present.
pub fn parse_findings(payload: &Value) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let push = |finding: Finding, seen: &mut std::collections::HashSet<String>, out: &mut Vec<Finding>| {
        if seen.insert(finding.id.clone()) {
            out.push(finding);
        }
    };

    if let Some(arr) = payload.get("rust_violations").and_then(Value::as_array) {
        for v in arr {
            if let Some(f) = build_rust_violation_finding(v) {
                push(f, &mut seen, &mut out);
            }
        }
    }

    if let Some(arr) = payload.get("violations").and_then(Value::as_array) {
        for v in arr {
            if let Some(f) = build_ts_violation_finding(v) {
                push(f, &mut seen, &mut out);
            }
        }
    }

    if let Some(arr) = payload.get("boundary_errors").and_then(Value::as_array) {
        for v in arr {
            if let Some(f) = build_boundary_error_finding(v) {
                push(f, &mut seen, &mut out);
            }
        }
    }

    if let Some(adr) = payload.get("adr_compliance") {
        if let Some(arr) = adr.get("violations").and_then(Value::as_array) {
            for v in arr {
                if let Some(f) = build_adr_violation_finding(v) {
                    push(f, &mut seen, &mut out);
                }
            }
        }
    }

    if let Some(arr) = payload.get("dead_code").and_then(Value::as_array) {
        for v in arr {
            if let Some(f) = build_dead_code_finding(v) {
                push(f, &mut seen, &mut out);
            }
        }
    }

    out
}

fn build_rust_violation_finding(v: &Value) -> Option<Finding> {
    let file = v.get("file").and_then(Value::as_str).unwrap_or("");
    let line = v.get("line").and_then(Value::as_u64).unwrap_or(0);
    let message = v.get("message").and_then(Value::as_str).unwrap_or("");
    if file.is_empty() && message.is_empty() {
        return None;
    }
    let title = if message.is_empty() {
        format!("rust boundary violation in {file}")
    } else {
        truncate(message, 120)
    };
    Some(Finding {
        id: stable_id("rust", &format!("{file}|{line}|{message}")),
        domain: Domain::Architecture,
        severity: Severity::High,
        title,
        evidence: vec![
            format!("{file}:{line}: {message}"),
            "hex analyze . --json: rust_violations".into(),
        ],
        suggested_action: SuggestedAction { kind: ActionKind::DraftWorkplan, draft_ref: None },
    })
}

fn build_ts_violation_finding(v: &Value) -> Option<Finding> {
    let source_file = v.get("source_file").and_then(Value::as_str).unwrap_or("");
    let imported_path = v.get("imported_path").and_then(Value::as_str).unwrap_or("");
    let rule = v.get("rule").and_then(Value::as_str).unwrap_or("");
    if source_file.is_empty() && imported_path.is_empty() {
        return None;
    }
    let title = format!("hex boundary violation: {source_file} imports {imported_path}");
    Some(Finding {
        id: stable_id("ts", &format!("{source_file}|{imported_path}|{rule}")),
        domain: Domain::Architecture,
        severity: Severity::High,
        title: truncate(&title, 120),
        evidence: vec![
            format!("{source_file} imports {imported_path}"),
            format!("rule = {rule}"),
            "hex analyze . --json: violations".into(),
        ],
        suggested_action: SuggestedAction { kind: ActionKind::DraftWorkplan, draft_ref: None },
    })
}

fn build_boundary_error_finding(v: &Value) -> Option<Finding> {
    let count = v.get("count").and_then(Value::as_u64);
    let count = count?;
    if count == 0 {
        return None;
    }
    Some(Finding {
        id: stable_id("boundary", &format!("count={count}")),
        domain: Domain::Architecture,
        severity: Severity::Medium,
        title: format!("nexus health reports {count} boundary error(s)"),
        evidence: vec![
            format!("count = {count}"),
            "hex analyze . --json: boundary_errors".into(),
        ],
        suggested_action: SuggestedAction { kind: ActionKind::AmendWorkplan, draft_ref: None },
    })
}

fn build_adr_violation_finding(v: &Value) -> Option<Finding> {
    let adr = v.get("adr").and_then(Value::as_str).unwrap_or("");
    let file = v.get("file").and_then(Value::as_str).unwrap_or("");
    let line = v.get("line").and_then(Value::as_u64).unwrap_or(0);
    let message = v.get("message").and_then(Value::as_str).unwrap_or("");
    let raw_severity = v.get("severity").and_then(Value::as_str).unwrap_or("warning");
    if adr.is_empty() && file.is_empty() && message.is_empty() {
        return None;
    }
    let severity = severity_from_str(raw_severity);
    let kind = match severity {
        Severity::Critical | Severity::High => ActionKind::DraftWorkplan,
        Severity::Medium => ActionKind::AmendWorkplan,
        _ => ActionKind::Informational,
    };
    let title = if adr.is_empty() {
        truncate(message, 120)
    } else if message.is_empty() {
        format!("{adr}: violation")
    } else {
        truncate(&format!("{adr}: {message}"), 120)
    };
    Some(Finding {
        id: stable_id("adr", &format!("{adr}|{file}|{line}|{message}|{raw_severity}")),
        domain: Domain::Architecture,
        severity,
        title,
        evidence: vec![
            format!("{file}:{line}: {message}"),
            format!("adr = {adr}"),
            format!("severity = {raw_severity}"),
        ],
        suggested_action: SuggestedAction { kind, draft_ref: None },
    })
}

fn build_dead_code_finding(v: &Value) -> Option<Finding> {
    let file = v.get("file").and_then(Value::as_str).unwrap_or("");
    let line = v.get("line").and_then(Value::as_u64).unwrap_or(0);
    let symbol = v.get("symbol").and_then(Value::as_str).unwrap_or("");
    let message = v.get("message").and_then(Value::as_str).unwrap_or("");
    if file.is_empty() && symbol.is_empty() && message.is_empty() {
        return None;
    }
    let raw_severity = v.get("severity").and_then(Value::as_str);
    let severity = raw_severity.map(severity_from_str).unwrap_or(Severity::Low);
    let title = if !symbol.is_empty() {
        format!("dead code: {symbol}")
    } else if !message.is_empty() {
        truncate(message, 120)
    } else {
        format!("dead code in {file}")
    };
    let mut evidence = vec![
        format!("{file}:{line} {symbol} {message}").trim().to_string(),
        "hex analyze . --json: dead_code".into(),
    ];
    if let Some(s) = raw_severity {
        evidence.push(format!("severity = {s}"));
    }
    Some(Finding {
        id: stable_id("dead", &format!("{file}|{line}|{symbol}|{message}")),
        domain: Domain::Architecture,
        severity,
        title: truncate(&title, 120),
        evidence,
        suggested_action: SuggestedAction { kind: ActionKind::AmendWorkplan, draft_ref: None },
    })
}

fn severity_from_str(s: &str) -> Severity {
    match s.to_ascii_lowercase().as_str() {
        "critical" | "fatal" => Severity::Critical,
        "error" | "high" => Severity::High,
        "warning" | "warn" | "medium" => Severity::Medium,
        "low" => Severity::Low,
        _ => Severity::Info,
    }
}

fn stable_id(prefix: &str, content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prefix.as_bytes());
    hasher.update([0u8]);
    hasher.update(content.as_bytes());
    let digest = hasher.finalize();
    let head = u64::from_be_bytes(digest[..8].try_into().unwrap());
    format!("f-arch-{prefix}-{head:016x}")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live_analyze_payload() -> Value {
        serde_json::json!({
            "score": 100,
            "violations": [],
            "boundary_errors": [],
            "rust_layers": [{ "layer": "Domain", "file_count": 25 }],
            "rust_violations": [],
            "adr_compliance": {
                "error_count": 0,
                "violation_count": 0,
                "violations": [],
                "warning_count": 0
            }
        })
    }

    #[test]
    fn clean_payload_yields_no_findings() {
        assert!(parse_findings(&live_analyze_payload()).is_empty());
    }

    #[test]
    fn rust_boundary_violation_becomes_high_architecture_finding() {
        let payload = serde_json::json!({
            "rust_violations": [
                { "file": "hex-nexus/src/adapters/foo.rs", "line": 12,
                  "message": "adapters/primary imports adapters/secondary" }
            ]
        });
        let findings = parse_findings(&payload);
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.domain, Domain::Architecture);
        assert_eq!(f.severity, Severity::High);
        assert_eq!(f.suggested_action.kind, ActionKind::DraftWorkplan);
        assert!(f.id.starts_with("f-arch-rust-"));
        assert!(
            f.evidence.iter().any(|e| e.contains("adapters/foo.rs:12")),
            "evidence missing file:line — {:?}",
            f.evidence
        );
        assert!(
            f.evidence.iter().any(|e| e.contains("rust_violations")),
            "evidence missing source tag — {:?}",
            f.evidence
        );
    }

    #[test]
    fn ts_violation_records_rule_in_evidence() {
        let payload = serde_json::json!({
            "violations": [
                { "source_file": "src/adapters/primary/cli.ts",
                  "imported_path": "src/adapters/secondary/db.ts",
                  "rule": "ADR-hex-adapter-isolation" }
            ]
        });
        let findings = parse_findings(&payload);
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.severity, Severity::High);
        assert_eq!(f.suggested_action.kind, ActionKind::DraftWorkplan);
        assert!(f.title.contains("primary/cli.ts"));
        assert!(f.evidence.iter().any(|e| e.contains("ADR-hex-adapter-isolation")));
        assert!(f.id.starts_with("f-arch-ts-"));
    }

    #[test]
    fn adr_compliance_severity_round_trips_from_payload() {
        let payload = serde_json::json!({
            "adr_compliance": {
                "violations": [
                    { "adr": "ADR-014", "file": "tests/foo.rs", "line": 99,
                      "message": "mock.module() forbidden", "severity": "error" },
                    { "adr": "ADR-027", "file": "src/lib.rs", "line": 5,
                      "message": "deprecated import path", "severity": "warning" },
                    { "adr": "ADR-099", "file": "src/x.rs", "line": 1,
                      "message": "fyi only", "severity": "info" }
                ]
            }
        });
        let findings = parse_findings(&payload);
        assert_eq!(findings.len(), 3);

        let by_adr = |id: &str| {
            findings
                .iter()
                .find(|f| f.title.contains(id))
                .unwrap_or_else(|| panic!("no finding for {id}; got {findings:?}"))
        };

        let err = by_adr("ADR-014");
        assert_eq!(err.severity, Severity::High);
        assert_eq!(err.suggested_action.kind, ActionKind::DraftWorkplan);
        assert!(err.evidence.iter().any(|e| e == "severity = error"));

        let warn = by_adr("ADR-027");
        assert_eq!(warn.severity, Severity::Medium);
        assert_eq!(warn.suggested_action.kind, ActionKind::AmendWorkplan);

        let info = by_adr("ADR-099");
        assert_eq!(info.severity, Severity::Info);
        assert_eq!(info.suggested_action.kind, ActionKind::Informational);
    }

    #[test]
    fn boundary_error_count_surfaces_as_medium_finding() {
        let payload = serde_json::json!({ "boundary_errors": [{ "count": 3 }] });
        let findings = parse_findings(&payload);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Medium);
        assert!(findings[0].title.contains("3 boundary error"));
    }

    #[test]
    fn boundary_error_zero_count_is_dropped() {
        let payload = serde_json::json!({ "boundary_errors": [{ "count": 0 }] });
        assert!(parse_findings(&payload).is_empty());
    }

    #[test]
    fn dead_code_array_becomes_low_amend_workplan_findings() {
        let payload = serde_json::json!({
            "dead_code": [
                { "file": "hex-cli/src/commands/legacy.rs", "line": 7, "symbol": "fn old_helper" },
                { "file": "hex-nexus/src/orphan.rs", "line": 1 }
            ]
        });
        let findings = parse_findings(&payload);
        assert_eq!(findings.len(), 2);
        for f in &findings {
            assert_eq!(f.domain, Domain::Architecture);
            assert_eq!(f.severity, Severity::Low);
            assert_eq!(f.suggested_action.kind, ActionKind::AmendWorkplan);
            assert!(f.id.starts_with("f-arch-dead-"));
        }
        assert!(findings.iter().any(|f| f.title.contains("old_helper")));
    }

    #[test]
    fn malformed_entries_are_skipped_not_panicked() {
        let payload = serde_json::json!({
            "rust_violations": [
                {},
                { "file": "", "line": 0, "message": "" },
                { "file": "src/ok.rs", "line": 1, "message": "real one" }
            ],
            "violations": [
                { "source_file": "", "imported_path": "" }
            ],
            "adr_compliance": { "violations": [{}] }
        });
        let findings = parse_findings(&payload);
        assert_eq!(findings.len(), 1, "expected only the real violation; got {findings:?}");
        assert!(findings[0].evidence.iter().any(|e| e.contains("src/ok.rs:1")));
    }

    #[test]
    fn finding_ids_are_stable_and_distinct_per_violation() {
        let payload = serde_json::json!({
            "rust_violations": [
                { "file": "a.rs", "line": 1, "message": "alpha" },
                { "file": "b.rs", "line": 2, "message": "beta"  }
            ]
        });
        let first = parse_findings(&payload);
        let second = parse_findings(&payload);
        assert_eq!(first[0].id, second[0].id, "ids must be deterministic");
        assert_eq!(first[1].id, second[1].id);
        assert_ne!(first[0].id, first[1].id, "distinct violations must not collide");
    }

    #[test]
    fn duplicate_entries_dedupe_within_a_single_payload() {
        let payload = serde_json::json!({
            "rust_violations": [
                { "file": "a.rs", "line": 1, "message": "dup" },
                { "file": "a.rs", "line": 1, "message": "dup" }
            ]
        });
        let findings = parse_findings(&payload);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn emitted_finding_round_trips_through_yaml() {
        let payload = serde_json::json!({
            "rust_violations": [
                { "file": "src/x.rs", "line": 9, "message": "boundary cross" }
            ]
        });
        let f = parse_findings(&payload).pop().unwrap();
        let yaml = serde_yaml::to_string(&f).expect("serialize yaml");
        let back: Finding = serde_yaml::from_str(&yaml).expect("deserialize yaml");
        assert_eq!(f, back);
        assert!(yaml.contains("domain: architecture"), "yaml = {yaml}");
        assert!(yaml.contains("severity: high"), "yaml = {yaml}");
    }
}
