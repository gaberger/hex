//! BS-5 ThoughtPattern detector — reads agent_thought via the nexus merge
//! API and emits findings for the improver loop.
//!
//! Patterns covered in v1:
//!   1. **adr_repetition** — same ADR-ID referenced across ≥3 recent thoughts
//!      by ≥2 distinct personas → likely-unresolved architectural concern.
//!   2. **frustration_spike** — `kind=frustration` count exceeds threshold in
//!      the recent window → resource or methodology problem.
//!
//! Wire-up: a TOML detector entry runs
//! `hex sched improver thought-patterns --json`; this prints
//! `{findings: [...]}`, which discover.rs ingests like any other detector.
//!
//! Closes the consumer side of Phase-2 thought memory. Pairs with the
//! emitter side committed in `7671f7d3` (strip_think_block + nemotron-mini
//! summarizer pin).

use std::collections::HashMap;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use regex::Regex;
use serde_json::{json, Value};

/// Default nexus base — override with HEX_NEXUS_URL.
const NEXUS_DEFAULT: &str = "http://127.0.0.1:5555";

/// Pull this many recent thoughts (newest first). 200 ≈ ~last day of board
/// chatter on a busy fleet; small enough to keep the detector under 1 s.
const THOUGHT_WINDOW: u32 = 200;

/// adr_repetition firing threshold.
const ADR_REPETITION_MIN_COUNT: usize = 3;
const ADR_REPETITION_MIN_ROLES: usize = 2;
const ADR_REPETITION_ERROR_COUNT: usize = 5;

/// frustration_spike firing threshold within the window.
const FRUSTRATION_SPIKE_MIN: usize = 5;
const FRUSTRATION_SPIKE_ERROR: usize = 10;

/// Match canonical hyphenated ADR IDs (`ADR-2605082500`).
/// We deliberately *don't* match legacy `ADR-NNN` short form — those produce
/// too many false positives against arbitrary numbers in body text.
fn adr_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"ADR-\d{4}-\d{2}-\d{2}-\d{3,4}").expect("ADR regex compiles")
    })
}

/// Run the detector and print `{findings: [...]}` JSON to stdout.
/// Returns Err only on transport/parse errors against the nexus API —
/// the discover.rs loop already treats those as detector-health findings.
pub async fn run(json: bool) -> Result<()> {
    let findings = detect().await?;
    if json {
        println!("{}", serde_json::to_string(&json!({"findings": findings}))?);
    } else if findings.is_empty() {
        println!("no thought-pattern findings");
    } else {
        for f in &findings {
            println!("- {}", f);
        }
    }
    Ok(())
}

/// Core detection logic — broken out so unit tests can drive it with a
/// synthetic thoughts payload without touching the network.
pub async fn detect() -> Result<Vec<Value>> {
    let thoughts = fetch_thoughts().await?;
    Ok(detect_in(&thoughts))
}

pub fn detect_in(thoughts: &[Value]) -> Vec<Value> {
    let mut findings: Vec<Value> = Vec::new();

    // ── Pattern 1: ADR repetition ─────────────────────────────────────
    // Map adr_id → (count, roles set, sample thought_ids).
    let mut adr_hits: HashMap<String, (usize, std::collections::BTreeSet<String>, Vec<u64>)> =
        HashMap::new();
    for t in thoughts {
        let content = t.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let role = t.get("agent_role").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let tid = t.get("thought_id").and_then(|v| v.as_u64()).unwrap_or(0);
        // De-dup per-thought so a single thought citing the same ADR twice
        // doesn't inflate the count.
        let mut seen_here = std::collections::HashSet::new();
        for m in adr_re().find_iter(content) {
            let adr_id = m.as_str().to_string();
            if !seen_here.insert(adr_id.clone()) {
                continue;
            }
            let entry = adr_hits.entry(adr_id).or_insert_with(|| {
                (0, std::collections::BTreeSet::new(), Vec::new())
            });
            entry.0 += 1;
            if !role.is_empty() {
                entry.1.insert(role.clone());
            }
            if entry.2.len() < 5 {
                entry.2.push(tid);
            }
        }
    }
    for (adr_id, (count, roles, sample_ids)) in adr_hits {
        if count < ADR_REPETITION_MIN_COUNT || roles.len() < ADR_REPETITION_MIN_ROLES {
            continue;
        }
        let severity = if count >= ADR_REPETITION_ERROR_COUNT { "error" } else { "warning" };
        let role_list: Vec<&String> = roles.iter().collect();
        findings.push(json!({
            "pattern":          "adr_repetition",
            "scope":            adr_id,
            "severity":         severity,
            "count":            count,
            "mentioning_roles": role_list,
            "sample_thought_ids": sample_ids,
            "window":           THOUGHT_WINDOW,
        }));
    }

    // ── Pattern 2: frustration spike ──────────────────────────────────
    let frustration_count = thoughts
        .iter()
        .filter(|t| t.get("kind").and_then(|v| v.as_str()) == Some("frustration"))
        .count();
    if frustration_count >= FRUSTRATION_SPIKE_MIN {
        let severity = if frustration_count >= FRUSTRATION_SPIKE_ERROR { "error" } else { "warning" };
        let by_role: HashMap<String, usize> = thoughts
            .iter()
            .filter(|t| t.get("kind").and_then(|v| v.as_str()) == Some("frustration"))
            .filter_map(|t| t.get("agent_role").and_then(|v| v.as_str()).map(String::from))
            .fold(HashMap::new(), |mut m, r| { *m.entry(r).or_insert(0) += 1; m });
        findings.push(json!({
            "pattern":  "frustration_spike",
            "scope":    "kind:frustration",
            "severity": severity,
            "count":    frustration_count,
            "by_role":  by_role,
            "window":   THOUGHT_WINDOW,
        }));
    }

    findings
}

