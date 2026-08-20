//! Code-quality analyst (workplan `wp-idle-research-swarm`, P2.2).
//!
//! Runs `cargo check --workspace --message-format=json` and
//! `cargo clippy --workspace --message-format=json`, then parses the streamed
//! `compiler-message` records into [`Finding`]s with `domain = code_quality`.
//!
//! The IO layer is deliberately thin: [`analyze_code_quality`] just spawns the
//! two cargo invocations and concatenates their stdout. All structural
//! parsing happens in [`parse_cargo_diagnostics`], which takes a `&str` so it
//! is trivially unit-testable against fixture output (no toolchain required
//! in the test environment).

use std::path::{Path, PathBuf};
use std::process::Command;

use hex_core::{ActionKind, Domain, Finding, Severity, SuggestedAction};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Errors produced by [`analyze_code_quality`].
#[derive(Debug)]
pub enum AnalystError {
    /// Failed to spawn or wait on a cargo invocation.
    Spawn { command: &'static str, source: std::io::Error },
}

impl std::fmt::Display for AnalystError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalystError::Spawn { command, source } => {
                write!(f, "failed to spawn `{command}`: {source}")
            }
        }
    }
}

impl std::error::Error for AnalystError {}

/// Run the deterministic code-quality analyst against `workspace_root`.
///
/// Spawns `cargo check` then `cargo clippy`, both in JSON message format,
/// concatenates their stdout, and parses the result. Non-zero exit codes are
/// expected — cargo exits non-zero when warnings exist under some
/// configurations, and clippy exits non-zero on lint failures. We intentionally
/// ignore status and rely on the parsed messages instead.
pub fn analyze_code_quality(workspace_root: &Path) -> Result<Vec<Finding>, AnalystError> {
    let mut combined = String::new();

    let check = run_cargo(workspace_root, &["check", "--workspace", "--message-format=json"])
        .map_err(|source| AnalystError::Spawn { command: "cargo check", source })?;
    combined.push_str(&check);

    let clippy = run_cargo(workspace_root, &["clippy", "--workspace", "--message-format=json"])
        .map_err(|source| AnalystError::Spawn { command: "cargo clippy", source })?;
    combined.push_str(&clippy);

    Ok(parse_cargo_diagnostics(&combined))
}

fn run_cargo(workspace_root: &Path, args: &[&str]) -> std::io::Result<String> {
    let output = Command::new("cargo")
        .args(args)
        .current_dir(workspace_root)
        .output()?;
    // Cargo emits diagnostics on stdout when --message-format=json; stderr
    // carries human-readable progress/summary lines we don't need.
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Parse cargo's JSON message stream into [`Finding`]s.
///
/// The input is the concatenated stdout of one or more cargo invocations run
/// with `--message-format=json` (one JSON object per line). Lines that fail to
/// parse, or that aren't `compiler-message`s with an `error` / `warning`
/// level, are skipped silently — cargo emits other reasons (`compiler-artifact`,
/// `build-script-executed`, ...) we don't care about here.
///
/// Findings are deduplicated by content hash (file/line/code/message) so
/// running both `check` and `clippy` doesn't produce two records for the same
/// underlying issue.
pub fn parse_cargo_diagnostics(input: &str) -> Vec<Finding> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<Finding> = Vec::new();

    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() || !line.starts_with('{') {
            continue;
        }
        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if value.get("reason").and_then(Value::as_str) != Some("compiler-message") {
            continue;
        }
        let Some(message) = value.get("message") else { continue };
        let Some(finding) = build_finding(message) else { continue };
        if seen.insert(finding.id.clone()) {
            out.push(finding);
        }
    }

    out
}

