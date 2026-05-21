//! Orphan hex-agent reaper (GC for live-but-unregistered worker processes).
//!
//! Companion to `zombie_sweeper` (which only handles already-defunct
//! processes). This one targets the harder failure mode: hex-agent
//! daemons that are still running but no longer claimed by any
//! `worker_process` row. Two ways a worker ends up orphan:
//!
//!   1. **Nexus restart.** `spawn_local_agent` registers the worker
//!      via `worker_process_register` and starts a tokio watchdog
//!      that polls `/proc/<pid>` every 2 s. When nexus dies the
//!      watchdog dies with it — but the hex-agent process keeps
//!      running. New nexus comes up, has no record of the old pid,
//!      and (per the supervisor_tick) reaps the stale row → spawns a
//!      fresh worker for the pool. The old hex-agent now has no row,
//!      no parent watcher, and never gets a signal to exit. Observed
//!      2026-05-21: 94 hex-agent processes on the host vs 32
//!      worker_process rows after a single restart cycle.
//!
//!   2. **Watchdog crash without process exit.** A panicking tokio
//!      task in nexus could lose the per-worker watcher without the
//!      hex-agent process actually terminating. Same end state.
//!
//! Algorithm (runs every `REAP_INTERVAL_SECS`):
//!   - Walk `/proc`, collect every running `hex-agent daemon ...` pid
//!     owned by this user (filtered by uid match) along with the
//!     `--agent-id <role>` arg parsed from the cmdline.
//!   - Pull every `worker_process` row in status ∈ { Healthy,
//!     Degraded, Starting } from STDB. Build a `Set<pid>` of
//!     "claimed" pids.
//!   - For each running pid NOT in the claimed set: send SIGTERM.
//!     Track it in a `terminating` map. On the NEXT tick, if the
//!     pid is still alive: send SIGKILL.
//!
//! Conservative-by-default: we only reap pids that match the
//! hex-agent binary AND have a parseable agent-id AND were started
//! more than `GRACE_PERIOD_SECS` ago (so newly-spawned workers that
//! haven't been registered yet aren't killed mid-startup).

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{info, warn};

use crate::state::SharedState;

/// How often to walk /proc + STDB. 60s matches zombie_sweeper.
const REAP_INTERVAL_SECS: u64 = 60;

/// Don't kill processes younger than this. Newly-spawned hex-agents
/// have a brief window between `fork()` and the supervisor_subscriber
/// running `worker_process_register` — without this grace, the reaper
/// would race the registrar and kill its own freshly-spawned worker.
const GRACE_PERIOD_SECS: u64 = 30;

/// Command names that count as a hex-agent worker. We match on the
/// EXECUTABLE basename, not cmdline arg 0 (which may include the full
/// path).
const HEX_AGENT_EXE: &str = "hex-agent";

pub struct OrphanReaper {
    state: SharedState,
}

impl OrphanReaper {
    pub fn spawn(state: SharedState) -> JoinHandle<()> {
        tokio::spawn(async move {
            Self { state }.run().await;
        })
    }

    async fn run(self) {
        let mut interval = time::interval(Duration::from_secs(REAP_INTERVAL_SECS));
        // Skip the first immediate tick; the supervisor_subscriber needs
        // a few seconds to reconcile orphan rows on nexus startup.
        interval.tick().await;
        info!(
            interval_secs = REAP_INTERVAL_SECS,
            grace_secs = GRACE_PERIOD_SECS,
            "orphan reaper started"
        );

        // pids we SIGTERM'd in the previous tick — escalate to SIGKILL
        // if still alive on the next pass.
        let mut pending_kill: HashSet<u32> = HashSet::new();

        loop {
            interval.tick().await;
            if let Err(e) = self.tick(&mut pending_kill).await {
                warn!(error = %e, "orphan reaper tick failed");
            }
        }
    }

