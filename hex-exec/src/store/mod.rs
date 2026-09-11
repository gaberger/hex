//! `FileStore` — [`ILocalStore`] backed by plain files (ADR-2608241500 P3.1).
//!
//! What this replaces: eight HTTP call sites in `hex-exec` that reached a
//! SpacetimeDB instance for the run feed, the cost meter, every ADR / spec /
//! workplan / code emission, and the `hexflo_memory` table. A tool that edits
//! files in a git repository needed a database and a daemon to be running
//! before it could remember anything.
//!
//! # Layout
//!
//! ```text
//! <repo>/.hex/local-store/runs.jsonl        append-only, newest last
//! <repo>/.hex/local-store/usage.jsonl       append-only, newest last
//! <repo>/.hex/local-store/emissions.jsonl   append-only, newest last
//! <repo>/.hex/memory/<file>.md              one entry, git-tracked
//! ~/.hex/memory/<file>.md                   one entry, cross-project
//! ```
//!
//! The three logs are runtime noise and fall under the repository's existing
//! `**/.hex/*` ignore rule. Memory is the exception and is tracked on purpose:
//! a lesson in `git diff` is reviewable, travels with a clone, is shared
//! between two machines by `git pull`, and survives a wiped database — none of
//! which was true of the table it replaces.
//!
//! # Durability and concurrency
//!
//! Appends are `O_APPEND` writes of one short line, which the kernel does not
//! interleave at these sizes, so two concurrent `hex do` runs cannot corrupt
//! each other's records. Reads skip any line that will not parse rather than
//! failing the whole feed: a log truncated by a killed process must still
//! serve the records written before it died.
//!
//! # Why not SQLite
//!
//! P3.1 allowed either. The volumes are a 2,000-run ring buffer and a few
//! hundred notes of ~500 bytes; JSONL needs no new dependency, no schema, and
//! no migration, and `tail` and `git diff` read it unaided.

use std::io::Write;
use std::path::{Path, PathBuf};

use hex_core::ports::local_store::{
    EmissionRecord, ILocalStore, LocalStoreError, MemoryEntry, MemoryScope, RunRecord, UsageRecord,
};
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Runs kept in `runs.jsonl` before the log is compacted.
const RUN_LOG_CAP: usize = 2_000;
/// Usage rows kept in `usage.jsonl` before the log is compacted.
const USAGE_LOG_CAP: usize = 10_000;
/// Emission rows kept in `emissions.jsonl` before the log is compacted.
const EMISSION_LOG_CAP: usize = 2_000;
/// Compact only once a log is this much over its cap, so a busy loop does not
/// rewrite the whole file on every append.
const COMPACT_SLACK: usize = 512;

/// [`ILocalStore`] over files under `.hex/`.
#[derive(Debug, Clone)]
pub struct FileStore {
    logs: PathBuf,
    project_memory: PathBuf,
    global_memory: Option<PathBuf>,
}

impl FileStore {
    /// A store rooted at `repo_root`, with global memory under `$HOME/.hex`.
    pub fn for_repo(repo_root: &Path) -> Self {
        Self {
            logs: repo_root.join(".hex").join("local-store"),
            project_memory: repo_root.join(".hex").join("memory"),
            global_memory: home_dir().map(|h| h.join(".hex").join("memory")),
        }
    }

    /// A store for the repository the current process is working in.
    pub fn current() -> Self {
        Self::for_repo(&crate::direct_exec::repo_root())
    }

    /// A store confined to one directory, with no global memory. For tests.
    pub fn isolated(root: &Path) -> Self {
        Self {
            logs: root.join("local-store"),
            project_memory: root.join("memory"),
            global_memory: None,
        }
    }

    fn runs_path(&self) -> PathBuf {
        self.logs.join("runs.jsonl")
    }
    fn usage_path(&self) -> PathBuf {
        self.logs.join("usage.jsonl")
    }
    fn emissions_path(&self) -> PathBuf {
        self.logs.join("emissions.jsonl")
    }

