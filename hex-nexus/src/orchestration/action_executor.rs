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
        "adr_status_set" => execute_adr_status_set(http, stdb_host, hex_db, repo_root, action).await,
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
    if gate_enabled && (rel_path.ends_with(".tsx") || rel_path.ends_with(".ts")) {
        // ADR-2605141631 follow-up — TypeScript writes get the same
        // compile-or-rollback discipline as Rust. Without this, the
        // 2026-05-14 dogfood run on AttentionFeed.tsx landed a file
        // with a hallucinated `import { AttentionItem } from './types'`
        // because no gate caught it.
        let ts_tool = crate::tools::typescript_check::TypescriptCheck;
        let ts_input = serde_json::json!({});
        use crate::tools::Tool;
        let ts_result = ts_tool.execute(ts_input).await;
        let has_errors = !ts_result.ok
            || ts_result
                .output
                .get("errors")
                .and_then(|e| e.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false);
        if has_errors {
            let errors_summary = ts_result
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
            if let Some(backup) = pre_write_backup.clone() {
                let _ = std::fs::write(&target, backup);
            } else {
                let _ = std::fs::remove_file(&target);
            }
            tracing::warn!(
                action_id = action.id,
                path = %target.display(),
                "action_executor: typescript_check rejected patch — rolled back"
            );
            return mark_failed(
                http,
                stdb_host,
                hex_db,
                action.id,
                &format!(
                    "ADR-2605141631 R1: typescript_check failed after write — rolled back. Errors: {}",
                    errors_summary.chars().take(800).collect::<String>()
                ),
            )
            .await;
        }
    }
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

    // Autonomous commit step — bridges the gap between "executor wrote
    // file to working tree" and "change persisted on main". Without
    // this, every approved file_write sits uncommitted until an operator
    // manually `git add`+`git commit`s it (the agentic-dev-roundtrip
    // spec on 2026-05-13 landed via SOP but required operator-Claude
    // commit `8e929b58` to actually persist). With this, the SOP loop
    // closes end-to-end without a human in the commit step.
    //
    // Failures are logged but do NOT fail the action — the executor
    // already marked it `executed`, the file is on disk, operator can
    // commit by hand if the auto-commit hit a pre-commit hook failure
    // or a concurrent git lock.
    //
    // Disable via HEX_DISABLE_AUTONOMOUS_COMMIT=1.
    if let Err(e) = git_commit_executed_file(repo_root, rel_path, content.len(), action, &evidence)
        .await
    {
        tracing::warn!(
            action_id = action.id,
            path = %target.display(),
            error = %e,
            "action_executor: autonomous commit step failed (file still on disk; operator may commit manually)"
        );
    }

    Ok(())
}

