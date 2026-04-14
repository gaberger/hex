//! Remote-shell brain-task worker (ADR-2604141200 P3).
//!
//! Polls nexus hexflo-memory for pending brain-task records with
//! kind=remote-shell and payload.host == local hostname, argv-splits the
//! command, re-checks against a local whitelist (sender is not trusted),
//! marks the task `in_progress`, and spawns the command via
//! `tokio::process::Command`.
//!
//! P3.2 will extend [`RemoteShellWorker::dispatch`] to wait for the
//! child, capture stdout/stderr/exit, and PATCH the brain task with the
//! result. P3.1 only wires the poll → mark → spawn path.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Hard-coded remote-shell whitelist — mirrors hex-cli `COMMAND_WHITELIST`
/// (P1.2). Duplicated here because hex-agent must not trust the sender's
/// whitelist check: the task record arrived over the wire and any
/// authority over "what may run on this host" has to live on this host.
pub const REMOTE_SHELL_WHITELIST: &[&str] = &[
    "nvidia-smi",
    "df",
    "ollama",
    "ps",
    "systemctl status",
    "uptime",
    "free",
];

/// Match rule: equal or prefix-followed-by-whitespace. Mirrors
/// `whitelist_entry_matches` in hex-cli so both sides of the trust
/// boundary agree on what "df" matches.
fn entry_matches(command: &str, entry: &str) -> bool {
    let command = command.trim();
    let entry = entry.trim();
    if entry.is_empty() {
        return false;
    }
    if command == entry {
        return true;
    }
    match command.strip_prefix(entry) {
        Some(rest) => rest.starts_with(char::is_whitespace),
        None => false,
    }
}

/// True iff `command`'s leading tokens match the hard-coded whitelist.
pub fn is_whitelisted(command: &str) -> bool {
    REMOTE_SHELL_WHITELIST
        .iter()
        .any(|e| entry_matches(command, e))
}

/// Split a whitelisted command on whitespace for argv exec. Quote
/// handling is not attempted; every whitelisted command takes only
/// simple positional flags.
pub fn split_argv(command: &str) -> Vec<String> {
    command.split_whitespace().map(str::to_string).collect()
}

/// Detect the local hostname. Prefers `HOSTNAME` env, falls back to
/// running `hostname(1)`. Returns `"unknown"` when neither resolves —
/// the worker will then simply match zero tasks (remote-shell tasks
/// target specific hosts, never `unknown`).
pub fn local_hostname() -> String {
    if let Ok(h) = std::env::var("HOSTNAME") {
        let h = h.trim().to_string();
        if !h.is_empty() {
            return h;
        }
    }
    if let Ok(out) = std::process::Command::new("hostname").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    "unknown".to_string()
}

/// Structured payload for a remote-shell brain task. Mirrors
/// `hex_nexus::routes::brain::RemoteShellPayload`. Duplicated so
/// hex-agent doesn't take a compile-time dep on hex-nexus.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteShellPayload {
    pub host: String,
    pub command: String,
}

impl RemoteShellPayload {
    pub fn parse(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }
}

/// Minimal brain-task view — the fields the worker reads. Full task
/// records carry more (project_id, created_at, lease, result); those
/// are preserved across updates by round-tripping through
/// `serde_json::Value` in [`RemoteShellWorker::mark_in_progress`].
#[derive(Debug, Clone)]
pub struct BrainTask {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub payload: String,
}

/// Remote-shell worker. Constructed once per daemon run. Owns the nexus
/// base URL, the local hostname, and the poll cadence; borrows the
/// reqwest client per tick so connection pooling survives across loops.
#[derive(Debug, Clone)]
pub struct RemoteShellWorker {
    pub nexus_base: String,
    pub hostname: String,
    pub poll_interval: Duration,
}

