//! `emit` — write an artifact the loop produced, and record that it did.
//!
//! Per ADR-2608241500 P3.2. Five tools — `adr_draft`, `spec_draft`,
//! `workplan_emit`, `adr_status_set` and `code_patch` — used to finish their
//! work by POSTing a `proposed_action_open` row to SpacetimeDB and returning
//! "queued; the digital-twin executor will write the file after approval".
//!
//! # Why they now write the file themselves
//!
//! The twin and the executor lived in `hex-nexus`. With the daemon gone there
//! is nothing on the other end of that queue, so a tool that only queues is a
//! tool that silently does nothing — the worst available outcome, because the
//! model is told the artifact was created. The tools write the file directly
//! and record the write.
//!
//! Nothing is ungoverned by this. The approval that mattered was never the
//! twin's auto-approve — it rubber-stamped anything tagged `tool:*` — it was
//! the evidence gate in `direct_exec`, which reverts the working tree unless
//! the caller's command exits 0, and git, which keeps every version. Both are
//! untouched.

use std::path::{Path, PathBuf};

use hex_core::ports::local_store::{EmissionKind, EmissionRecord, ILocalStore};

use crate::store::{now_rfc3339, FileStore};

/// Why an artifact could not be written.
#[derive(Debug)]
pub struct EmitError(pub String);

impl std::fmt::Display for EmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The repository this process writes into.
///
/// `HEX_REPO_ROOT` wins, then `HEX_PROJECT_ROOT`, then the enclosing git
/// repository. One resolution for every emitter: `code_patch` used to default
/// to a hardcoded absolute path on one developer's machine, so on any other
/// machine it wrote outside the repository or not at all.
pub fn repo_root() -> PathBuf {
    std::env::var("HEX_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| crate::direct_exec::repo_root())
}

/// Reject a path that would escape the repository.
///
/// Callers apply their own allowlists on top; this is the floor that every
/// write shares, so a new emitter cannot forget it.
pub fn resolve_in_repo(repo_root: &Path, rel_path: &str) -> Result<PathBuf, EmitError> {
    if rel_path.is_empty() {
        return Err(EmitError("path is empty".into()));
    }
    if rel_path.starts_with('/') {
        return Err(EmitError(format!("path '{rel_path}' must be repo-relative")));
    }
    if Path::new(rel_path).components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(EmitError(format!("path '{rel_path}' contains '..'")));
    }
    Ok(repo_root.join(rel_path))
}

/// Write `content` to `rel_path` and append an emission record.
///
/// Parent directories are created. The recorded write is best-effort: a store
/// that cannot be written must not lose the artifact itself.
pub fn write_artifact(
    kind: EmissionKind,
    rel_path: &str,
    content: &str,
    tool: &str,
    note: &str,
) -> Result<PathBuf, EmitError> {
    let target = resolve_in_repo(&repo_root(), rel_path)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| EmitError(format!("create {}: {e}", parent.display())))?;
    }
    std::fs::write(&target, content)
        .map_err(|e| EmitError(format!("write {}: {e}", target.display())))?;
    record(kind, rel_path, tool, content.len() as u64, note);
    Ok(target)
}

/// Append an emission record without writing a file.
///
/// Used by `escalate_to_operator`, whose artifact is the message itself.
pub fn record(kind: EmissionKind, rel_path: &str, tool: &str, bytes: u64, note: &str) {
    let emission = EmissionRecord {
        at: now_rfc3339(),
        kind,
        path: rel_path.to_string(),
        tool: tool.to_string(),
        bytes,
        note: note.to_string(),
    };
    if let Err(e) = FileStore::current().append_emission(&emission) {
        tracing::debug!(error = %e, "emission record failed (non-fatal)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repo_relative_path_resolves_under_the_root() {
        let root = Path::new("/repo");
        let got = resolve_in_repo(root, "docs/adrs/ADR-x.md").unwrap();
        assert_eq!(got, Path::new("/repo/docs/adrs/ADR-x.md"));
    }

    #[test]
    fn an_escaping_path_is_rejected() {
        let root = Path::new("/repo");
        assert!(resolve_in_repo(root, "/etc/passwd").is_err());
        assert!(resolve_in_repo(root, "../outside.md").is_err());
        assert!(resolve_in_repo(root, "docs/../../outside.md").is_err());
        assert!(resolve_in_repo(root, "").is_err());
    }

    #[test]
    fn a_dot_segment_is_not_mistaken_for_an_escape() {
        let root = Path::new("/repo");
        assert!(resolve_in_repo(root, "docs/./adrs/ADR-x.md").is_ok());
        // A file whose name merely contains dots is fine.
        assert!(resolve_in_repo(root, "docs/adrs/ADR..x.md").is_ok());
    }
}