fn build_finding(message: &Value) -> Option<Finding> {
    let level = message.get("level").and_then(Value::as_str)?;
    let severity = match level {
        "error" | "error: internal compiler error" => Severity::High,
        "warning" => Severity::Medium,
        // note / help / failure-note carry no independent finding signal —
        // they're already attached to the parent diagnostic via children.
        _ => return None,
    };

    let title = message.get("message").and_then(Value::as_str)?.to_string();
    let code: Option<&str> = message
        .get("code")
        .and_then(|c| c.get("code"))
        .and_then(Value::as_str);

    let primary_span = message
        .get("spans")
        .and_then(Value::as_array)
        .and_then(|spans| {
            spans
                .iter()
                .find(|s| s.get("is_primary").and_then(Value::as_bool).unwrap_or(false))
                .or_else(|| spans.first())
        });

    let mut evidence = Vec::new();
    if let Some(span) = primary_span {
        let file = span.get("file_name").and_then(Value::as_str).unwrap_or("?");
        let line = span.get("line_start").and_then(Value::as_u64).unwrap_or(0);
        evidence.push(format!("{file}:{line}"));
    }
    if let Some(c) = code {
        evidence.push(format!("code: {c}"));
    }
    evidence.push(format!("level: {level}"));

    let suggested_action = match severity {
        Severity::High | Severity::Critical => SuggestedAction {
            kind: ActionKind::DraftWorkplan,
            draft_ref: None,
        },
        _ => SuggestedAction {
            kind: ActionKind::Informational,
            draft_ref: None,
        },
    };

    let id = stable_id(level, code, primary_span, &title);

    Some(Finding {
        id,
        domain: Domain::CodeQuality,
        severity,
        title,
        evidence,
        suggested_action,
    })
}

fn stable_id(level: &str, code: Option<&str>, span: Option<&Value>, title: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(level.as_bytes());
    hasher.update([0u8]);
    hasher.update(code.unwrap_or("").as_bytes());
    hasher.update([0u8]);
    if let Some(span) = span {
        let file = span.get("file_name").and_then(Value::as_str).unwrap_or("");
        let line = span.get("line_start").and_then(Value::as_u64).unwrap_or(0);
        hasher.update(file.as_bytes());
        hasher.update([0u8]);
        hasher.update(line.to_le_bytes());
    }
    hasher.update([0u8]);
    hasher.update(title.as_bytes());
    let digest = hasher.finalize();
    format!("f-cq-{:016x}", u64::from_be_bytes(digest[..8].try_into().unwrap()))
}

