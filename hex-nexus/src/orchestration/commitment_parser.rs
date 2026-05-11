//! Parse persona replies for Confirm/PLAN structures and write them to
//! the STDB `commitment` table so the operator can audit
//! who-promised-what and which deadlines slipped.
//!
//! Recognised shapes:
//!   `Confirm: I (cto) will <action> by <when>`        (board contributor)
//!   `Confirm: I will <action> by <when>`              (loose form)
//!   `PLAN:` block with FIRST ACTION + SUCCESS lines   (board planner)
//!
//! For everything else (free-form filler, "I'll communicate with the
//! engineering lead" etc), we DO NOT extract a commitment — that's how
//! the dashboard makes the no-commitment case visible: nothing in the
//! ledger means the persona didn't actually agree to do anything
//! verifiable.

use std::time::Duration;

const STDB_HOST_DEFAULT: &str = "http://127.0.0.1:3033";

fn stdb_host() -> String {
    std::env::var("HEX_SPACETIMEDB_HOST").unwrap_or_else(|_| STDB_HOST_DEFAULT.to_string())
}

fn hex_db() -> String {
    std::env::var("HEX_STDB_DATABASE")
        .unwrap_or_else(|_| hex_core::stdb_database_for_module("hexflo-coordination").to_string())
}

#[derive(Debug)]
pub struct ExtractedCommitment {
    pub raw_text: String,
    pub action: String,
    pub deadline_micros: i64,
    pub success_artifact: String,
    pub artifact_kind: String, // verifiable_path | verifiable_route | operator_action | none
}

pub async fn extract_and_record(
    role: &str,
    reply: &str,
    thread_id: &str,
    related_msg_id: u64,
) {
    let extracted = parse(reply);
    if extracted.is_empty() {
        return;
    }
    let url = format!(
        "{}/v1/database/{}/call/commitment_open",
        stdb_host(),
        hex_db()
    );
    let http = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(error = %e, "commitment_parser: http client build failed");
            return;
        }
    };
    for c in extracted {
        let body = serde_json::json!([
            role,
            c.raw_text,
            c.action,
            c.deadline_micros,
            c.success_artifact,
            c.artifact_kind,
            thread_id,
            related_msg_id,
        ]);
        match http.post(&url).json(&body).send().await {
            Ok(r) if r.status().is_success() => {
                tracing::info!(
                    role = %role,
                    action = %c.action,
                    artifact = %c.success_artifact,
                    "commitment recorded"
                );
            }
            Ok(r) => tracing::debug!(
                status = %r.status(),
                role = %role,
                "commitment_parser: open non-2xx"
            ),
            Err(e) => tracing::debug!(
                error = %e,
                role = %role,
                "commitment_parser: open transport error"
            ),
        }
    }
}

/// Top-level parse. Walks the reply line-by-line, extracting any
/// Confirm: line and the FIRST ACTION / SUCCESS pair from a PLAN: block.
pub fn parse(reply: &str) -> Vec<ExtractedCommitment> {
    let mut out: Vec<ExtractedCommitment> = Vec::new();

    // Confirm: lines.
    for line in reply.lines() {
        let lt = line.trim();
        let lower = lt.to_ascii_lowercase();
        if lower.starts_with("confirm:") || lower.starts_with("confirm ") {
            let body = lt.split_once(':').map(|(_, r)| r.trim()).unwrap_or(lt);
            // Split out the success: clause if present (per ADR-2026-05-08-2400 contract).
            //   "Confirm: I will X by Y — success: docs/specs/foo.md"
            // The dash before success can be `—`, `-`, `--`, or just whitespace.
            let (body_no_success, success_artifact) = split_success_clause(body);
            let (action, deadline_micros) = split_action_and_deadline(body_no_success);
            let artifact_kind = if success_artifact.is_empty() {
                "none".to_string()
            } else {
                classify_artifact(&success_artifact)
            };
            out.push(ExtractedCommitment {
                raw_text: lt.to_string(),
                action: if action.is_empty() {
                    body_no_success.to_string()
                } else {
                    action
                },
                deadline_micros,
                success_artifact,
                artifact_kind,
            });
        }
    }

    // PLAN: block — extract FIRST ACTION + SUCCESS.
    if let Some(plan_block) = extract_plan_block(reply) {
        let mut first_action = String::new();
        let mut success = String::new();
        let mut owner = String::new();
        for line in plan_block.lines() {
            let lt = line.trim();
            let lower = lt.to_ascii_lowercase();
            if let Some(rest) = lower.strip_prefix("first action:") {
                first_action = lt[lt.len() - rest.len()..].trim().to_string();
            } else if let Some(rest) = lower.strip_prefix("success:") {
                success = lt[lt.len() - rest.len()..].trim().to_string();
            } else if let Some(rest) = lower.strip_prefix("owner:") {
                owner = lt[lt.len() - rest.len()..].trim().to_string();
            }
        }
        if !first_action.is_empty() {
            let kind = classify_artifact(&success);
            out.push(ExtractedCommitment {
                raw_text: plan_block.lines().take(8).collect::<Vec<_>>().join("\n"),
                action: if owner.is_empty() {
                    first_action.clone()
                } else {
                    format!("[owner={}] {}", owner, first_action)
                },
                deadline_micros: 0, // PLAN block has no deadline; use grace
                success_artifact: success,
                artifact_kind: kind,
            });
        }
    }

    out
}