    fn memory_dir(&self, scope: MemoryScope) -> Option<PathBuf> {
        match scope {
            MemoryScope::Project => Some(self.project_memory.clone()),
            MemoryScope::Global => self.global_memory.clone(),
        }
    }
}

impl Default for FileStore {
    fn default() -> Self {
        Self::current()
    }
}

// ── append-only logs ─────────────────────────────────────────────────────────

fn io_err(path: &Path, source: std::io::Error) -> LocalStoreError {
    LocalStoreError::Io { path: path.display().to_string(), source }
}

/// Append one JSON line, creating the parent directory on first use.
fn append_line<T: Serialize>(path: &Path, record: &T) -> Result<(), LocalStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
    }
    let mut line = serde_json::to_string(record)
        .map_err(|e| LocalStoreError::Invalid(format!("record is not serializable: {e}")))?;
    line.push('\n');
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| io_err(path, e))?;
    f.write_all(line.as_bytes()).map_err(|e| io_err(path, e))
}

/// Read every parseable line, oldest first. A missing file is an empty log.
///
/// Unparseable lines are skipped, not fatal — see the module docs.
fn read_lines<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, LocalStoreError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_err(path, e)),
    };
    let mut out = Vec::new();
    let mut skipped = 0usize;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<T>(line) {
            Ok(v) => out.push(v),
            Err(_) => skipped += 1,
        }
    }
    if skipped > 0 {
        tracing::debug!(path = %path.display(), skipped, "skipped unparseable local-store lines");
    }
    Ok(out)
}

/// Drop the oldest lines once a log runs `COMPACT_SLACK` past `cap`.
///
/// Best-effort: a failure here leaves a longer log, which is harmless, so it
/// never fails the append that triggered it.
fn compact(path: &Path, cap: usize) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() <= cap + COMPACT_SLACK {
        return;
    }
    let kept = lines[lines.len() - cap..].join("\n");
    let tmp = path.with_extension("jsonl.tmp");
    if std::fs::write(&tmp, format!("{kept}\n")).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Newest-first view of an append-only log.
fn newest_first<T>(mut records: Vec<T>, limit: usize) -> Vec<T> {
    records.reverse();
    records.truncate(limit);
    records
}

// ── memory files ─────────────────────────────────────────────────────────────

/// A filename that is readable, unique enough, and legal everywhere.
///
/// `lesson:workplan-drift` becomes `lesson-workplan-drift.md`. The true key
/// lives in the frontmatter, so a collision only matters when writing; see
/// [`memory_path_for`].
fn file_stem_for(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut last_dash = false;
    for c in key.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    let trimmed = if trimmed.is_empty() { "entry".to_string() } else { trimmed };
    trimmed.chars().take(80).collect()
}

/// A short stable suffix, used only to separate two keys that sanitize alike.
fn short_hash(key: &str) -> String {
    // FNV-1a. Not cryptographic — this only has to make two names differ.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{h:08x}")[..8].to_string()
}

/// Where `key` is stored in `dir`, avoiding a collision with a different key.
fn memory_path_for(dir: &Path, key: &str) -> PathBuf {
    let plain = dir.join(format!("{}.md", file_stem_for(key)));
    match read_entry(&plain) {
        // Free, or already ours.
        Ok(None) => plain,
        Ok(Some(e)) if e.key == key => plain,
        // Taken by a different key that sanitizes to the same name.
        _ => dir.join(format!("{}-{}.md", file_stem_for(key), short_hash(key))),
    }
}

fn write_entry(path: &Path, entry: &MemoryEntry) -> Result<(), LocalStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
    }
    let body = format!(
        "---\nkey: {}\nscope: {}\nupdated_at: {}\n---\n{}\n",
        entry.key,
        entry.scope.as_str(),
        entry.updated_at,
        entry.value.trim_end(),
    );
    std::fs::write(path, body).map_err(|e| io_err(path, e))
}