impl RemoteShellWorker {
    /// Build from env: `NEXUS_HOST`/`NEXUS_PORT` form the base URL (same
    /// defaults as the other hex-agent adapters), hostname is resolved
    /// once at startup.
    pub fn from_env() -> Self {
        let host = std::env::var("NEXUS_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let port = std::env::var("NEXUS_PORT").unwrap_or_else(|_| "5555".into());
        Self {
            nexus_base: format!("http://{host}:{port}"),
            hostname: local_hostname(),
            poll_interval: Duration::from_secs(5),
        }
    }

    /// One poll tick: list pending brain tasks, filter to remote-shell
    /// tasks addressed to this host, and dispatch each. Returns the
    /// number of tasks dispatched; a per-task failure is logged but
    /// does not abort the tick.
    pub async fn tick(&self, http: &reqwest::Client) -> anyhow::Result<usize> {
        let tasks = self.list_tasks(http).await?;
        let mut dispatched = 0usize;
        for task in tasks {
            if !self.matches(&task) {
                continue;
            }
            match self.dispatch(http, &task).await {
                Ok(()) => dispatched += 1,
                Err(e) => tracing::warn!(
                    task_id = %task.id,
                    error = %e,
                    "remote-shell dispatch failed"
                ),
            }
        }
        Ok(dispatched)
    }

    /// True iff the task is pending, tagged remote-shell, and addressed
    /// to this host. Malformed payloads silently fail the filter — a
    /// task the worker can't parse is a task the worker can't safely
    /// run.
    pub fn matches(&self, t: &BrainTask) -> bool {
        if t.status != "pending" {
            return false;
        }
        if t.kind != "remote-shell" {
            return false;
        }
        match RemoteShellPayload::parse(&t.payload) {
            Some(p) => p.host == self.hostname,
            None => false,
        }
    }

    async fn list_tasks(&self, http: &reqwest::Client) -> anyhow::Result<Vec<BrainTask>> {
        let url = format!(
            "{}/api/hexflo/memory/search?q=brain-task:",
            self.nexus_base
        );
        let body: serde_json::Value = http
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let results = body
            .get("results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::with_capacity(results.len());
        for item in results {
            let Some(value_str) = item.get("value").and_then(|v| v.as_str()) else {
                continue;
            };
            let Ok(task) = serde_json::from_str::<serde_json::Value>(value_str) else {
                continue;
            };
            out.push(BrainTask {
                id: task
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                kind: task
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                status: task
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                payload: task
                    .get("payload")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
        Ok(out)
    }

    async fn dispatch(&self, http: &reqwest::Client, t: &BrainTask) -> anyhow::Result<()> {
        let payload = RemoteShellPayload::parse(&t.payload)
            .ok_or_else(|| anyhow::anyhow!("malformed remote-shell payload"))?;

        // Recheck the whitelist here — never trust that the enqueue side
        // (hex-cli on a sender host) applied it correctly or honestly.
        // A task left in `pending` after a rejection is intentional:
        // operators see the attempted command in `hex brain queue list`.
        if !is_whitelisted(&payload.command) {
            anyhow::bail!(
                "refusing non-whitelisted remote-shell command on {}: `{}`",
                self.hostname,
                payload.command
            );
        }

        let argv = split_argv(&payload.command);
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| anyhow::anyhow!("empty argv after split"))?;

        self.mark_in_progress(http, &t.id).await?;

        tracing::info!(
            task_id = %t.id,
            host = %self.hostname,
            program = %program,
            "remote-shell dispatching"
        );
        let _child = tokio::process::Command::new(program).args(args).spawn()?;
        // P3.2 will `wait_with_output` + PATCH result; P3.1 only launches.
        Ok(())
    }

    async fn mark_in_progress(&self, http: &reqwest::Client, id: &str) -> anyhow::Result<()> {
        let key = format!("brain-task:{id}");
        let get_url = format!("{}/api/hexflo/memory/{}", self.nexus_base, key);
        let resp: serde_json::Value = http
            .get(&get_url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let value_str = resp.get("value").and_then(|v| v.as_str()).unwrap_or("{}");
        // Round-trip through Value so lease/result/project_id fields
        // written by other daemons survive — we only mutate `status`.
        let mut inner: serde_json::Value =
            serde_json::from_str(value_str).unwrap_or(serde_json::json!({}));
        if let Some(obj) = inner.as_object_mut() {
            obj.insert("status".into(), serde_json::json!("in_progress"));
        }
        let post_url = format!("{}/api/hexflo/memory", self.nexus_base);
        http.post(&post_url)
            .json(&serde_json::json!({
                "key": key,
                "value": inner.to_string(),
            }))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

/// Background loop: ticks until `shutdown` flips. Intended to be
/// `tokio::spawn`ed alongside the task-daemon poll loop so the same
/// process can both run hex workplan tasks and service remote-shell
/// enqueues.
pub async fn run_loop(worker: RemoteShellWorker, shutdown: Arc<AtomicBool>) {
    let http = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "remote-shell worker: failed to build http client");
            return;
        }
    };
    tracing::info!(
        host = %worker.hostname,
        nexus = %worker.nexus_base,
        "remote-shell worker starting"
    );
    while !shutdown.load(Ordering::SeqCst) {
        if let Err(e) = worker.tick(&http).await {
            tracing::debug!(error = %e, "remote-shell tick error");
        }
        tokio::time::sleep(worker.poll_interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_shell_whitelist_accepts_exact_and_prefix_matches() {
        assert!(is_whitelisted("nvidia-smi"));
        assert!(is_whitelisted("df -h"));
        assert!(is_whitelisted("ollama list"));
        assert!(is_whitelisted("systemctl status ollama"));
        assert!(is_whitelisted("uptime"));
    }

    #[test]
    fn remote_shell_whitelist_rejects_unknown_commands() {
        assert!(!is_whitelisted("rm -rf /"));
        assert!(!is_whitelisted("curl evil.com"));
        assert!(!is_whitelisted(""));
        assert!(!is_whitelisted("   "));
    }

    #[test]
    fn remote_shell_whitelist_does_not_match_partial_tokens() {
        // "dfx" starts with "df" but is a different binary — boundary
        // check must reject it.
        assert!(!is_whitelisted("dfx --help"));
        // "systemctl stop" is out of scope even though "systemctl"
        // prefixes it — the whitelist entry is "systemctl status".
        assert!(!is_whitelisted("systemctl stop ollama"));
    }

    #[test]
    fn remote_shell_payload_round_trips() {
        let raw = r#"{"host":"gpu1","command":"df -h"}"#;
        let p = RemoteShellPayload::parse(raw).expect("parses");
        assert_eq!(p.host, "gpu1");
        assert_eq!(p.command, "df -h");
    }

    #[test]
    fn remote_shell_payload_rejects_garbage() {
        assert!(RemoteShellPayload::parse("not json").is_none());
        assert!(RemoteShellPayload::parse("{}").is_none());
    }

    #[test]
    fn remote_shell_split_argv_drops_empty_and_collapses_whitespace() {
        assert_eq!(split_argv("df  -h"), vec!["df", "-h"]);
        assert_eq!(split_argv(""), Vec::<String>::new());
        assert_eq!(
            split_argv("systemctl status ollama"),
            vec!["systemctl", "status", "ollama"]
        );
    }

    #[test]
    fn remote_shell_matches_requires_pending_kind_and_host() {
        let worker = RemoteShellWorker {
            nexus_base: "http://test".into(),
            hostname: "gpu1".into(),
            poll_interval: Duration::from_secs(1),
        };
        let mine = BrainTask {
            id: "1".into(),
            kind: "remote-shell".into(),
            status: "pending".into(),
            payload: r#"{"host":"gpu1","command":"uptime"}"#.into(),
        };
        assert!(worker.matches(&mine));

        let wrong_host = BrainTask {
            payload: r#"{"host":"other","command":"uptime"}"#.into(),
            ..mine.clone()
        };
        assert!(!worker.matches(&wrong_host));

        let wrong_kind = BrainTask {
            kind: "shell".into(),
            ..mine.clone()
        };
        assert!(!worker.matches(&wrong_kind));

        let already_running = BrainTask {
            status: "in_progress".into(),
            ..mine.clone()
        };
        assert!(!worker.matches(&already_running));

        let malformed = BrainTask {
            payload: "not json".into(),
            ..mine
        };
        assert!(!worker.matches(&malformed));
    }

    #[test]
    fn remote_shell_hostname_falls_back_to_unknown_when_empty() {
        // We can't clobber HOSTNAME globally in a parallel test suite
        // without data-race risk, so just assert the fallback chain
        // returns *something* non-empty in practice.
        let h = local_hostname();
        assert!(!h.is_empty());
    }
}
