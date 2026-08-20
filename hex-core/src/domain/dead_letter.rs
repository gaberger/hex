//! Dead-letter domain types — runtime quarantine for brain-tasks that
//! exceeded their retry budget (ADR-2026-05-19-0900 P2.1).
//!
//! Pure domain shape, no I/O. Adapters in
//! `hex-nexus/src/adapters/spacetime_dead_letter.rs` translate between
//! these types and the STDB `dead_letter` table.

use serde::{Deserialize, Serialize};

/// One row in the `dead_letter` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadLetterRecord {
    /// Original brain-task / sched_task id — same handle the dashboard's
    /// queue view shows. Replaying re-creates a row in sched_task pending
    /// using this id (with priority + payload from the row below).
    pub task_id: String,
    /// "workplan" | "hex-command" | "shell" — matches the brain-task kind.
    pub kind: String,
    /// Original payload (workplan path, command args, shell line).
    pub payload: String,
    /// Most recent error from the dispatcher / executor. Bounded ~1 KB
    /// at write time so a long traceback can't blow up the STDB row
    /// size limit (see ADR-2026-05-08-2600).
    pub last_error: String,
    /// Monotonic across replays — the brain-task's retry_count gets
    /// reset on each new dispatch attempt, this stays the real total.
    pub attempt_count: u32,
    /// RFC 3339 timestamps. The dashboard uses last_failed_at to age-sort.
    pub first_failed_at: String,
    pub last_failed_at: String,
    /// Operator-tunable priority at quarantine time — `replay` preserves
    /// it so the re-enqueued task lands in the same bucket.
    pub original_priority: i32,
}