/// Parse one memory file. A file without frontmatter is not an entry.
fn read_entry(path: &Path) -> Result<Option<MemoryEntry>, LocalStoreError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_err(path, e)),
    };
    let Some(rest) = text.strip_prefix("---\n") else {
        return Ok(None);
    };
    let Some((front, value)) = rest.split_once("\n---\n") else {
        return Err(LocalStoreError::Corrupt {
            path: path.display().to_string(),
            detail: "frontmatter is never closed".into(),
        });
    };
    let mut key = None;
    let mut scope = MemoryScope::Project;
    let mut updated_at = String::new();
    for line in front.lines() {
        let Some((field, val)) = line.split_once(": ") else {
            continue;
        };
        match field.trim() {
            "key" => key = Some(val.trim().to_string()),
            "scope" => {
                if val.trim() == MemoryScope::Global.as_str() {
                    scope = MemoryScope::Global;
                }
            }
            "updated_at" => updated_at = val.trim().to_string(),
            _ => {}
        }
    }
    let Some(key) = key.filter(|k| !k.is_empty()) else {
        return Err(LocalStoreError::Corrupt {
            path: path.display().to_string(),
            detail: "frontmatter has no key".into(),
        });
    };
    Ok(Some(MemoryEntry { key, value: value.trim_end().to_string(), scope, updated_at }))
}