/// Cheap deadline parser. Handles a few common forms:
///   "by 23:00"         (today HH:MM, local — converted via SystemTime)
///   "by tomorrow"      (24h from now)
///   "by EOD" / "EOW"   (24h / 7d)
///   "in 2h" / "in 30m"
///   default: 0 (caller falls back to 1h grace)
fn split_action_and_deadline(s: &str) -> (String, i64) {
    let lower = s.to_ascii_lowercase();
    let now_micros = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0);
    let hour = 60 * 60 * 1_000_000i64;
    let day = 24 * hour;
    let week = 7 * day;

    let mut deadline = 0i64;
    if lower.contains(" by tomorrow") || lower.contains(" by tmrw") {
        deadline = now_micros + day;
    } else if lower.contains(" by eod") || lower.contains(" by end of day") {
        deadline = now_micros + day;
    } else if lower.contains(" by eow") || lower.contains(" by end of week") {
        deadline = now_micros + week;
    } else if let Some(rest) = lower.find(" in ") {
        let tail = &lower[rest + 4..];
        let mut digits = String::new();
        let mut unit = ' ';
        for ch in tail.chars() {
            if ch.is_ascii_digit() {
                digits.push(ch);
            } else if ch == 'h' || ch == 'm' || ch == 'd' {
                unit = ch;
                break;
            } else if !digits.is_empty() {
                break;
            }
        }
        if let Ok(n) = digits.parse::<i64>() {
            deadline = match unit {
                'h' => now_micros + n * hour,
                'm' => now_micros + n * 60 * 1_000_000,
                'd' => now_micros + n * day,
                _ => 0,
            };
        }
    }

    // Action = everything before " by " or " in " if found, else whole thing.
    let action = ["by tomorrow", "by tmrw", "by eod", "by end of day", "by eow", "by end of week"]
        .iter()
        .find_map(|kw| lower.find(kw).map(|i| s[..i].trim().to_string()))
        .or_else(|| lower.find(" in ").map(|i| s[..i].trim().to_string()))
        .unwrap_or_else(|| s.trim().to_string());

    (action, deadline)
}

fn extract_plan_block(reply: &str) -> Option<String> {
    let mut start: Option<usize> = None;
    for (i, line) in reply.lines().enumerate() {
        if line.trim_start().to_ascii_uppercase().starts_with("PLAN:") {
            start = Some(i);
            break;
        }
    }
    let s = start?;
    let lines: Vec<&str> = reply.lines().collect();
    Some(lines[s..(s + 8).min(lines.len())].join("\n"))
}

/// Split a Confirm body on the `success:` token. Handles the dashed
/// variants the strict prompt produces ("— success:", "- success:",
/// "-- success:") plus bare "success:".
fn split_success_clause(body: &str) -> (&str, String) {
    let lower = body.to_ascii_lowercase();
    // Search for the literal "success:" token; backtrack to consume
    // any preceding dash + whitespace from the action half.
    let pos = match lower.find("success:") {
        Some(p) => p,
        None => return (body, String::new()),
    };
    let action_end = body[..pos].trim_end();
    // Strip trailing dash separators (—, --, -) and whitespace.
    let action_end = action_end
        .trim_end_matches(|c: char| c.is_whitespace() || c == '—' || c == '-' || c == '–');
    let success_raw = body[pos + "success:".len()..].trim();
    (action_end, success_raw.to_string())
}

fn classify_artifact(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    if lower.contains("requires-operator-action")
        || lower.contains("operator action")
        || lower.contains("manual")
    {
        return "operator_action".to_string();
    }
    if lower.starts_with('#') || lower.starts_with("/dashboard") || lower.starts_with("dashboard:") {
        return "verifiable_route".to_string();
    }
    if lower.contains("docs/")
        || lower.contains("src/")
        || lower.contains("hex-nexus/")
        || lower.contains("hex-cli/")
        || lower.contains(".md")
        || lower.contains(".rs")
        || lower.contains(".tsx")
        || lower.contains(".ts")
    {
        return "verifiable_path".to_string();
    }
    "none".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_confirm_with_eod() {
        let r = "Confirm: I (cto) will draft ADR-foo by EOD";
        let cs = parse(r);
        assert_eq!(cs.len(), 1);
        assert!(cs[0].deadline_micros > 0);
        assert!(cs[0].action.contains("draft ADR-foo"));
    }

    #[test]
    fn parse_plan_block() {
        let r = "PLAN:\nOBJECTIVE: ship the foo\nOWNER: cto\nFIRST ACTION: write docs/specs/foo.md\nSUCCESS: docs/specs/foo.md exists";
        let cs = parse(r);
        assert!(!cs.is_empty());
        let plan = cs.last().unwrap();
        assert_eq!(plan.artifact_kind, "verifiable_path");
        assert!(plan.success_artifact.contains("docs/specs/foo.md"));
    }

    #[test]
    fn empty_promise_yields_nothing() {
        let r = "I'll communicate with the engineering lead and ensure clarity.";
        assert!(parse(r).is_empty());
    }

    #[test]
    fn parse_confirm_with_success_dash() {
        let r = "Confirm: I (cto) will document foo by EOD — success: docs/specs/foo.md";
        let cs = parse(r);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].artifact_kind, "verifiable_path");
        assert_eq!(cs[0].success_artifact, "docs/specs/foo.md");
        assert!(cs[0].action.contains("document foo"));
        assert!(!cs[0].action.contains("success:"));
        assert!(!cs[0].action.ends_with('—'));
    }

    #[test]
    fn parse_confirm_with_success_no_dash() {
        let r = "Confirm: I will draft the spec by tomorrow success: src/foo.rs";
        let cs = parse(r);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].success_artifact, "src/foo.rs");
    }

    #[test]
    fn parse_confirm_requires_operator_action() {
        let r = "Confirm: I (coo) will run a security audit — success: requires-operator-action — call legal";
        let cs = parse(r);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].artifact_kind, "operator_action");
    }
}
