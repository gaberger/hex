//! Heartbeat domain types — runtime liveness signal for long-running components.
//!
//! Part of ADR-2026-05-19-0900 (Runtime Supervision Architecture). The
//! supervision layer's contract: every long-running adapter (sched
//! daemon, nexus server, workplan_executor, hex-agent worker, STDB
//! gateway, Ollama bridge) emits a heartbeat row at a known cadence;
//! components observing missed beats take a documented action (restart
//! local, escalate operator, route around).
//!
//! These types are domain-pure: zero external deps, no I/O, no STDB
//! awareness. Adapters in `hex-nexus/src/adapters/secondary/` translate
//! between this shape and the actual `worker_process` STDB rows.

use serde::{Deserialize, Serialize};

/// Current liveness self-assessment a component publishes when it beats.
///
/// Components downgrade themselves from `Healthy` proactively when they
/// know they can't fully serve their port contract (e.g. STDB connection
/// down → `Degraded`). The supervisor decides what to do; the component
/// only reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeartbeatStatus {
    /// Fully operational. Default state on register + on each tick where
    /// the component's invariants hold.
    Healthy,
    /// Component is alive but can't fully serve its contract — e.g. an
    /// adapter's downstream is unreachable but the adapter itself is
    /// responsive and re-trying. Supervisor escalates after a TTL.
    Degraded,
    /// Graceful shutdown in progress. Supervisor should not respawn until
    /// it sees an explicit deregister.
    Stopping,
}

impl HeartbeatStatus {
    /// Stable string form for storage / log lines. Lowercase so the STDB
    /// `worker_process.status` column has one canonical encoding.
    pub fn as_str(self) -> &'static str {
        match self {
            HeartbeatStatus::Healthy => "healthy",
            HeartbeatStatus::Degraded => "degraded",
            HeartbeatStatus::Stopping => "stopping",
        }
    }

    /// Inverse of `as_str` — case-insensitive parse for round-tripping
    /// rows out of the registry. Returns `None` for unrecognized values
    /// so the caller can decide whether to treat unknowns as `Stopping`
    /// or surface as an error.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "healthy" => Some(Self::Healthy),
            "degraded" => Some(Self::Degraded),
            "stopping" => Some(Self::Stopping),
            _ => None,
        }
    }
}

/// One row in the `worker_process` registry. Adapters surface this to
/// supervisor logic and dashboard views.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerHeartbeat {
    /// Unique per-worker handle — typically `<role>-<host>-<pid>` or a
    /// caller-supplied UUID. Same value the caller used to `register`.
    pub worker_id: String,
    /// Logical pool the worker belongs to (e.g. `hex-coder-default`,
    /// `sched-daemon`). Multiple workers can share a pool.
    pub pool_id: String,
    /// Role the worker fulfills (e.g. `hex-coder`, `sched-daemon`,
    /// `nexus-server`, `workplan-executor`). Used for filtered queries.
    pub role: String,
    /// OS process id of the running worker.
    pub pid: u32,
    /// Hostname the worker is running on (matters for multi-host swarms).
    pub host: String,
    /// RFC 3339 timestamp the worker first registered.
    pub registered_at: String,
    /// RFC 3339 timestamp of the most recent heartbeat. Supervisors
    /// compare this against `now - role.ttl` to decide aliveness.
    pub last_heartbeat_at: String,
    /// Last self-reported status.
    pub status: HeartbeatStatus,
    /// Optional free-form note attached to the latest beat — e.g. the
    /// downstream that's currently degraded, or the workplan task in
    /// progress. Indexable in the dashboard but not in the registry.
    pub evidence: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_through_string() {
        for s in [HeartbeatStatus::Healthy, HeartbeatStatus::Degraded, HeartbeatStatus::Stopping] {
            assert_eq!(HeartbeatStatus::parse(s.as_str()), Some(s));
        }
    }

    #[test]
    fn status_parse_is_case_insensitive() {
        assert_eq!(HeartbeatStatus::parse("HEALTHY"), Some(HeartbeatStatus::Healthy));
        assert_eq!(HeartbeatStatus::parse("Degraded"), Some(HeartbeatStatus::Degraded));
        assert_eq!(HeartbeatStatus::parse("StOpPiNg"), Some(HeartbeatStatus::Stopping));
    }

    #[test]
    fn status_parse_rejects_unknown() {
        assert_eq!(HeartbeatStatus::parse("dead"), None);
        assert_eq!(HeartbeatStatus::parse(""), None);
    }
}
