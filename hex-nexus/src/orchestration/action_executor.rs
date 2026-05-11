//! Action executor — runs proposed_action rows the digital twin approved.
//!
//! Today: ONE sink — `file_write` via stdlib + final-mile guards. Future
//! ADRs add `shell_exec`, `dm_send`, etc. Each new kind goes through the
//! same approve→execute path so the audit trail stays consistent.

use std::path::{Path, PathBuf};
use std::time::Duration;

const POLL_INTERVAL_SECS: u64 = 15;

pub fn spawn(stdb_host: String, hex_db: String, repo_root: PathBuf) {
    if std::env::var("HEX_DISABLE_ACTION_EXECUTOR").is_ok() {
        tracing::info!("action_executor disabled via HEX_DISABLE_ACTION_EXECUTOR");
        return;
    }
    tokio::spawn(async move {
        let http = match reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "action_executor: http client build failed; disabled");
                return;
            }
        };
        tracing::info!(
            stdb_host = %stdb_host,
            db = %hex_db,
            repo_root = %repo_root.display(),
            "action_executor: started"
        );
        tokio::time::sleep(Duration::from_secs(60)).await;

        let mut ticker = tokio::time::interval(Duration::from_secs(POLL_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;
            if let Err(e) = run_one(&http, &stdb_host, &hex_db, &repo_root).await {
                tracing::debug!(error = %e, "action_executor: tick error");
            }
        }
    });
}

#[derive(Debug)]
struct ApprovedAction {
    id: u64,
    kind: String,
    payload_json: String,
    related_commitment_id: u64,
}

async fn run_one(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    repo_root: &Path,
) -> Result<(), String> {
    let approved = fetch_approved(http, stdb_host, hex_db).await?;
    for action in approved.into_iter().take(3) {
        if let Err(e) = execute_one(http, stdb_host, hex_db, repo_root, &action).await {
            tracing::warn!(action_id = action.id, error = %e, "action_executor: execute_one failed");
        }
    }
    Ok(())
}