    async fn tick(&self, pending_kill: &mut HashSet<u32>) -> Result<(), String> {
        // Step 1: enumerate live hex-agent processes.
        let running = scan_hex_agent_processes(GRACE_PERIOD_SECS);
        if running.is_empty() {
            // No agents running → also clear any stale escalation set.
            pending_kill.clear();
            return Ok(());
        }

        // Step 2: pull claimed pids from STDB. If STDB is unreachable,
        // skip this tick — safer to leave processes alone than kill
        // them blind. Read host from the canonical discovery path so
        // P4.2's rediscovery-after-drift logic applies here too.
        let stdb_host = crate::adapters::stdb_endpoint::discover_endpoint();
        let claimed = match fetch_claimed_pids(&stdb_host).await {
            Ok(set) => set,
            Err(e) => return Err(format!("fetch_claimed_pids: {e}")),
        };

        // Step 3: split running into orphans vs. claimed.
        let orphans: Vec<&HexAgentProcess> = running
            .iter()
            .filter(|p| !claimed.contains(&p.pid))
            .collect();

        if orphans.is_empty() {
            pending_kill.clear();
            return Ok(());
        }

        // Step 4: escalation pass — anyone SIGTERM'd last tick who is
        // STILL listed as orphan gets SIGKILL'd now.
        let mut killed_hard = 0u32;
        let mut still_pending: HashSet<u32> = HashSet::new();
        for orph in &orphans {
            if pending_kill.contains(&orph.pid) {
                if signal_pid(orph.pid, libc::SIGKILL) {
                    killed_hard += 1;
                    warn!(
                        pid = orph.pid,
                        role = %orph.agent_id,
                        age_secs = orph.age_secs,
                        "orphan reaper: SIGKILL (escalation after SIGTERM ignored)"
                    );
                }
                // either way, don't re-escalate next tick
            } else {
                still_pending.insert(orph.pid);
            }
        }

        // Step 5: SIGTERM the new orphans (not already pending). Don't
        // escalate them this tick — give them one cycle to clean up.
        let mut termed = 0u32;
        for orph in &orphans {
            if pending_kill.contains(&orph.pid) {
                continue; // already escalated above
            }
            if signal_pid(orph.pid, libc::SIGTERM) {
                termed += 1;
                info!(
                    pid = orph.pid,
                    role = %orph.agent_id,
                    age_secs = orph.age_secs,
                    "orphan reaper: SIGTERM (no worker_process row claims this pid)"
                );
            }
        }

        // Promote currently-orphan pids to pending_kill for next pass.
        *pending_kill = still_pending;

        // Step 6: best-effort audit log via the state port. Same
        // shape as zombie_sweeper. Missing port → silent skip (the
        // SIGTERM/SIGKILL paths already logged via tracing).
        if termed + killed_hard > 0 {
            if let Some(port) = self.state.state_port.as_ref() {
                let payload = serde_json::json!({
                    "running_total": running.len(),
                    "claimed_total": claimed.len(),
                    "orphans_total": orphans.len(),
                    "sigterm": termed,
                    "sigkill": killed_hard,
                })
                .to_string();
                let key = format!("orphan_reap:{}", chrono::Utc::now().timestamp());
                let _ = port.hexflo_memory_store(&key, &payload, "supervision").await;
            }
        }

        Ok(())
    }
}