/// Every entry in one directory. A missing directory holds no entries.
fn read_dir_entries(dir: &Path) -> Vec<MemoryEntry> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in rd.flatten() {
        let path = item.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        match read_entry(&path) {
            Ok(Some(e)) => out.push(e),
            Ok(None) => {}
            Err(e) => tracing::warn!(path = %path.display(), error = %e, "skipping memory file"),
        }
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Now, as an RFC 3339 timestamp.
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ── the port ─────────────────────────────────────────────────────────────────

impl ILocalStore for FileStore {
    fn append_run(&self, run: &RunRecord) -> Result<(), LocalStoreError> {
        let path = self.runs_path();
        append_line(&path, run)?;
        compact(&path, RUN_LOG_CAP);
        Ok(())
    }

    fn recent_runs(&self, limit: usize) -> Result<Vec<RunRecord>, LocalStoreError> {
        Ok(newest_first(read_lines(&self.runs_path())?, limit))
    }

    fn append_usage(&self, usage: &UsageRecord) -> Result<(), LocalStoreError> {
        let path = self.usage_path();
        append_line(&path, usage)?;
        compact(&path, USAGE_LOG_CAP);
        Ok(())
    }

    fn usage_since(&self, since: &str) -> Result<Vec<UsageRecord>, LocalStoreError> {
        // Timestamps are RFC 3339 in UTC, which compares correctly as text.
        let all: Vec<UsageRecord> = read_lines(&self.usage_path())?;
        Ok(newest_first(
            all.into_iter().filter(|u| u.at.as_str() >= since).collect(),
            usize::MAX,
        ))
    }

    fn append_emission(&self, emission: &EmissionRecord) -> Result<(), LocalStoreError> {
        let path = self.emissions_path();
        append_line(&path, emission)?;
        compact(&path, EMISSION_LOG_CAP);
        Ok(())
    }

    fn recent_emissions(&self, limit: usize) -> Result<Vec<EmissionRecord>, LocalStoreError> {
        Ok(newest_first(read_lines(&self.emissions_path())?, limit))
    }

    fn put_memory(
        &self,
        key: &str,
        value: &str,
        scope: MemoryScope,
    ) -> Result<(), LocalStoreError> {
        let key = key.trim();
        if key.is_empty() {
            return Err(LocalStoreError::Invalid("memory key is empty".into()));
        }
        if key.contains('\n') {
            return Err(LocalStoreError::Invalid("memory key contains a newline".into()));
        }
        let Some(dir) = self.memory_dir(scope) else {
            return Err(LocalStoreError::Invalid(
                "global memory needs a home directory, and $HOME is not set".into(),
            ));
        };
        // A key may move between scopes; leaving the old copy behind would make
        // one key read back twice with different values.
        if let Some(other) = self.memory_dir(other_scope(scope)) {
            let stale = memory_path_for(&other, key);
            if matches!(read_entry(&stale), Ok(Some(ref e)) if e.key == key) {
                let _ = std::fs::remove_file(&stale);
            }
        }
        let entry = MemoryEntry {
            key: key.to_string(),
            value: value.to_string(),
            scope,
            updated_at: now_rfc3339(),
        };
        std::fs::create_dir_all(&dir).map_err(|e| io_err(&dir, e))?;
        write_entry(&memory_path_for(&dir, key), &entry)
    }

    fn get_memory(&self, key: &str) -> Result<Option<MemoryEntry>, LocalStoreError> {
        Ok(self.list_memory()?.into_iter().find(|e| e.key == key))
    }

    fn list_memory(&self) -> Result<Vec<MemoryEntry>, LocalStoreError> {
        let mut out = read_dir_entries(&self.project_memory);
        let seen: std::collections::HashSet<String> =
            out.iter().map(|e| e.key.clone()).collect();
        if let Some(global) = &self.global_memory {
            // Project scope wins: a repository's own lesson is more specific
            // than a machine-wide one under the same key.
            out.extend(read_dir_entries(global).into_iter().filter(|e| !seen.contains(&e.key)));
        }
        Ok(out)
    }

    fn delete_memory(&self, key: &str) -> Result<bool, LocalStoreError> {
        let mut removed = false;
        for scope in [MemoryScope::Project, MemoryScope::Global] {
            let Some(dir) = self.memory_dir(scope) else { continue };
            let path = memory_path_for(&dir, key);
            if matches!(read_entry(&path), Ok(Some(ref e)) if e.key == key) {
                std::fs::remove_file(&path).map_err(|e| io_err(&path, e))?;
                removed = true;
            }
        }
        Ok(removed)
    }
}

fn other_scope(scope: MemoryScope) -> MemoryScope {
    match scope {
        MemoryScope::Project => MemoryScope::Global,
        MemoryScope::Global => MemoryScope::Project,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::ports::local_store::EmissionKind;

    fn store() -> (tempfile::TempDir, FileStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FileStore::isolated(dir.path());
        (dir, store)
    }

    fn run(id: &str, ok: bool) -> RunRecord {
        RunRecord {
            id: id.to_string(),
            agent: "direct-react".into(),
            started_at: "2026-09-11T10:00:00Z".into(),
            instruction: "do the thing".into(),
            file: "hex-exec/src/lib.rs".into(),
            model: "some-model".into(),
            ok,
            attempts: 1,
            steps: 3,
            evidence_passed: ok,
            committed: ok.then(|| "abc1234".to_string()),
            duration_ms: 1200,
            error: None,
        }
    }

    #[test]
    fn reads_are_empty_before_anything_is_written() {
        let (_d, s) = store();
        assert!(s.recent_runs(10).unwrap().is_empty());
        assert!(s.usage_since("2000-01-01T00:00:00Z").unwrap().is_empty());
        assert!(s.recent_emissions(10).unwrap().is_empty());
        assert!(s.list_memory().unwrap().is_empty());
        assert!(s.get_memory("lesson:absent").unwrap().is_none());
    }

    #[test]
    fn runs_come_back_newest_first() {
        let (_d, s) = store();
        for i in 1..=3 {
            s.append_run(&run(&format!("r{i}"), true)).unwrap();
        }
        let got = s.recent_runs(10).unwrap();
        assert_eq!(got.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["r3", "r2", "r1"]);
        assert_eq!(s.recent_runs(2).unwrap().len(), 2);
    }

    #[test]
    fn run_round_trips_every_field() {
        let (_d, s) = store();
        let original = run("r1", false);
        s.append_run(&original).unwrap();
        assert_eq!(s.recent_runs(1).unwrap()[0], original);
    }

    #[test]
    fn a_corrupt_line_does_not_lose_the_rest_of_the_feed() {
        let (_d, s) = store();
        s.append_run(&run("r1", true)).unwrap();
        let path = s.runs_path();
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{ this is not json\n").unwrap();
        drop(f);
        s.append_run(&run("r2", true)).unwrap();
        let got = s.recent_runs(10).unwrap();
        assert_eq!(got.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["r2", "r1"]);
    }

    #[test]
    fn the_run_log_stays_bounded_and_keeps_the_newest_runs() {
        let (_d, s) = store();
        let total = RUN_LOG_CAP + COMPACT_SLACK * 3;
        for i in 0..total {
            s.append_run(&run(&format!("r{i}"), true)).unwrap();
        }
        // Trimming runs after the append that crosses the line, so the file
        // sits between the cap and one append past cap + slack. What matters
        // is that it is bounded, not that it lands on an exact number.
        let kept = std::fs::read_to_string(s.runs_path()).unwrap().lines().count();
        assert!(
            (RUN_LOG_CAP..=RUN_LOG_CAP + COMPACT_SLACK + 1).contains(&kept),
            "log grew to {kept} lines"
        );
        // The newest survive; the oldest are the ones dropped.
        assert_eq!(s.recent_runs(1).unwrap()[0].id, format!("r{}", total - 1));
        assert!(!s.recent_runs(usize::MAX).unwrap().iter().any(|r| r.id == "r0"));
    }

    #[test]
    fn usage_is_filtered_by_timestamp() {
        let (_d, s) = store();
        for at in ["2026-09-01T00:00:00Z", "2026-09-10T00:00:00Z", "2026-09-11T00:00:00Z"] {
            s.append_usage(&UsageRecord {
                at: at.into(),
                model: "some-model".into(),
                role: "direct-react".into(),
                intent: "edit".into(),
                input_tokens: 10,
                output_tokens: 20,
                cost_usd: Some(0.5),
            })
            .unwrap();
        }
        assert_eq!(s.usage_since("2026-09-10T00:00:00Z").unwrap().len(), 2);
        assert_eq!(s.usage_since("2027-01-01T00:00:00Z").unwrap().len(), 0);
        assert_eq!(s.usage_since("2000-01-01T00:00:00Z").unwrap().len(), 3);
    }

    #[test]
    fn emissions_round_trip() {
        let (_d, s) = store();
        let e = EmissionRecord {
            at: now_rfc3339(),
            kind: EmissionKind::Adr,
            path: "docs/adrs/ADR-2026-09-11-1000-x.md".into(),
            tool: "tool:adr_draft".into(),
            bytes: 420,
            note: String::new(),
        };
        s.append_emission(&e).unwrap();
        let got = s.recent_emissions(10).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], e);
        assert_eq!(got[0].kind.as_str(), "adr");
    }

    #[test]
    fn memory_round_trips_and_overwrites_in_place() {
        let (_d, s) = store();
        s.put_memory("lesson:trace-consumers", "grep the whole workspace", MemoryScope::Project)
            .unwrap();
        let got = s.get_memory("lesson:trace-consumers").unwrap().expect("present");
        assert_eq!(got.value, "grep the whole workspace");
        assert_eq!(got.scope, MemoryScope::Project);
        assert!(!got.updated_at.is_empty());

        s.put_memory("lesson:trace-consumers", "including feature-gated ones", MemoryScope::Project)
            .unwrap();
        assert_eq!(s.list_memory().unwrap().len(), 1, "a rewrite must not add a second entry");
        assert_eq!(
            s.get_memory("lesson:trace-consumers").unwrap().unwrap().value,
            "including feature-gated ones"
        );
    }

    #[test]
    fn a_memory_file_is_readable_markdown_with_frontmatter() {
        let (d, s) = store();
        s.put_memory("lesson:read-me", "a human can read this", MemoryScope::Project).unwrap();
        let path = d.path().join("memory").join("lesson-read-me.md");
        let text = std::fs::read_to_string(&path).expect("named after the key");
        assert!(text.starts_with("---\nkey: lesson:read-me\n"));
        assert!(text.contains("scope: project\n"));
        assert!(text.trim_end().ends_with("a human can read this"));
    }

    #[test]
    fn two_keys_that_sanitize_alike_do_not_overwrite_each_other() {
        let (_d, s) = store();
        s.put_memory("lesson:a/b", "first", MemoryScope::Project).unwrap();
        s.put_memory("lesson:a:b", "second", MemoryScope::Project).unwrap();
        assert_eq!(s.get_memory("lesson:a/b").unwrap().unwrap().value, "first");
        assert_eq!(s.get_memory("lesson:a:b").unwrap().unwrap().value, "second");
        assert_eq!(s.list_memory().unwrap().len(), 2);
    }

    #[test]
    fn search_matches_key_or_value_case_insensitively() {
        let (_d, s) = store();
        s.put_memory("lesson:workplan-drift", "reconcile after agent work", MemoryScope::Project)
            .unwrap();
        s.put_memory("gap:no-gpu-check", "bench needs a GPU probe", MemoryScope::Project).unwrap();

        assert_eq!(s.search_memory("WORKPLAN").unwrap().len(), 1);
        assert_eq!(s.search_memory("reconcile").unwrap().len(), 1);
        assert_eq!(s.search_memory("gap:").unwrap().len(), 1);
        assert_eq!(s.search_memory("").unwrap().len(), 2, "an empty query returns everything");
        assert_eq!(s.search_memory("nothing-matches-this").unwrap().len(), 0);
    }

    #[test]
    fn delete_reports_whether_the_key_existed() {
        let (_d, s) = store();
        s.put_memory("lesson:temporary", "x", MemoryScope::Project).unwrap();
        assert!(s.delete_memory("lesson:temporary").unwrap());
        assert!(!s.delete_memory("lesson:temporary").unwrap());
        assert!(s.list_memory().unwrap().is_empty());
    }

    #[test]
    fn an_empty_key_is_rejected() {
        let (_d, s) = store();
        assert!(s.put_memory("   ", "value", MemoryScope::Project).is_err());
        assert!(s.put_memory("bad\nkey", "value", MemoryScope::Project).is_err());
    }

    #[test]
    fn global_memory_is_unavailable_when_the_store_is_isolated() {
        let (_d, s) = store();
        // `isolated` has no global directory; asking for global scope must say
        // so rather than silently writing into the project.
        assert!(s.put_memory("lesson:x", "v", MemoryScope::Global).is_err());
        assert!(s.list_memory().unwrap().is_empty());
    }

    #[test]
    fn project_scope_shadows_a_global_entry_with_the_same_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FileStore {
            logs: dir.path().join("local-store"),
            project_memory: dir.path().join("project-memory"),
            global_memory: Some(dir.path().join("global-memory")),
        };
        store.put_memory("lesson:shared", "global value", MemoryScope::Global).unwrap();
        store.put_memory("lesson:shared", "project value", MemoryScope::Project).unwrap();

        let all = store.list_memory().unwrap();
        assert_eq!(all.len(), 1, "one key must read back once");
        assert_eq!(all[0].value, "project value");
    }

    #[test]
    fn file_stem_is_readable_and_bounded() {
        assert_eq!(file_stem_for("lesson:workplan-drift"), "lesson-workplan-drift");
        assert_eq!(file_stem_for("project:hex/solo refactor"), "project-hex-solo-refactor");
        assert_eq!(file_stem_for(":::"), "entry");
        assert!(file_stem_for(&"x".repeat(500)).len() <= 80);
    }
}
