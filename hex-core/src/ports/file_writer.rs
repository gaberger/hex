//! File writer port — autonomous-write contract with critical-path
//! protection. See ADR `wp-safe-file-writer-adapter` and
//! `domain::validation::CRITICAL_FILES`.
//!
//! Distinct from `IFileSystemPort`:
//! - `IFileSystemPort` is the general FS facade used by hex-nexus
//!   sandbox tooling (read/write/list/glob, async, path-traversal aware).
//! - `IFileWriter` is the narrow contract used by autonomous codegen
//!   (e.g. `workplan_executor`) — synchronous, write-only, and required
//!   to refuse writes to hex infrastructure files.
//!
//! Implementations MUST consult `validation::is_critical_path` and
//! return an `Err` describing the block. Tests rely on the error
//! message containing "critical".
use std::path::Path;

/// Autonomous write contract. Any path that matches `CRITICAL_FILES`
/// (sched.rs, monitor.rs, workplan_executor.rs, main.rs, …) MUST be
/// rejected with an error whose message contains the word `critical`
/// so callers and humans can grep failures consistently.
pub trait IFileWriter: Send + Sync {
    fn write_file(&self, path: &Path, content: &str) -> Result<(), String>;
}