/// Helper to default to the current working directory when no root is given.
#[allow(dead_code)]
pub fn analyze_cwd() -> Result<Vec<Finding>, AnalystError> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    analyze_code_quality(&cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal but realistic samples of `cargo --message-format=json` output.
    // Each line is one full JSON object (cargo emits NDJSON).
    fn warning_line() -> String {
        serde_json::json!({
            "reason": "compiler-message",
            "package_id": "hex-nexus 0.1.0",
            "manifest_path": "/repo/hex-nexus/Cargo.toml",
            "target": { "kind": ["lib"], "name": "hex_nexus" },
            "message": {
                "rendered": "warning: unused variable: `x`\n",
                "level": "warning",
                "message": "unused variable: `x`",
                "code": { "code": "unused_variables", "explanation": null },
                "spans": [{
                    "file_name": "hex-nexus/src/research/code_quality_analyst.rs",
                    "byte_start": 100,
                    "byte_end": 101,
                    "line_start": 42,
                    "line_end": 42,
                    "column_start": 9,
                    "column_end": 10,
                    "is_primary": true,
                    "text": [],
                    "label": null,
                    "suggested_replacement": null,
                    "suggestion_applicability": null,
                    "expansion": null
                }],
                "children": []
            }
        })
        .to_string()
    }

    fn error_line() -> String {
        serde_json::json!({
            "reason": "compiler-message",
            "message": {
                "rendered": "error[E0425]: cannot find value `does_not_exist` in this scope",
                "level": "error",
                "message": "cannot find value `does_not_exist` in this scope",
                "code": { "code": "E0425", "explanation": null },
                "spans": [{
                    "file_name": "hex-nexus/src/lib.rs",
                    "line_start": 7,
                    "line_end": 7,
                    "is_primary": true
                }]
            }
        })
        .to_string()
    }

    fn artifact_line() -> String {
        // Non-diagnostic message — must be ignored.
        serde_json::json!({
            "reason": "compiler-artifact",
            "package_id": "hex-nexus 0.1.0",
            "filenames": []
        })
        .to_string()
    }

    fn note_line() -> String {
        // `note` is not a finding level — children of a parent diagnostic.
        serde_json::json!({
            "reason": "compiler-message",
            "message": {
                "level": "note",
                "message": "the macro was defined here",
                "spans": []
            }
        })
        .to_string()
    }

    #[test]
    fn parses_warning_into_medium_finding() {
        let findings = parse_cargo_diagnostics(&warning_line());
        assert_eq!(findings.len(), 1, "got: {findings:?}");
        let f = &findings[0];
        assert_eq!(f.domain, Domain::CodeQuality);
        assert_eq!(f.severity, Severity::Medium);
        assert_eq!(f.title, "unused variable: `x`");
        assert!(
            f.evidence.iter().any(|e| e.contains("code_quality_analyst.rs:42")),
            "evidence missing file:line — {:?}",
            f.evidence
        );
        assert!(
            f.evidence.iter().any(|e| e == "code: unused_variables"),
            "evidence missing code — {:?}",
            f.evidence
        );
        assert_eq!(f.suggested_action.kind, ActionKind::Informational);
    }

    #[test]
    fn parses_error_into_high_finding_with_draft_workplan() {
        let findings = parse_cargo_diagnostics(&error_line());
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.severity, Severity::High);
        assert_eq!(f.suggested_action.kind, ActionKind::DraftWorkplan);
        assert!(f.evidence.iter().any(|e| e.contains("hex-nexus/src/lib.rs:7")));
        assert!(f.evidence.iter().any(|e| e == "code: E0425"));
    }

    #[test]
    fn ignores_non_diagnostic_messages() {
        let mut input = String::new();
        input.push_str(&artifact_line());
        input.push('\n');
        input.push_str(&note_line());
        input.push('\n');
        let findings = parse_cargo_diagnostics(&input);
        assert!(findings.is_empty(), "expected no findings, got {findings:?}");
    }

    #[test]
    fn dedupes_identical_diagnostics_across_check_and_clippy() {
        // Same diagnostic emitted by both `cargo check` and `cargo clippy`
        // (clippy re-runs the checker) should collapse to a single finding.
        let mut input = String::new();
        input.push_str(&warning_line());
        input.push('\n');
        input.push_str(&warning_line());
        input.push('\n');
        let findings = parse_cargo_diagnostics(&input);
        assert_eq!(findings.len(), 1, "expected dedup; got {findings:?}");
    }

    #[test]
    fn skips_malformed_lines() {
        let mut input = String::new();
        input.push_str("not json at all\n");
        input.push_str("{ broken json\n");
        input.push('\n');
        input.push_str(&warning_line());
        input.push('\n');
        let findings = parse_cargo_diagnostics(&input);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn finding_id_is_stable_for_same_input() {
        let a = parse_cargo_diagnostics(&warning_line());
        let b = parse_cargo_diagnostics(&warning_line());
        assert_eq!(a[0].id, b[0].id);
        assert!(a[0].id.starts_with("f-cq-"));
    }

    #[test]
    fn yaml_roundtrip_for_emitted_finding() {
        // Anything we emit must satisfy the on-disk YAML contract from
        // `hex-core::research_finding`.
        let f = parse_cargo_diagnostics(&warning_line()).pop().unwrap();
        let yaml = serde_yaml::to_string(&f).expect("serialize");
        let back: Finding = serde_yaml::from_str(&yaml).expect("deserialize");
        assert_eq!(f, back);
        assert!(yaml.contains("domain: code_quality"), "yaml = {yaml}");
    }
}
