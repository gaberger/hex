//! Auto-reconcile completed brain-chat dispatches against the workplans
//! they reference.
//!
//! Operator pain point that motivated this:
//!   1. PM-agent gets "@pm-agent unblock wp-foo P1.1: do X"
//!   2. PM dispatches @hex-coder, who actually does X via worker pool
//!   3. inference_task row goes Pending → InProgress → Completed
//!   4. ...but the workplan task wp-foo P1.1 stays `blocked` forever and
//!      keeps appearing in the Decisions Needed panel, even though the
//!      work is done.
//!
//! This poller closes that loop. Every 15s:
//!   - Query inference_task for rows where workplan_id starts with
//!     "brain-chat:" AND status == "Completed".
//!   - Skip rows already auto-resolved (tracked via hexflo memory key
//!     "brain-resolved:<inference_task_id>").
//!   - Parse the prompt for workplan-id references using the regex
//!     `(wp-[a-z0-9-]+)\s+(P\d+(?:\.\d+)?)` — case-insensitive, handles
//!     `wp-foo P1`, `wp-foo P1.1`, `WP-FOO P3.2`.
//!   - For each match, open the workplan JSON, find the matching task,
//!     flip status to "done" with traceability fields:
//!       completed_by_brain_dispatch: <inference_task_id>
//!       completed_at: <RFC3339>
//!     Clear blocked_reason if present.
//!   - Mark the dispatch as resolved in memory.
//!
//! Safety: the resolve action mirrors the operator's manual "Done" button.
//! Operator can revert via "Unblock" or by editing the workplan directly.
//! No file outside docs/workplans/wp-*.json is touched.

use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{info, warn};

use crate::ports::state::{IStatePort, InferenceTaskInfo};
use crate::state::SharedState;

const POLL_INTERVAL_SECS: u64 = 15;
const RESOLVED_KEY_PREFIX: &str = "brain-resolved:";

pub struct BrainDispatchReconciler {
    state: SharedState,
}

impl BrainDispatchReconciler {
    pub fn spawn(state: SharedState) -> JoinHandle<()> {
        tokio::spawn(async move {
            Self { state }.run().await;
        })
    }

    async fn run(self) {
        let mut interval = time::interval(Duration::from_secs(POLL_INTERVAL_SECS));
        info!("brain dispatch reconciler started (poll {}s)", POLL_INTERVAL_SECS);
        loop {
            interval.tick().await;
            if let Err(e) = self.tick().await {
                warn!("brain dispatch reconciler tick error: {}", e);
            }
        }
    }

    async fn tick(&self) -> Result<(), String> {
        let port: &Arc<dyn IStatePort> = match self.state.state_port.as_ref() {
            Some(p) => p,
            None => return Ok(()),
        };

        let rows = port
            .inference_task_list_all()
            .await
            .map_err(|e| format!("inference_task_list_all: {}", e))?;

        let candidates: Vec<&InferenceTaskInfo> = rows
            .iter()
            .filter(|r| r.workplan_id.starts_with("brain-chat:"))
            .filter(|r| r.status == "Completed")
            .collect();

        if candidates.is_empty() {
            return Ok(());
        }

        let scan_root = std::env::current_dir()
            .map_err(|e| format!("cwd: {}", e))?;
        let workplans_dir = scan_root.join("docs/workplans");

        for c in candidates {
            // Already resolved? Skip.
            let memo_key = format!("{}{}", RESOLVED_KEY_PREFIX, c.id);
            let already = port.hexflo_memory_retrieve(&memo_key).await.ok().flatten();
            if already.is_some() {
                continue;
            }

            let refs = parse_workplan_refs(&c.prompt);
            if refs.is_empty() {
                // Nothing to reconcile; record so we don't re-scan every tick.
                let _ = port
                    .hexflo_memory_store(
                        &memo_key,
                        &serde_json::json!({
                            "status": "no-workplan-ref",
                            "ts": chrono::Utc::now().to_rfc3339(),
                        })
                        .to_string(),
                        "system",
                    )
                    .await;
                continue;
            }

            let mut resolved_pairs: Vec<(String, String)> = Vec::new();
            for (wp_id, task_id) in refs {
                let path = workplans_dir.join(format!("{}.json", wp_id));
                match try_complete_task(&path, &task_id, &c.id) {
                    Ok(true) => {
                        info!(
                            "brain reconciler: marked {} {} done from dispatch {}",
                            wp_id, task_id, c.id
                        );
                        resolved_pairs.push((wp_id, task_id));
                    }
                    Ok(false) => {
                        // Task not found OR already done — fine, don't retry.
                    }
                    Err(e) => {
                        warn!(
                            "brain reconciler: workplan {} task {} update failed: {}",
                            wp_id, task_id, e
                        );
                    }
                }
            }

            // Mark processed even if no edit happened (e.g. workplan file
            // doesn't exist) so we don't re-attempt every tick.
            let payload = serde_json::json!({
                "status": if resolved_pairs.is_empty() { "no-match" } else { "resolved" },
                "resolved": resolved_pairs.iter().map(|(w, t)| format!("{}/{}", w, t)).collect::<Vec<_>>(),
                "ts": chrono::Utc::now().to_rfc3339(),
            });
            let _ = port
                .hexflo_memory_store(&memo_key, &payload.to_string(), "system")
                .await;
        }

        Ok(())
    }
}

