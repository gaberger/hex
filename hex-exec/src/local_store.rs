//! Local, file-backed durability — the daemon's tables as JSONL under `~/.hex`.
//!
//! Four things reached SpacetimeDB from inside the agent loop: the proposal queue
//! (`proposed_action_open`, five tools), the agent-run feed (`record_agent_run` plus a startup
//! hydrate), token spend, and memory. All of it over HTTP to a database the daemon owned — so the
//! loop needed a database up to draft an ADR.
//!
//! JSONL, not SQLite. These are append-only feeds read newest-first and bounded; a line per record
//! is the whole data model. It adds no dependency, survives a partial write (a torn last line is
//! skipped, not a corrupt file), and can be read with `tail`. If any of these grows a query beyond
//! "last N", that is the moment to reach for a real database — not before.

use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use serde_json::Value;

/// `~/.hex`, or `$HEX_HOME`. Created on demand.
pub fn hex_home() -> PathBuf {
    std::env::var("HEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".hex")
        })
}

fn feed_path(name: &str) -> PathBuf {
    hex_home().join(name)
}

/// Append one record. Best-effort by design, and the reason is the same one the STDB persist had:
/// the feed is observability, and losing a line must never fail the work that produced it.
fn append(name: &str, record: &Value) -> Result<(), String> {
    append_in(&hex_home(), name, record)
}

/// The primitive, with the directory passed in.
///
/// Tests drive THIS, not the `HEX_HOME` env var. Two tests that each `set_var` the same name race
/// in cargo's thread pool and clobber one another — a bug this file's first draft had, and the same
/// one that makes `hex-agent`'s safe_file_writer suite flaky. An injected path cannot race.
fn append_in(dir: &std::path::Path, name: &str, record: &Value) -> Result<(), String> {
    let path = dir.join(name);
    if let Some(dir) = path.parent() {
        create_dir_all(dir).map_err(|e| format!("{}: {}", dir.display(), e))?;
    }
    let mut line = serde_json::to_string(record).map_err(|e| e.to_string())?;
    line.push('\n');
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("{}: {}", path.display(), e))?;
    f.write_all(line.as_bytes()).map_err(|e| e.to_string())
}

/// Read the last `limit` records, newest first. A line that does not parse is SKIPPED rather than
/// failing the read: a torn final line from a killed process must not hide the history behind it.
fn read_tail(name: &str, limit: usize) -> Vec<Value> {
    read_tail_in(&hex_home(), name, limit)
}

fn read_tail_in(dir: &std::path::Path, name: &str, limit: usize) -> Vec<Value> {
    let path = dir.join(name);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Vec::new(), // absent feed is an empty feed, not an error
    };
    let mut out: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();
    out.reverse();
    out.truncate(limit);
    out
}

// ── the proposal queue ────────────────────────────────────────────────────────

const PROPOSALS: &str = "proposals.jsonl";

/// Open a proposed action — a change awaiting a human.
///
/// Was `proposed_action_open` on the hexflo-coordination module. The id was assigned by the
/// database; here it is the wall-clock microsecond, which is monotonic enough to order a feed one
/// process appends to and needs no coordination to allocate.
fn propose_id() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

pub fn propose_action(kind: &str, payload: &str, source: &str) -> Result<u64, String> {
    let id = propose_id();
    append(
        PROPOSALS,
        &serde_json::json!({
            "id": id,
            "kind": kind,
            "payload": payload,
            "source": source,
            "status": "open",
            "ts": chrono::Utc::now().to_rfc3339(),
        }),
    )?;
    Ok(id)
}

/// Open proposals, newest first.
pub fn proposals(limit: usize) -> Vec<Value> {
    read_tail(PROPOSALS, limit)
}

// ── the agent-run feed ────────────────────────────────────────────────────────

const RUNS: &str = "agent-runs.jsonl";

/// Persist one agent run. Best-effort: the in-memory ring is the fast path, this is the copy that
/// survives a restart. Never fails a recorder.
pub fn persist_run(run: &Value) {
    if let Err(e) = append(RUNS, run) {
        tracing::debug!(error = %e, "agent-run persist failed (non-fatal)");
    }
}

/// Newest-first runs, for hydrating the in-memory feed at startup.
pub fn recent_runs(limit: usize) -> Vec<Value> {
    read_tail(RUNS, limit)
}

// ── token spend ───────────────────────────────────────────────────────────────

const SPEND: &str = "inference-log.jsonl";

/// Record one inference call's token usage.
pub fn record_spend(model: &str, input_tokens: u64, output_tokens: u64) {
    let _ = append(
        SPEND,
        &serde_json::json!({
            "model": model,
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "ts": chrono::Utc::now().to_rfc3339(),
        }),
    );
}

