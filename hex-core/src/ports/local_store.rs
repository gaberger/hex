//! `ILocalStore` — the small amount of state the agent loop actually keeps.
//!
//! Per ADR-2608241500 P3.1. Before this port, `hex-exec` persisted through
//! eight plain HTTP calls to a SpacetimeDB instance: the agent-run feed, the
//! cost meter's `inference_log` reads, `proposed_action_open` writes for every
//! ADR / spec / workplan / code emission, and the `hexflo_memory` table. That
//! made a database — and a daemon in front of it — a hard requirement for a
//! tool that writes files into a git repository.
//!
//! None of that state is database-shaped. It is four append-only logs and a
//! few hundred short notes. This port says so, and the adapter in
//! `hex-exec/src/store` backs it with files.
//!
//! # Why the trait is synchronous
//!
//! Every operation is a small local read or write. Making them `async` would
//! oblige `hex-core` to take a position on a runtime, which the crate exists
//! not to do, and would buy nothing: there is no socket to wait on. Callers
//! inside `async fn` may call these directly — the files are kilobytes.
//!
//! # What is deliberately *not* here
//!
//! No swarms, tasks, agents, heartbeats, or leases. Those belonged to the
//! fleet model retired by this ADR. A store that cannot express them cannot
//! quietly grow them back.

use serde::{Deserialize, Serialize};

/// Why a local-store operation could not be completed.
#[derive(Debug, thiserror::Error)]
pub enum LocalStoreError {
    /// The underlying file could not be read or written.
    #[error("local store io at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// A stored record could not be parsed back.
    #[error("local store record is corrupt in {path}: {detail}")]
    Corrupt { path: String, detail: String },
    /// The caller supplied something the store cannot represent.
    #[error("{0}")]
    Invalid(String),
}

/// One agent run, as shown in the run feed.
///
/// Field-for-field the shape the `agent_run` table held, so the feed survives
/// the move without a migration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    /// Globally unique: `<started_at>#<sequence>`.
    pub id: String,
    /// Which loop produced it, e.g. `direct-react`.
    pub agent: String,
    /// RFC 3339 start time.
    pub started_at: String,
    /// The instruction, truncated for display.
    pub instruction: String,
    /// Repository-relative target file, when the run had one.
    pub file: String,
    /// Model that answered.
    pub model: String,
    pub ok: bool,
    pub attempts: u32,
    /// Tool calls taken. Equal to `attempts` on the single-shot path.
    #[serde(default)]
    pub steps: u32,
    pub evidence_passed: bool,
    /// Commit sha, when the evidence gate passed and the run committed.
    #[serde(default)]
    pub committed: Option<String>,
    pub duration_ms: u64,
    #[serde(default)]
    pub error: Option<String>,
}

/// Token and cost accounting for one inference call.
///
/// The cost meter used to read this from a table the daemon wrote. In-process,
/// the caller that placed the request already holds every field, so it records
/// its own usage and the meter reads it back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageRecord {
    /// RFC 3339 timestamp of the call.
    pub at: String,
    pub model: String,
    /// Which loop spent it, e.g. `direct-react`. Free-form.
    #[serde(default)]
    pub role: String,
    /// What it was spent on, truncated. Free-form.
    #[serde(default)]
    pub intent: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Reported cost in USD, when the provider reported one.
    #[serde(default)]
    pub cost_usd: Option<f64>,
}

/// What kind of artifact an [`EmissionRecord`] describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmissionKind {
    Adr,
    Spec,
    Workplan,
    /// A source-code patch applied by the loop.
    Code,
    /// A question or blocker raised for the operator.
    Escalation,
}

impl EmissionKind {
    /// Stable lowercase name, for display and for JSON written by hand.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Adr => "adr",
            Self::Spec => "spec",
            Self::Workplan => "workplan",
            Self::Code => "code",
            Self::Escalation => "escalation",
        }
    }
}

/// One artifact the loop wrote, recorded for the audit trail.
///
/// These used to be `proposed_action_open` rows awaiting a digital twin's
/// approval and an executor's write. With no daemon there is no twin and no
/// executor: the tool writes the file itself, under the same git evidence gate
/// as every other change, and records that it did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmissionRecord {
    /// RFC 3339 timestamp.
    pub at: String,
    pub kind: EmissionKind,
    /// Repository-relative path written.
    pub path: String,
    /// Which tool emitted it, e.g. `tool:adr_draft`.
    pub tool: String,
    /// Size of the written content in bytes.
    pub bytes: u64,
    /// Free-form detail. Carries the message body for an escalation.
    #[serde(default)]
    pub note: String,
}

/// Where a memory entry lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    /// Travels with the repository, and is reviewable in `git diff`.
    Project,
    /// Follows the machine's user, across every project.
    Global,
}

impl MemoryScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Global => "global",
        }
    }
}

/// One memory entry: a lesson, a known gap, a project note, or a decision.
///
/// Key prefixes are conventional, not enforced: `lesson:`, `gap:`, `project:`,
/// `decision:`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub key: String,
    pub value: String,
    pub scope: MemoryScope,
    /// RFC 3339 timestamp of the last write.
    pub updated_at: String,
}

/// The state an agent loop keeps between runs.
///
/// Implementations must tolerate a store that does not exist yet: a first read
/// on a clean machine returns empty, never an error.
pub trait ILocalStore: Send + Sync {
    /// Append one run to the feed.
    fn append_run(&self, run: &RunRecord) -> Result<(), LocalStoreError>;

    /// The most recent runs, newest first, at most `limit`.
    fn recent_runs(&self, limit: usize) -> Result<Vec<RunRecord>, LocalStoreError>;

    /// Append one inference call's accounting.
    fn append_usage(&self, usage: &UsageRecord) -> Result<(), LocalStoreError>;

    /// Usage recorded at or after `since` (an RFC 3339 timestamp), newest first.
    fn usage_since(&self, since: &str) -> Result<Vec<UsageRecord>, LocalStoreError>;

    /// Append one emission record.
    fn append_emission(&self, emission: &EmissionRecord) -> Result<(), LocalStoreError>;

    /// The most recent emissions, newest first, at most `limit`.
    fn recent_emissions(&self, limit: usize) -> Result<Vec<EmissionRecord>, LocalStoreError>;

    /// Write a memory entry, replacing any entry with the same key.
    fn put_memory(
        &self,
        key: &str,
        value: &str,
        scope: MemoryScope,
    ) -> Result<(), LocalStoreError>;

    /// Read one memory entry by exact key.
    fn get_memory(&self, key: &str) -> Result<Option<MemoryEntry>, LocalStoreError>;

    /// Every memory entry, project scope first.
    fn list_memory(&self) -> Result<Vec<MemoryEntry>, LocalStoreError>;

    /// Entries whose key or value contains `query`, case-insensitively.
    ///
    /// An empty query matches everything. The default implementation filters
    /// [`list_memory`](Self::list_memory); a store with an index may override
    /// it, but at a few hundred short entries none is warranted.
    fn search_memory(&self, query: &str) -> Result<Vec<MemoryEntry>, LocalStoreError> {
        let needle = query.trim().to_lowercase();
        let all = self.list_memory()?;
        if needle.is_empty() {
            return Ok(all);
        }
        Ok(all
            .into_iter()
            .filter(|e| {
                e.key.to_lowercase().contains(&needle) || e.value.to_lowercase().contains(&needle)
            })
            .collect())
    }

    /// Remove one memory entry. Returns whether it existed.
    fn delete_memory(&self, key: &str) -> Result<bool, LocalStoreError>;
}