/// Parse free-form text for workplan-id + phase/task references.
///
/// Two-pass: collect ALL wp-ids and ALL P-refs in the text, then form the
/// cartesian product of the two sets. The downstream `try_complete_task`
/// only updates a task that actually exists in the named workplan, so
/// false combinations are no-ops. This is much more forgiving than
/// requiring adjacency — agents like to say things like
///   "Find the workplan `wp-foo` (try docs/workplans/...) and complete P1.1"
/// where the P-ref is far from the wp-id mention.
///
/// Patterns recognized (case-insensitive):
///   wp-foo, wp-foo-bar, `wp-foo`, "wp-foo"
///   P1, P1.1, P3.2 (must be `P` then digits, optionally `.<digits>`)
/// Returns deduped (wp_id, task_id) pairs.
fn parse_workplan_refs(text: &str) -> Vec<(String, String)> {
    use std::collections::HashSet;
    let lower = text.to_ascii_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ':' | '`' | '"' | '\'' | '(' | ')' | '*' | '['  | ']'))
        .filter(|t| !t.is_empty())
        .collect();

    let mut wp_ids: Vec<String> = Vec::new();
    let mut task_ids: Vec<String> = Vec::new();
    let mut wp_seen: HashSet<String> = HashSet::new();
    let mut task_seen: HashSet<String> = HashSet::new();

    for tok in tokens {
        // wp-id detection: token starts with "wp-", strip trailing punct
        if tok.starts_with("wp-") && tok.len() > 3 {
            let trimmed = tok.trim_end_matches(|c: char| matches!(c, '.' | ',' | ':' | '!' | '?' | '/'));
            if is_valid_wp_id(trimmed) && wp_seen.insert(trimmed.to_string()) {
                wp_ids.push(trimmed.to_string());
            }
            continue;
        }
        // P-ref detection: starts with 'p', then digits, optionally '.digits...'
        let trimmed = tok.trim_end_matches(|c: char| matches!(c, '.' | ',' | ':' | '!' | '?'));
        if trimmed.len() < 2 || !trimmed.starts_with('p') { continue; }
        let rest = &trimmed[1..];
        if rest.is_empty() { continue; }
        if !rest.chars().next().unwrap().is_ascii_digit() { continue; }
        if !rest.chars().all(|c| c.is_ascii_digit() || c == '.') { continue; }
        let task_id = format!("P{}", rest);
        if task_seen.insert(task_id.clone()) {
            task_ids.push(task_id);
        }
    }

    // Cartesian product. Most pairs will not match a real task, and that's fine —
    // try_complete_task returns Ok(false) on mismatch and we skip silently.
    let mut out: Vec<(String, String)> = Vec::with_capacity(wp_ids.len() * task_ids.len());
    for w in &wp_ids {
        for t in &task_ids {
            out.push((w.clone(), t.clone()));
        }
    }
    out
}