/// Recent spend rows, newest first.
pub fn recent_spend(limit: usize) -> Vec<Value> {
    read_tail(SPEND, limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory per test. No env var, so nothing races: cargo runs these in parallel and
    /// two tests that both `set_var("HEX_HOME", …)` overwrite each other's answer.
    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn a_proposal_round_trips_and_carries_its_source() {
        let d = dir();
        append_in(
            d.path(),
            PROPOSALS,
            &serde_json::json!({ "id": 1u64, "kind": "file_write", "source": "tool:adr_draft", "status": "open" }),
        )
        .unwrap();
        let all = read_tail_in(d.path(), PROPOSALS, 10);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0]["kind"], "file_write");
        assert_eq!(all[0]["source"], "tool:adr_draft");
        assert_eq!(all[0]["status"], "open");
    }

    #[test]
    fn proposal_ids_are_monotonic_so_a_feed_orders_without_a_database() {
        let a = propose_id();
        let b = propose_id();
        assert!(b >= a, "ids must not go backwards: {} then {}", a, b);
        assert!(a > 0, "id should be a real timestamp, not zero");
    }

    #[test]
    fn reads_are_newest_first_and_bounded() {
        let d = dir();
        for i in 0..5 {
            append_in(d.path(), RUNS, &serde_json::json!({ "n": i })).unwrap();
        }
        let got = read_tail_in(d.path(), RUNS, 3);
        assert_eq!(got.len(), 3, "limit is honoured");
        assert_eq!(got[0]["n"], 4, "newest first");
    }

    #[test]
    fn a_torn_last_line_does_not_hide_the_history_behind_it() {
        // A process killed mid-append leaves half a line. Failing the whole read there would lose
        // every earlier record too — so the bad line is skipped, not fatal.
        let d = dir();
        append_in(d.path(), RUNS, &serde_json::json!({ "n": 1 })).unwrap();
        let mut f = OpenOptions::new().append(true).open(d.path().join(RUNS)).unwrap();
        f.write_all(b"{\"n\": 2, \"tr").unwrap();
        let got = read_tail_in(d.path(), RUNS, 10);
        assert_eq!(got.len(), 1, "the intact record survives");
        assert_eq!(got[0]["n"], 1);
    }

    #[test]
    fn an_absent_feed_is_empty_not_an_error() {
        let d = dir();
        assert!(read_tail_in(d.path(), RUNS, 10).is_empty());
    }
}

// ── memory ────────────────────────────────────────────────────────────────────

const MEMORY: &str = "memory.jsonl";

/// How far back a memory read scans before it gives up on finding a key.
///
/// The feed is append-only, so a rewrite is a newer line rather than an edit.
/// A few hundred short entries is the whole realistic corpus.
const MEMORY_SCAN: usize = 5_000;

/// `(key, value)` pairs from the local memory feed, newest first, bounded.
///
/// Was `SELECT key, value FROM hexflo_memory` over SpacetimeDB's HTTP SQL endpoint. Rows missing
/// either field are skipped rather than defaulted — a lesson with no text is not a lesson.
///
/// **Newest wins.** A rewritten key appends rather than replacing, so the same
/// key can appear many times; only its most recent line is returned. Without
/// that, `hex memory store` on an existing key would leave the loop reading
/// both the old lesson and the new one.
pub fn memory_entries(limit: usize) -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for v in read_tail(MEMORY, MEMORY_SCAN) {
        let (Some(k), Some(val)) = (
            v.get("key").and_then(|x| x.as_str()),
            v.get("value").and_then(|x| x.as_str()),
        ) else {
            continue;
        };
        if v.get("deleted").and_then(|x| x.as_bool()).unwrap_or(false) {
            seen.insert(k.to_string()); // a tombstone hides older lines too
            continue;
        }
        if seen.insert(k.to_string()) {
            out.push((k.to_string(), val.to_string()));
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

/// Write a memory entry, superseding any earlier line with the same key.
pub fn memory_put(key: &str, value: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("memory key is empty".to_string());
    }
    append(
        MEMORY,
        &serde_json::json!({
            "key": key,
            "value": value,
            "ts": chrono::Utc::now().to_rfc3339(),
        }),
    )
}

/// The current value for one key, or `None`.
pub fn memory_get(key: &str) -> Option<String> {
    memory_entries(MEMORY_SCAN).into_iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// Entries whose key or value contains `query`, case-insensitively.
/// An empty query matches everything.
pub fn memory_search(query: &str) -> Vec<(String, String)> {
    let needle = query.trim().to_lowercase();
    memory_entries(MEMORY_SCAN)
        .into_iter()
        .filter(|(k, v)| {
            needle.is_empty()
                || k.to_lowercase().contains(&needle)
                || v.to_lowercase().contains(&needle)
        })
        .collect()
}

/// Hide a key from future reads. Returns whether it was present.
///
/// A tombstone line rather than a rewrite of the file: the feed stays
/// append-only, which is what makes a torn write survivable.
pub fn memory_delete(key: &str) -> Result<bool, String> {
    if memory_get(key).is_none() {
        return Ok(false);
    }
    append(
        MEMORY,
        &serde_json::json!({
            "key": key,
            "value": "",
            "deleted": true,
            "ts": chrono::Utc::now().to_rfc3339(),
        }),
    )?;
    Ok(true)
}

/// Spend rows in the shape `cost_meter` already aggregates:
/// `[group_key, input_tokens, output_tokens, cost_usd, created_at]`.
///
/// Deliberately the STDB row shape rather than a nicer one. cost_meter's aggregation — the window
/// filter, the HashMap, the cost parse — is fine and was never the problem; only where the rows
/// came from was. Keeping the shape means none of that code changes.
///
/// `cost_usd` is "0": these are LOCAL completions. Reporting a dollar figure for inference that
/// cost nothing would be worse than reporting zero, and the ladder tiers work down to local
/// models precisely so it IS zero.
pub fn spend_rows(group_by: &str, limit: usize) -> Vec<Value> {
    read_tail(SPEND, limit)
        .into_iter()
        .map(|v| {
            let key = v
                .get(group_by)
                .and_then(|x| x.as_str())
                .or_else(|| v.get("model").and_then(|x| x.as_str()))
                .unwrap_or("unknown")
                .to_string();
            serde_json::json!([
                key,
                v.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                v.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                "0",
                v.get("ts").and_then(|x| x.as_str()).unwrap_or(""),
            ])
        })
        .collect()
}