/// Auto-stage and commit a single file the executor just wrote. Stages
/// ONLY the named path (never `git add -A`), commits with `--only` so
/// concurrent staging of other paths is ignored, runs pre-commit hooks
/// normally (never `--no-verify`). The commit author is whatever git is
/// configured for; an explicit `Co-Authored-By: hex-autonomous` footer
/// distinguishes these from operator-driven commits.
async fn git_commit_executed_file(
    repo_root: &Path,
    rel_path: &str,
    content_bytes: usize,
    action: &ApprovedAction,
    evidence: &str,
) -> Result<String, String> {
    let disabled = std::env::var("HEX_DISABLE_AUTONOMOUS_COMMIT")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if disabled {
        return Err("HEX_DISABLE_AUTONOMOUS_COMMIT=1".to_string());
    }

    // Safety: never auto-commit env files, secrets, lock files, hub.db.
    // Per CLAUDE.md "NEVER commit secrets" + the SQLite-removal lesson.
    if is_no_autocommit_path(rel_path) {
        return Err(format!(
            "path '{}' is on the autonomous-commit denylist (env/secret/sqlite/lock)",
            rel_path
        ));
    }

    let (kind, scope) = derive_commit_scope(rel_path);
    let basename = std::path::Path::new(rel_path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| rel_path.to_string());

    let subject = format!(
        "{}({}): auto — action#{} → {}",
        kind, scope, action.id, basename
    );

    let body = format!(
        "Path:           {rel_path}\n\
         Bytes:          {bytes}\n\
         Action ID:      {action_id}\n\
         Commitment ID:  {cid}\n\
         Kind:           {kind}\n\
         Evidence:       {evidence}\n\n\
         Auto-committed by the SOP loop (twin-approved file_write).\n\
         Disable via HEX_DISABLE_AUTONOMOUS_COMMIT=1.\n\n\
         Co-Authored-By: hex-autonomous <noreply@hex.local>\n",
        rel_path = rel_path,
        bytes = content_bytes,
        action_id = action.id,
        cid = action.related_commitment_id,
        kind = action.kind,
        evidence = evidence,
    );
    let message = format!("{}\n\n{}", subject, body);

    // Verify the file actually changed in git's view before committing.
    // If the executor wrote bit-identical content (idempotent re-emit),
    // `git commit --only -- <path>` would create an empty commit which
    // we don't want.
    let status_out = tokio::process::Command::new("git")
        .current_dir(repo_root)
        .args(["status", "--porcelain", "--", rel_path])
        .output()
        .await
        .map_err(|e| format!("git status: {}", e))?;
    if !status_out.status.success() {
        return Err(format!(
            "git status failed: {}",
            String::from_utf8_lossy(&status_out.stderr).trim()
        ));
    }
    if status_out.stdout.is_empty() {
        return Err(format!(
            "no-op: '{}' has no git diff (file matches HEAD)",
            rel_path
        ));
    }

    // Stage + commit ONLY this file. `--only -- <path>` makes the commit
    // ignore anything else already in the index, so concurrent operator
    // staging doesn't leak into this commit.
    let stage = tokio::process::Command::new("git")
        .current_dir(repo_root)
        .args(["add", "--", rel_path])
        .output()
        .await
        .map_err(|e| format!("git add: {}", e))?;
    if !stage.status.success() {
        return Err(format!(
            "git add failed: {}",
            String::from_utf8_lossy(&stage.stderr).trim()
        ));
    }

    // `-m <msg>` must come BEFORE `-- <pathspec>` or git treats `-m` as
    // another pathspec (`error: pathspec '-m' did not match any file(s)`).
    // Observed 2026-05-14 on the first smoke run after the operator-
    // passthrough bypass made the loop reach this step.
    let commit = tokio::process::Command::new("git")
        .current_dir(repo_root)
        .args(["commit", "--only", "-m", &message, "--", rel_path])
        .output()
        .await
        .map_err(|e| format!("git commit: {}", e))?;
    if !commit.status.success() {
        // Pre-commit hook failure or other rejection. Unstage so the
        // operator's working tree isn't polluted with our stage.
        let _ = tokio::process::Command::new("git")
            .current_dir(repo_root)
            .args(["reset", "HEAD", "--", rel_path])
            .output()
            .await;
        return Err(format!(
            "git commit failed: {}",
            String::from_utf8_lossy(&commit.stderr).trim()
        ));
    }

    let head = tokio::process::Command::new("git")
        .current_dir(repo_root)
        .args(["rev-parse", "HEAD"])
        .output()
        .await
        .map_err(|e| format!("git rev-parse: {}", e))?;
    let sha = String::from_utf8_lossy(&head.stdout).trim().to_string();
    tracing::info!(
        action_id = action.id,
        path = %rel_path,
        sha = %sha,
        subject = %subject,
        "action_executor: autonomous commit landed"
    );
    Ok(sha)
}

fn derive_commit_scope(rel_path: &str) -> (&'static str, &'static str) {
    if rel_path.starts_with("docs/adrs/") {
        ("docs", "adr")
    } else if rel_path.starts_with("docs/specs/") {
        ("docs", "spec")
    } else if rel_path.starts_with("docs/workplans/") {
        ("docs", "workplan")
    } else if rel_path.starts_with("docs/analysis/") {
        ("docs", "analysis")
    } else if rel_path.starts_with("docs/") {
        ("docs", "misc")
    } else if rel_path.starts_with("hex-cli/") {
        ("feat", "hex-cli")
    } else if rel_path.starts_with("hex-nexus/") {
        ("feat", "hex-nexus")
    } else if rel_path.starts_with("hex-core/") {
        ("feat", "hex-core")
    } else if rel_path.starts_with("hex-agent/") {
        ("feat", "hex-agent")
    } else if rel_path.starts_with("hex-parser/") {
        ("feat", "hex-parser")
    } else if rel_path.starts_with("hex-desktop/") {
        ("feat", "hex-desktop")
    } else if rel_path.starts_with("hex-analyzer/") {
        ("feat", "hex-analyzer")
    } else if rel_path.starts_with("spacetime-modules/") {
        ("feat", "stdb")
    } else if rel_path.starts_with("scripts/") {
        ("chore", "scripts")
    } else {
        ("chore", "misc")
    }
}