async fn fetch_approved(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
) -> Result<Vec<ApprovedAction>, String> {
    let url = format!("{}/v1/database/{}/sql", stdb_host, hex_db);
    let body = "SELECT id, kind, payload_json, related_commitment_id, status FROM proposed_action";
    let resp = http
        .post(&url)
        .header("Content-Type", "text/plain")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("http: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| format!("json: {}", e))?;
    let rows = v
        .as_array()
        .and_then(|a| a.first())
        .and_then(|t| t.get("rows"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for r in rows {
        let cols = match r.as_array() {
            Some(c) => c,
            None => continue,
        };
        if cols.len() < 5 {
            continue;
        }
        let status = cols.get(4).and_then(|x| x.as_str()).unwrap_or("");
        if status != "approved" {
            continue;
        }
        out.push(ApprovedAction {
            id: cols.first().and_then(|x| x.as_u64()).unwrap_or(0),
            kind: cols.get(1).and_then(|x| x.as_str()).unwrap_or("").to_string(),
            payload_json: cols.get(2).and_then(|x| x.as_str()).unwrap_or("").to_string(),
            related_commitment_id: cols.get(3).and_then(|x| x.as_u64()).unwrap_or(0),
        });
    }
    Ok(out)
}

async fn execute_one(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    repo_root: &Path,
    action: &ApprovedAction,
) -> Result<(), String> {
    match action.kind.as_str() {
        "file_write" => execute_file_write(http, stdb_host, hex_db, repo_root, action).await,
        other => {
            mark_failed(
                http,
                stdb_host,
                hex_db,
                action.id,
                &format!("unknown action kind: {}", other),
            )
            .await
        }
    }
}

async fn execute_file_write(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    repo_root: &Path,
    action: &ApprovedAction,
) -> Result<(), String> {
    let payload: serde_json::Value = serde_json::from_str(&action.payload_json)
        .map_err(|e| format!("payload parse: {}", e))?;
    let rel_path = payload
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or("missing path")?;
    let content = payload
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or("missing content")?;

    // Resolve + canonicalise to confirm the path stays under repo root.
    let target = repo_root.join(rel_path);
    let canonical_root = repo_root
        .canonicalize()
        .map_err(|e| format!("canonicalise repo_root: {}", e))?;
    let parent = match target.parent() {
        Some(p) => p,
        None => return Err("target has no parent dir".to_string()),
    };
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("create parent dir {}: {}", parent.display(), e))?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|e| format!("canonicalise parent {}: {}", parent.display(), e))?;
    if !canonical_parent.starts_with(&canonical_root) {
        return mark_failed(
            http,
            stdb_host,
            hex_db,
            action.id,
            &format!(
                "executor refused: {} resolves outside repo root",
                target.display()
            ),
        )
        .await;
    }

    // ADR-2605110700 R1 — capture pre-write state so we can roll back if
    // cargo_check rejects the patch. Without this, autonomous SOP runs
    // accumulate broken state overnight (5 of 6 cycles on 2026-05-10 failed
    // cargo_check after this gate was advisory-only).
    let pre_write_backup: Option<String> = if target.exists() {
        std::fs::read_to_string(&target).ok()
    } else {
        None
    };

    // Atomic write via temp + rename.
    let tmp = target.with_extension("twinwrite-tmp");
    if let Err(e) = std::fs::write(&tmp, content) {
        return mark_failed(
            http,
            stdb_host,
            hex_db,
            action.id,
            &format!("tmp write: {}", e),
        )
        .await;
    }
    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return mark_failed(
            http,
            stdb_host,
            hex_db,
            action.id,
            &format!("rename to target: {}", e),
        )
        .await;
    }

    // ADR-2605110700 R1 — hard verifier gate for Rust source files.
    // Run cargo_check on the affected crate immediately after write. If
    // errors, roll back to pre-write state and mark action failed. This
    // turns the cargo_check chain from advisory into authoritative.
    //
    // Disable via HEX_DISABLE_CARGO_GATE=1 for forensic situations.
    let gate_enabled = std::env::var("HEX_DISABLE_CARGO_GATE")
        .map(|v| v != "1" && !v.eq_ignore_ascii_case("true"))
        .unwrap_or(true);
    if gate_enabled && rel_path.ends_with(".rs") {
        if let Some(crate_name) = infer_rust_crate(rel_path) {
            let check_tool = crate::tools::cargo_check::CargoCheck;
            let check_input = serde_json::json!({ "crate": crate_name });
            use crate::tools::Tool;
            let check_result = check_tool.execute(check_input).await;
            let has_errors = check_result
                .output
                .get("errors")
                .and_then(|e| e.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false);
            if has_errors {
                // Roll back
                let errors_summary = check_result
                    .output
                    .get("errors")
                    .and_then(|e| e.as_array())
                    .map(|arr| {
                        arr.iter()
                            .take(3)
                            .map(|v| v.to_string())
                            .collect::<Vec<_>>()
                            .join(" | ")
                    })
                    .unwrap_or_default();
                if let Some(backup) = pre_write_backup {
                    let _ = std::fs::write(&target, backup);
                } else {
                    let _ = std::fs::remove_file(&target);
                }
                tracing::warn!(
                    action_id = action.id,
                    path = %target.display(),
                    crate_name = crate_name,
                    "action_executor: cargo_check rejected patch — rolled back"
                );
                return mark_failed(
                    http,
                    stdb_host,
                    hex_db,
                    action.id,
                    &format!(
                        "ADR-2605110700 R1: cargo_check failed on {} after write — rolled back. Errors: {}",
                        crate_name,
                        errors_summary.chars().take(800).collect::<String>()
                    ),
                )
                .await;
            }
        }
    }

    let evidence = format!(
        "auto-executed by ceo-twin: wrote {} ({} bytes)",
        target.display(),
        content.len()
    );
    tracing::info!(
        action_id = action.id,
        path = %target.display(),
        bytes = content.len(),
        "action_executor: file_write succeeded"
    );

    // Mark executed.
    let url = format!(
        "{}/v1/database/{}/call/proposed_action_mark_executed",
        stdb_host, hex_db
    );
    let body = serde_json::json!([action.id, true, "", evidence.clone()]);
    let resp = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("mark_executed http: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "mark_executed HTTP {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        ));
    }

    // Satisfy the originating commitment.
    if action.related_commitment_id > 0 {
        let satisfy_url = format!("{}/v1/database/{}/call/commitment_satisfy", stdb_host, hex_db);
        let satisfy_body = serde_json::json!([action.related_commitment_id, evidence]);
        if let Err(e) = http
            .post(&satisfy_url)
            .json(&satisfy_body)
            .send()
            .await
        {
            tracing::debug!(commitment_id = action.related_commitment_id, error = %e, "commitment_satisfy http error");
        }
    }
    Ok(())
}

async fn mark_failed(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    id: u64,
    error: &str,
) -> Result<(), String> {
    let url = format!(
        "{}/v1/database/{}/call/proposed_action_mark_executed",
        stdb_host, hex_db
    );
    let body = serde_json::json!([id, false, error, ""]);
    let resp = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("mark_failed http: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "mark_failed HTTP {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        ));
    }
    tracing::warn!(action_id = id, error, "action_executor: marked failed");
    Ok(())
}

/// ADR-2605110700 R1 helper — derive workspace crate name from a repo-
/// relative path so we can scope cargo_check to one crate.
fn infer_rust_crate(rel_path: &str) -> Option<&'static str> {
    if rel_path.starts_with("hex-nexus/src/") { Some("hex-nexus") }
    else if rel_path.starts_with("hex-cli/src/") { Some("hex-cli") }
    else if rel_path.starts_with("hex-agent/src/") { Some("hex-agent") }
    else if rel_path.starts_with("hex-core/src/") { Some("hex-core") }
    else if rel_path.starts_with("hex-parser/src/") { Some("hex-parser") }
    else if rel_path.starts_with("hex-analyzer/src/") { Some("hex-analyzer") }
    else if rel_path.starts_with("hex-desktop/src/") { Some("hex-desktop") }
    else { None }
}