/// Pull pids of `worker_process` rows whose status indicates they're
/// supposed to be running. `stopping`/`exited` rows are NOT claimed —
/// their pids may legitimately be orphans on the host.
///
/// Direct SQL query to STDB rather than going through `IStatePort`
/// — we'd need a new typed method just for this caller; the SQL is
/// stable and the call site is the only consumer.
async fn fetch_claimed_pids(stdb_host: &str) -> Result<HashSet<u32>, String> {
    let url = format!("{stdb_host}/v1/database/hex/sql");
    let sql = "SELECT pid FROM worker_process \
        WHERE status = 'healthy' OR status = 'degraded' OR status = 'starting'";
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let resp = client
        .post(&url)
        .body(sql.to_string())
        .send()
        .await
        .map_err(|e| format!("STDB SQL: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("STDB HTTP {}", resp.status()));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("parse: {e}"))?;
    // Response shape: [{"schema":{...},"rows":[[<pid>],[<pid>],...]}]
    let mut out = HashSet::new();
    if let Some(arr) = body.as_array() {
        for table in arr {
            if let Some(rows) = table.get("rows").and_then(|v| v.as_array()) {
                for row in rows {
                    if let Some(vals) = row.as_array() {
                        // pid is u32 in STDB, but the SQL response can
                        // serialise it as either a JSON number or a
                        // string-encoded number depending on width.
                        let pid = vals.first().and_then(|v| {
                            v.as_u64().or_else(|| {
                                v.as_str().and_then(|s| s.parse::<u64>().ok())
                            })
                        });
                        if let Some(p) = pid {
                            out.insert(p as u32);
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct HexAgentProcess {
    pub pid: u32,
    pub agent_id: String,
    pub age_secs: u64,
}

/// Walk `/proc` once, returning every running hex-agent daemon owned by
/// the current user (uid filter). Skips processes younger than
/// `grace_secs` so newly-spawned but not-yet-registered workers don't
/// get killed mid-startup. Skips zombies (state=Z) — those are
/// `zombie_sweeper`'s domain.
pub fn scan_hex_agent_processes(grace_secs: u64) -> Vec<HexAgentProcess> {
    let mut out = Vec::new();
    let dir = match std::fs::read_dir("/proc") {
        Ok(d) => d,
        Err(_) => return out,
    };
    let my_uid = unsafe { libc::geteuid() };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    for entry in dir.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else { continue };
        if !name_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(pid): Result<u32, _> = name_str.parse() else { continue };

        // Read exe basename. If the process owns no executable (kernel
        // thread, exited, etc.) skip.
        let exe_link = format!("/proc/{}/exe", pid);
        let exe_path = match std::fs::read_link(&exe_link) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let exe_name = exe_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if exe_name != HEX_AGENT_EXE {
            continue;
        }

        // Filter by uid match. Reading /proc/<pid>/status's Uid: line.
        let status_path = format!("/proc/{}/status", pid);
        let status = match std::fs::read_to_string(&status_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if !proc_owned_by(&status, my_uid) {
            continue;
        }

        // Skip zombies (Z state) — zombie_sweeper handles them.
        if proc_state(&status) == Some("Z".to_string()) {
            continue;
        }

        // Read cmdline to extract --agent-id <role>. Skip if no agent-id
        // arg present (means it wasn't spawned by the supervisor).
        let cmdline_path = format!("/proc/{}/cmdline", pid);
        let cmdline = match std::fs::read_to_string(&cmdline_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let agent_id = match parse_agent_id(&cmdline) {
            Some(a) => a,
            None => continue,
        };

        // Compute age in seconds. /proc/<pid>/stat field 22 is
        // starttime in clock ticks since boot. Easier: stat the
        // /proc/<pid> directory and use mtime.
        let age_secs = match std::fs::metadata(format!("/proc/{}", pid)) {
            Ok(meta) => meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| now.saturating_sub(d.as_secs()))
                .unwrap_or(0),
            Err(_) => 0,
        };

        if age_secs < grace_secs {
            // Too new — let the registrar catch up.
            continue;
        }

        out.push(HexAgentProcess {
            pid,
            agent_id,
            age_secs,
        });
    }
    out
}

/// Send a signal to a pid via libc::kill. Returns true on success.
fn signal_pid(pid: u32, sig: libc::c_int) -> bool {
    if !Path::new(&format!("/proc/{}", pid)).exists() {
        return false; // already gone
    }
    let rc = unsafe { libc::kill(pid as libc::pid_t, sig) };
    rc == 0
}

/// Parse `Uid:\tNNN\t...` line from /proc/<pid>/status, compare against
/// `my_uid`. Returns true on a match. Defensive: returns false on any
/// parse failure.
fn proc_owned_by(status: &str, my_uid: libc::uid_t) -> bool {
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            // Real, effective, saved, fs — first one is what we want.
            let first = rest.split_whitespace().next();
            return first.and_then(|s| s.parse::<libc::uid_t>().ok()) == Some(my_uid);
        }
    }
    false
}

/// Extract process state from /proc/<pid>/status. The line looks like
/// `State:\tS (sleeping)`.
fn proc_state(status: &str) -> Option<String> {
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("State:") {
            return rest.trim().split_whitespace().next().map(String::from);
        }
    }
    None
}

/// Parse `--agent-id <role>` out of a NUL-separated cmdline. Returns
/// None if the flag isn't present or has no value.
pub fn parse_agent_id(cmdline: &str) -> Option<String> {
    let args: Vec<&str> = cmdline.split('\0').filter(|s| !s.is_empty()).collect();
    let idx = args.iter().position(|a| *a == "--agent-id")?;
    args.get(idx + 1).map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_agent_id_extracts_value() {
        let cmdline = "hex-agent\0daemon\0--agent-id\0ceo\0--nexus-host\0127.0.0.1\0";
        assert_eq!(parse_agent_id(cmdline), Some("ceo".to_string()));
    }

    #[test]
    fn parse_agent_id_returns_none_when_missing() {
        assert!(parse_agent_id("hex-agent\0daemon\0--nexus-host\0x\0").is_none());
        assert!(parse_agent_id("").is_none());
        assert!(parse_agent_id("hex-agent\0--agent-id\0").is_none());
    }

    #[test]
    fn proc_owned_by_compares_uid() {
        let status = "Name:\thex-agent\nUid:\t1000\t1000\t1000\t1000\nGid:\t1000\t1000\t1000\t1000\n";
        assert!(proc_owned_by(status, 1000));
        assert!(!proc_owned_by(status, 1001));
    }

    #[test]
    fn proc_state_extracts_letter() {
        assert_eq!(
            proc_state("State:\tS (sleeping)\nName:\thex-agent\n"),
            Some("S".to_string())
        );
        assert_eq!(
            proc_state("State:\tZ (zombie)\n"),
            Some("Z".to_string())
        );
        assert_eq!(proc_state("Name:\thex-agent\n"), None);
    }
}
