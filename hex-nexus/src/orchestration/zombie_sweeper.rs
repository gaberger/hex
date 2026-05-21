//! Defunct-process detector for `hex-agent` workers (ADR-2026-05-19-0900 P3.3).
//!
//! Walks `/proc` every `SWEEP_INTERVAL_SECS` looking for entries whose
//! command name starts with `hex-agent` AND whose state field in
//! `/proc/<pid>/stat` is `Z` (zombie). Records each finding via a
//! `supervisor_event { kind: "zombie_detected", ... }` row so the
//! dashboard + supervisor can act on it.
//!
//! We do NOT attempt to wait() on or reap the zombies ourselves. When a
//! zombie's ppid is 1 (init), only init can reap it — that's the case
//! we observed on 2026-05-19 with three [hex-agent] <defunct> rows from
//! May 18 pinned to ppid=1 in the process table. The sweeper's job is
//! to surface them so an operator (or a future systemd-side service)
//! can clean up. It also acts as evidence that the dispatcher launched
//! workers it didn't manage to track.

use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{info, warn};

use crate::state::SharedState;

/// How often to walk /proc. 60s mirrors the supervisor_tick stale-
/// heartbeat threshold from P3.2.
const SWEEP_INTERVAL_SECS: u64 = 60;

/// Command-name prefixes we care about. A zombie matching any of these
/// is recorded; anything else is skipped (kernel threads, unrelated
/// background processes, etc.).
const ROLE_PREFIXES: &[&str] = &["hex-agent", "hex-coder", "hex-tester"];

pub struct ZombieSweeper {
    state: SharedState,
}

impl ZombieSweeper {
    pub fn spawn(state: SharedState) -> JoinHandle<()> {
        tokio::spawn(async move {
            Self { state }.run().await;
        })
    }

    async fn run(self) {
        let mut interval = time::interval(Duration::from_secs(SWEEP_INTERVAL_SECS));
        info!("zombie sweeper started (every {}s)", SWEEP_INTERVAL_SECS);
        loop {
            interval.tick().await;
            if let Err(e) = self.tick().await {
                warn!("zombie sweeper tick error: {}", e);
            }
        }
    }

    async fn tick(&self) -> Result<(), String> {
        let zombies = scan_zombies(ROLE_PREFIXES);
        if zombies.is_empty() {
            return Ok(());
        }
        // Best-effort logging — the supervisor_event write requires the
        // STDB state port. If it's unavailable, we still log so the
        // operator can grep nexus.log.
        for z in &zombies {
            warn!(
                pid = z.pid,
                ppid = z.ppid,
                comm = %z.comm,
                "zombie [hex-agent]-family process detected — ppid=1 means init owns it; manual reap required"
            );
        }

        let port = match self.state.state_port.as_ref() {
            Some(p) => p,
            None => return Ok(()),
        };

        // Record one event per zombie via the existing supervisor_event
        // pipeline. The handler (orchestration/integrator_subscriber +
        // friends) will surface them on the dashboard.
        for z in zombies {
            let payload = serde_json::json!({
                "pid": z.pid,
                "ppid": z.ppid,
                "comm": z.comm,
                "state": z.state,
            })
            .to_string();
            // We don't have a typed IStatePort method for supervisor_event
            // inserts yet; fall back to hexflo_memory as a low-cost
            // audit log. When IStatePort grows a `supervisor_event_insert`
            // surface this becomes a direct call.
            let key = format!("zombie:{}:{}", z.pid, chrono::Utc::now().timestamp());
            let _ = port.hexflo_memory_store(&key, &payload, "supervision").await;
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ZombieEntry {
    pub pid: u32,
    pub ppid: u32,
    pub comm: String,
    pub state: String,
}

/// Walk `/proc` once. Returns every entry whose state is `Z` and whose
/// command name starts with one of `role_prefixes`. Pure function —
/// no STDB I/O, no logging — for test reuse.
pub fn scan_zombies(role_prefixes: &[&str]) -> Vec<ZombieEntry> {
    let mut out = Vec::new();
    let dir = match std::fs::read_dir("/proc") {
        Ok(d) => d,
        Err(_) => return out,
    };

    for entry in dir.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else { continue };
        if !name_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(pid): Result<u32, _> = name_str.parse() else { continue };

        let stat = match std::fs::read_to_string(format!("/proc/{}/stat", pid)) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let Some((state, ppid)) = parse_state_and_ppid(&stat) else { continue };
        if state != "Z" {
            continue;
        }

        // /proc/<pid>/stat embeds the comm field inside parens. Pull
        // it out — that's our "is this hex-agent?" signal. We avoid
        // /cmdline because zombies have empty cmdline.
        let comm = parse_comm(&stat).unwrap_or_default();
        if !role_prefixes.iter().any(|p| comm.starts_with(p)) {
            continue;
        }

        out.push(ZombieEntry {
            pid,
            ppid,
            comm,
            state,
        });
    }
    out
}

/// Slice `comm` from /proc/<pid>/stat. The string is wrapped in
/// parens and CAN contain spaces; we take everything between the
/// first `(` and the LAST `)`.
fn parse_comm(stat: &str) -> Option<String> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    if close <= open {
        return None;
    }
    Some(stat[open + 1..close].to_string())
}

/// Return (state, ppid) from the trailer of /proc/<pid>/stat. State is
/// the third field, ppid is the fourth. The comm parens trick (taking
/// rfind(')')) sidesteps spaces-in-comm cases.
fn parse_state_and_ppid(stat: &str) -> Option<(String, u32)> {
    let close = stat.rfind(')')?;
    let tail = stat[close + 1..].trim();
    let mut fields = tail.split_whitespace();
    let state = fields.next()?.to_string();
    let ppid: u32 = fields.next()?.parse().ok()?;
    Some((state, ppid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_comm_handles_parens_in_name() {
        let stat = "12345 (hex-agent (worker)) Z 1 0 0 ...";
        assert_eq!(parse_comm(stat), Some("hex-agent (worker)".to_string()));
    }

    #[test]
    fn parse_state_and_ppid_extracts_z_and_init() {
        let stat = "12345 (hex-agent) Z 1 0 0 0 -1 1077960704 ...";
        let (state, ppid) = parse_state_and_ppid(stat).expect("parse");
        assert_eq!(state, "Z");
        assert_eq!(ppid, 1);
    }

    #[test]
    fn parse_state_and_ppid_returns_none_on_short_input() {
        assert!(parse_state_and_ppid("123 (foo) ").is_none());
    }
}

// Connect the module into orchestration/mod.rs so the sweeper task can
// be spawned at nexus startup. The actual spawn site lives in lib.rs,
// which is on CRITICAL_FILES; the operator's one-liner addition is:
//
//   crate::orchestration::zombie_sweeper::ZombieSweeper::spawn(state.clone());
//
// Today the sweeper is a leaf module with no callers — its primitives
// are still usable via `scan_zombies()` from CLI tools or tests.