fn is_no_autocommit_path(rel_path: &str) -> bool {
    let p = rel_path.to_ascii_lowercase();
    // Env / secret files
    if p == ".env" || p.starts_with(".env.") || p.ends_with("/.env") || p.contains("/.env.") {
        return true;
    }
    if p.contains("secret") || p.contains("credential") || p.contains("password") {
        return true;
    }
    // SQLite / lock files — STDB is sole backend per memory feedback_no_sqlite
    if p.ends_with(".db") || p == "hub.db" || p.ends_with(".db-journal") || p.ends_with(".sqlite") {
        return true;
    }
    if p.ends_with(".lock") || p == "cargo.lock" {
        return true;
    }
    // Git internals
    if p.starts_with(".git/") {
        return true;
    }
    // The action_executor's own temp files
    if p.ends_with(".twinwrite-tmp") || p.ends_with(".stubwrite-tmp") {
        return true;
    }
    false
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

/// ADR-2605121505 — execute an `adr_status_set` action.
///
/// Mutates a single ADR file under `docs/adrs/` by rewriting its `Status:`
/// line and inserting a dated reason line after the `Date:` line. Refuses
/// any of: target outside `docs/adrs/`, file not found, current status
/// != Proposed, status line not parseable.
async fn execute_adr_status_set(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    repo_root: &Path,
    action: &ApprovedAction,
) -> Result<(), String> {
    let payload: serde_json::Value = match serde_json::from_str(&action.payload_json) {
        Ok(v) => v,
        Err(e) => {
            return mark_failed(http, stdb_host, hex_db, action.id, &format!("payload parse: {}", e)).await;
        }
    };
    let adr_id = match payload.get("adr_id").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return mark_failed(http, stdb_host, hex_db, action.id, "missing adr_id").await,
    };
    let new_status = match payload.get("new_status").and_then(|v| v.as_str()) {
        Some(s) if matches!(s, "Accepted" | "Abandoned" | "Superseded") => s.to_string(),
        _ => return mark_failed(http, stdb_host, hex_db, action.id, "invalid new_status").await,
    };
    let reason = match payload.get("reason").and_then(|v| v.as_str()) {
        Some(s) if !s.trim().is_empty() => s.to_string(),
        _ => return mark_failed(http, stdb_host, hex_db, action.id, "missing reason").await,
    };

    let adr_dir = repo_root.join("docs/adrs");
    let adr_file = match resolve_adr_file(&adr_dir, &adr_id) {
        Ok(p) => p,
        Err(e) => {
            return mark_failed(http, stdb_host, hex_db, action.id, &format!("resolve_adr_file: {}", e)).await;
        }
    };

    let canonical_root = match repo_root.canonicalize() {
        Ok(p) => p,
        Err(e) => return Err(format!("canonicalise repo_root: {}", e)),
    };
    let canonical_target = match adr_file.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return mark_failed(http, stdb_host, hex_db, action.id, &format!("canonicalise target: {}", e)).await;
        }
    };
    if !canonical_target.starts_with(&canonical_root) {
        return mark_failed(
            http, stdb_host, hex_db, action.id,
            &format!("refused: {} outside repo root", canonical_target.display()),
        ).await;
    }

    let original = match std::fs::read_to_string(&adr_file) {
        Ok(s) => s,
        Err(e) => {
            return mark_failed(http, stdb_host, hex_db, action.id, &format!("read: {}", e)).await;
        }
    };
    let mutated = match mutate_adr_status(&original, &new_status, &reason, action.related_commitment_id) {
        Ok(s) => s,
        Err(e) => {
            return mark_failed(http, stdb_host, hex_db, action.id, &e).await;
        }
    };

    let tmp = adr_file.with_extension("twinwrite-tmp");
    if let Err(e) = std::fs::write(&tmp, &mutated) {
        return mark_failed(http, stdb_host, hex_db, action.id, &format!("tmp write: {}", e)).await;
    }
    if let Err(e) = std::fs::rename(&tmp, &adr_file) {
        let _ = std::fs::remove_file(&tmp);
        return mark_failed(http, stdb_host, hex_db, action.id, &format!("rename: {}", e)).await;
    }

    let evidence = format!(
        "auto-executed by ceo-twin: flipped {} → {} in {}",
        adr_id,
        new_status,
        adr_file.strip_prefix(&canonical_root).unwrap_or(&adr_file).display()
    );
    tracing::info!(action_id = action.id, adr_id, new_status, "action_executor: adr_status_set succeeded");

    // Mark executed.
    let mark_url = format!("{}/v1/database/{}/call/proposed_action_mark_executed", stdb_host, hex_db);
    let mark_body = serde_json::json!([action.id, true, "", evidence.clone()]);
    let resp = http.post(&mark_url).json(&mark_body).send().await
        .map_err(|e| format!("mark_executed http: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("mark_executed HTTP {}: {}", resp.status(), resp.text().await.unwrap_or_default()));
    }

    if action.related_commitment_id > 0 {
        let satisfy_url = format!("{}/v1/database/{}/call/commitment_satisfy", stdb_host, hex_db);
        let satisfy_body = serde_json::json!([action.related_commitment_id, evidence]);
        if let Err(e) = http.post(&satisfy_url).json(&satisfy_body).send().await {
            tracing::debug!(commitment_id = action.related_commitment_id, error = %e, "commitment_satisfy http error");
        }
    }
    Ok(())
}