fn is_valid_wp_id(s: &str) -> bool {
    s.starts_with("wp-")
        && s.len() > 3
        && s[3..].chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Open a workplan JSON, find the named task across all phases, flip
/// its status to "done" with traceability fields. Returns Ok(true) if a
/// matching task was updated, Ok(false) if no matching task or already done,
/// Err on filesystem / parse errors.
fn try_complete_task(path: &std::path::Path, task_id: &str, dispatch_id: &str) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }
    let raw = std::fs::read_to_string(path).map_err(|e| format!("read: {}", e))?;
    let mut doc: serde_json::Value = serde_json::from_str(&raw).map_err(|e| format!("parse: {}", e))?;

    let mut updated = false;
    if let Some(phases) = doc.get_mut("phases").and_then(|v| v.as_array_mut()) {
        for phase in phases {
            if let Some(tasks) = phase.get_mut("tasks").and_then(|v| v.as_array_mut()) {
                for task in tasks {
                    let id_match = task
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.eq_ignore_ascii_case(task_id))
                        .unwrap_or(false);
                    if !id_match {
                        continue;
                    }
                    let current_status = task.get("status").and_then(|v| v.as_str()).unwrap_or("");
                    if current_status == "done" || current_status == "completed" {
                        continue; // already resolved
                    }
                    if let Some(obj) = task.as_object_mut() {
                        obj.insert("status".to_string(), serde_json::json!("done"));
                        obj.remove("blocked_reason");
                        obj.insert(
                            "completed_by_brain_dispatch".to_string(),
                            serde_json::json!(dispatch_id),
                        );
                        obj.insert(
                            "completed_at".to_string(),
                            serde_json::json!(chrono::Utc::now().to_rfc3339()),
                        );
                        updated = true;
                    }
                }
            }
        }
    }
    if !updated {
        return Ok(false);
    }
    let serialized = serde_json::to_string_pretty(&doc).map_err(|e| format!("serialize: {}", e))?;
    std::fs::write(path, serialized).map_err(|e| format!("write: {}", e))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_wp_ref() {
        let r = parse_workplan_refs("please complete wp-foo P1.1 today");
        assert_eq!(r, vec![("wp-foo".to_string(), "P1.1".to_string())]);
    }

    #[test]
    fn parses_phase_only_ref() {
        let r = parse_workplan_refs("wp-bar P3 should be done");
        assert_eq!(r, vec![("wp-bar".to_string(), "P3".to_string())]);
    }

    #[test]
    fn parses_multiple_dashes() {
        let r = parse_workplan_refs("see wp-safe-file-writer-adapter P2.1");
        assert_eq!(r, vec![("wp-safe-file-writer-adapter".to_string(), "P2.1".to_string())]);
    }

    #[test]
    fn case_insensitive() {
        let r = parse_workplan_refs("WP-Foo P1.1");
        assert_eq!(r, vec![("wp-foo".to_string(), "P1.1".to_string())]);
    }

    #[test]
    fn ignores_random_p_words() {
        let r = parse_workplan_refs("wp-foo punch the planet");
        assert!(r.is_empty(), "Got: {:?}", r);
    }

    #[test]
    fn handles_non_adjacent_pref() {
        // The actual prompt that broke the strict-adjacency parser.
        let r = parse_workplan_refs(
            "Find the workplan file matching the slug `wp-safe-file-writer-adapter` (try docs/workplans/...) and complete P1.1"
        );
        assert!(r.contains(&("wp-safe-file-writer-adapter".to_string(), "P1.1".to_string())),
            "Got: {:?}", r);
    }

    #[test]
    fn cartesian_product_two_wps_two_prefs() {
        let r = parse_workplan_refs("see wp-foo and wp-bar — both touch P1.1 and P2");
        // 2 wps × 2 P-refs = 4 pairs
        assert_eq!(r.len(), 4);
        assert!(r.contains(&("wp-foo".to_string(), "P1.1".to_string())));
        assert!(r.contains(&("wp-bar".to_string(), "P2".to_string())));
    }

    #[test]
    fn dedupes_repeated_refs() {
        let r = parse_workplan_refs("wp-foo P1.1 again wp-foo P1.1");
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn ignores_no_wp_prefix() {
        let r = parse_workplan_refs("just-a-name P1.1");
        assert!(r.is_empty());
    }
}