async fn fetch_thoughts() -> Result<Vec<Value>> {
    let base = std::env::var("HEX_NEXUS_URL").unwrap_or_else(|_| NEXUS_DEFAULT.to_string());
    let url = format!("{}/api/merge/thoughts?limit={}", base.trim_end_matches('/'), THOUGHT_WINDOW);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .context("build http client")?;
    let resp = client.get(&url).send().await.with_context(|| format!("GET {}", url))?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("nexus returned {} for {}", status, url);
    }
    let body: Value = resp.json().await.context("parse nexus response as JSON")?;
    let thoughts = body
        .get("thoughts")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(thoughts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thought(id: u64, role: &str, kind: &str, content: &str) -> Value {
        json!({
            "thought_id":      id,
            "agent_role":      role,
            "kind":            kind,
            "content":         content,
            "related_msg_id":  0,
            "related_task_id": "",
            "confidence":      0.0,
            "created_at":      "",
        })
    }

    #[test]
    fn empty_input_produces_no_findings() {
        assert!(detect_in(&[]).is_empty());
    }

    #[test]
    fn single_adr_reference_does_not_fire() {
        let t = vec![thought(1, "cto", "decision", "ADR-2605082500 is the spine")];
        assert!(detect_in(&t).is_empty());
    }

    #[test]
    fn three_mentions_one_role_does_not_fire() {
        let t = vec![
            thought(1, "cto", "decision", "ADR-2605082500 is the spine"),
            thought(2, "cto", "decision", "we need to close ADR-2605082500"),
            thought(3, "cto", "decision", "ADR-2605082500 status pending"),
        ];
        // ≥3 mentions but only 1 role → does not fire (the min-roles guard).
        assert!(detect_in(&t).is_empty());
    }

    #[test]
    fn three_mentions_two_roles_fires_warning() {
        let t = vec![
            thought(1, "cto", "decision", "ADR-2605082500 is the spine"),
            thought(2, "cpo", "decision", "cost spec depends on ADR-2605082500"),
            thought(3, "ciso", "decision", "ADR-2605082500 needs A01 follow-up"),
        ];
        let f = detect_in(&t);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0]["pattern"], "adr_repetition");
        assert_eq!(f[0]["scope"], "ADR-2605082500");
        assert_eq!(f[0]["severity"], "warning");
        assert_eq!(f[0]["count"], 3);
    }

    #[test]
    fn five_mentions_escalates_to_error() {
        let t: Vec<Value> = (0..6)
            .map(|i| {
                let role = if i % 2 == 0 { "cto" } else { "cpo" };
                thought(i, role, "decision", "ADR-2605082500 again")
            })
            .collect();
        let f = detect_in(&t);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0]["severity"], "error");
    }

    #[test]
    fn frustration_spike_fires() {
        let t: Vec<Value> = (0..6)
            .map(|i| thought(i, "cto", "frustration", "stuck again"))
            .collect();
        let f = detect_in(&t);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0]["pattern"], "frustration_spike");
        assert_eq!(f[0]["severity"], "warning");
    }

    #[test]
    fn duplicated_adr_in_single_thought_counts_once() {
        let t = vec![
            thought(1, "cto", "decision", "ADR-2605082500 and ADR-2605082500"),
            thought(2, "cpo", "decision", "ADR-2605082500 mentioned"),
        ];
        // Only 2 thoughts mention the ADR; doesn't reach min-count of 3.
        assert!(detect_in(&t).is_empty());
    }

    #[test]
    fn short_form_adr_ids_are_not_matched() {
        // ADR-NNN is a legacy form; we don't match it to avoid FPs against
        // arbitrary numbers in prose. Two roles, three thoughts → would
        // fire IF we matched the short form.
        let t = vec![
            thought(1, "cto", "decision", "ADR-014 has the rule"),
            thought(2, "cpo", "decision", "see ADR-014"),
            thought(3, "ciso", "decision", "ADR-014 audit"),
        ];
        assert!(detect_in(&t).is_empty());
    }
}