/// Find the canonical ADR filename inside `docs/adrs/` by id prefix.
/// e.g. `ADR-2605090100` resolves to `ADR-2605090100-adr-alias-table-...md`.
fn resolve_adr_file(adr_dir: &Path, adr_id: &str) -> Result<PathBuf, String> {
    if !adr_dir.exists() {
        return Err(format!("dir not found: {}", adr_dir.display()));
    }
    let prefix = format!("{}-", adr_id);
    let exact = format!("{}.md", adr_id);
    let entries = std::fs::read_dir(adr_dir)
        .map_err(|e| format!("read_dir: {}", e))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == exact || (name.starts_with(&prefix) && name.ends_with(".md")) {
            return Ok(entry.path());
        }
    }
    Err(format!("no ADR file matching '{}' under {}", adr_id, adr_dir.display()))
}

/// Pure transform: take an ADR file body, rewrite the Status line, and insert
/// a dated reason line after the Date line. Returns an error if the file
/// isn't shaped like an ADR (no Status line, or current status != Proposed).
fn mutate_adr_status(
    original: &str,
    new_status: &str,
    reason: &str,
    commitment_id: u64,
) -> Result<String, String> {
    let mut lines: Vec<String> = original.lines().map(|s| s.to_string()).collect();
    let status_idx = lines
        .iter()
        .position(|l| l.trim_start().starts_with("Status:") && l.contains("**"))
        .ok_or_else(|| "no Status line found".to_string())?;
    let current = &lines[status_idx];
    if !current.contains("**Proposed**") {
        return Err(format!(
            "current status is not Proposed (line: {})",
            current.trim()
        ));
    }
    lines[status_idx] = format!("Status: **{}**", new_status);

    // Insert the reason line after the Date line if present, else immediately
    // after the Status line.
    let insert_after = lines
        .iter()
        .enumerate()
        .skip(status_idx)
        .take(5)
        .find(|(_, l)| l.trim_start().starts_with("Date:"))
        .map(|(i, _)| i)
        .unwrap_or(status_idx);

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let reason_line = format!(
        "{}: {} — {} (autonomous via SOP, commitment {})",
        new_status, today, reason, commitment_id
    );
    lines.insert(insert_after + 1, reason_line);

    let mut out = lines.join("\n");
    if original.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod adr_status_executor_tests {
    use super::{mutate_adr_status, resolve_adr_file};
    use std::fs;
    use std::path::PathBuf;

    fn tmp_dir(tag: &str) -> PathBuf {
        // Per-test directory so concurrent test runs don't clobber each other.
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("hex-adr-test-{}-{}-{}", std::process::id(), tag, n));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn mutates_status_and_inserts_reason() {
        let input = "# ADR-X — Title\n\nStatus: **Proposed**\nDate: 2026-05-10\n\n## Context\nbody\n";
        let out = mutate_adr_status(input, "Accepted", "test reason", 42).unwrap();
        assert!(out.contains("Status: **Accepted**"));
        assert!(out.contains("Accepted: "));
        assert!(out.contains("test reason"));
        assert!(out.contains("commitment 42"));
        assert!(!out.contains("Status: **Proposed**"));
    }

    #[test]
    fn rejects_non_proposed() {
        let input = "# ADR-X\n\nStatus: **Accepted**\nDate: 2026-05-10\n";
        assert!(mutate_adr_status(input, "Accepted", "x", 1).is_err());
    }

    #[test]
    fn rejects_no_status_line() {
        let input = "# ADR-X\n\nDate: 2026-05-10\n";
        assert!(mutate_adr_status(input, "Accepted", "x", 1).is_err());
    }

    #[test]
    fn preserves_trailing_newline() {
        let with_nl = "Status: **Proposed**\nDate: 2026-05-10\n";
        let out = mutate_adr_status(with_nl, "Accepted", "x", 1).unwrap();
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn resolve_finds_canonical_filename() {
        let d = tmp_dir("resolve");
        let target = d.join("ADR-2605090100-foo-bar.md");
        fs::write(&target, "x").unwrap();
        let found = resolve_adr_file(&d, "ADR-2605090100").unwrap();
        assert_eq!(found, target);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn resolve_errors_when_missing() {
        let d = tmp_dir("missing");
        assert!(resolve_adr_file(&d, "ADR-2099-12-31-9999").is_err());
        let _ = fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod autonomous_commit_tests {
    use super::{derive_commit_scope, is_no_autocommit_path};

    #[test]
    fn scope_adr() {
        assert_eq!(
            derive_commit_scope("docs/adrs/ADR-2605131849-foo.md"),
            ("docs", "adr")
        );
    }

    #[test]
    fn scope_spec() {
        assert_eq!(
            derive_commit_scope("docs/specs/agentic-dev-roundtrip.md"),
            ("docs", "spec")
        );
    }

    #[test]
    fn scope_workplan() {
        assert_eq!(
            derive_commit_scope("docs/workplans/wp-x.json"),
            ("docs", "workplan")
        );
    }

    #[test]
    fn scope_analysis() {
        assert_eq!(
            derive_commit_scope("docs/analysis/r.md"),
            ("docs", "analysis")
        );
    }

    #[test]
    fn scope_hex_crates() {
        assert_eq!(
            derive_commit_scope("hex-cli/src/foo.rs"),
            ("feat", "hex-cli")
        );
        assert_eq!(
            derive_commit_scope("hex-nexus/src/orchestration/x.rs"),
            ("feat", "hex-nexus")
        );
        assert_eq!(
            derive_commit_scope("hex-core/src/lib.rs"),
            ("feat", "hex-core")
        );
    }

    #[test]
    fn scope_stdb() {
        assert_eq!(
            derive_commit_scope("spacetime-modules/hexflo-coordination/src/lib.rs"),
            ("feat", "stdb")
        );
    }

    #[test]
    fn scope_misc_fallback() {
        assert_eq!(derive_commit_scope("README.md"), ("chore", "misc"));
        assert_eq!(derive_commit_scope("CLAUDE.md"), ("chore", "misc"));
    }

    #[test]
    fn denylist_env_files() {
        assert!(is_no_autocommit_path(".env"));
        assert!(is_no_autocommit_path(".env.local"));
        assert!(is_no_autocommit_path("foo/.env"));
        assert!(is_no_autocommit_path("foo/.env.prod"));
    }

    #[test]
    fn denylist_secrets() {
        assert!(is_no_autocommit_path("secrets/api.key"));
        assert!(is_no_autocommit_path("docs/credential-rotation.md"));
        assert!(is_no_autocommit_path("config/password.toml"));
    }

    #[test]
    fn denylist_sqlite_and_locks() {
        assert!(is_no_autocommit_path("hub.db"));
        assert!(is_no_autocommit_path("data.sqlite"));
        assert!(is_no_autocommit_path("Cargo.lock"));
        assert!(is_no_autocommit_path("foo.lock"));
    }

    #[test]
    fn denylist_git_internals() {
        assert!(is_no_autocommit_path(".git/HEAD"));
        assert!(is_no_autocommit_path(".git/config"));
    }

    #[test]
    fn denylist_executor_temp_files() {
        assert!(is_no_autocommit_path("docs/specs/foo.twinwrite-tmp"));
        assert!(is_no_autocommit_path("docs/adrs/bar.stubwrite-tmp"));
    }

    #[test]
    fn denylist_does_not_block_normal_paths() {
        assert!(!is_no_autocommit_path("docs/specs/foo.md"));
        assert!(!is_no_autocommit_path("docs/adrs/ADR-x.md"));
        assert!(!is_no_autocommit_path("hex-cli/src/main.rs"));
        assert!(!is_no_autocommit_path("docs/workplans/wp-foo.json"));
    }
}
