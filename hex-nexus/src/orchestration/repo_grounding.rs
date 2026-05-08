//! Repo grounding for executive personas.
//!
//! The org_responder's personas are powered by an LLM that, without grounding,
//! happily fabricates references like "I'm sending the ADR to your secure
//! channel". This module supplies the persona system prompt with a small set
//! of REAL repo facts (recent ADRs by id + title + path, dashboard URLs) and
//! an explicit anti-fabrication rule.
//!
//! The fact set is computed once at startup and cached. It is intentionally
//! small (~3 KB) so it fits inside every reply's system prompt without
//! eating context budget.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

/// Snapshot of repo state injected into every persona prompt.
pub struct RepoFacts {
    pub adrs: Vec<AdrSummary>,
    pub generated_at: Instant,
    pub repo_root: PathBuf,
}

#[derive(Clone)]
pub struct AdrSummary {
    pub id: String,
    pub title: String,
    pub status: String,
    pub path: String,
}

static FACTS: OnceLock<RepoFacts> = OnceLock::new();

/// Initialise the facts cache. Best-effort — failures fall back to an empty
/// fact set (the persona prompt still works, just without grounding).
pub fn init(repo_root: &Path) {
    let _ = FACTS.set(load(repo_root));
}

pub fn facts() -> Option<&'static RepoFacts> {
    FACTS.get()
}

fn load(repo_root: &Path) -> RepoFacts {
    let adr_dir = repo_root.join("docs/adrs");
    let adrs = if adr_dir.is_dir() {
        scan_adrs(&adr_dir)
    } else {
        Vec::new()
    };
    RepoFacts {
        adrs,
        generated_at: Instant::now(),
        repo_root: repo_root.to_path_buf(),
    }
}

fn scan_adrs(dir: &Path) -> Vec<AdrSummary> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<AdrSummary> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s,
            None => continue,
        };
        let stem_lower = stem.to_ascii_lowercase();
        if !stem_lower.starts_with("adr-") {
            continue;
        }
        let id = stem
            .split_once('-')
            .map(|(_, rest)| rest.split('-').next().unwrap_or(rest))
            .unwrap_or(stem)
            .to_string();
        let (title, status) = read_header(&path);
        out.push(AdrSummary {
            id,
            title,
            status,
            path: path
                .strip_prefix(dir.parent().and_then(|p| p.parent()).unwrap_or(dir))
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| path.to_string_lossy().to_string()),
        });
    }
    // Sort by id descending (most recent first). ADR ids are timestamp-ish so
    // lexical sort works.
    out.sort_by(|a, b| b.id.cmp(&a.id));
    out
}

fn read_header(path: &Path) -> (String, String) {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let mut title = String::new();
    let mut status = String::new();
    for line in content.lines().take(40) {
        let lt = line.trim();
        if title.is_empty() && lt.starts_with("# ") {
            title = lt.trim_start_matches("# ").trim().to_string();
        }
        let lower = lt.to_ascii_lowercase();
        if status.is_empty()
            && (lower.starts_with("status:") || lower.starts_with("**status:**"))
        {
            status = lt
                .splitn(2, ':')
                .nth(1)
                .map(|s| s.trim().trim_matches('*').trim().to_string())
                .unwrap_or_default();
        }
        if !title.is_empty() && !status.is_empty() {
            break;
        }
    }
    if title.is_empty() {
        title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("(untitled)")
            .to_string();
    }
    if status.is_empty() {
        status = "Unknown".to_string();
    }
    (title, status)
}

/// Produce the grounding block appended to every persona system prompt.
/// Returns an empty string if facts haven't been initialised.
pub fn grounding_block(max_adrs: usize) -> String {
    let f = match FACTS.get() {
        Some(f) => f,
        None => return String::new(),
    };
    let mut s = String::new();
    s.push_str("\n\n--- REPO FACTS (real artifacts you may cite) ---\n");
    s.push_str("Dashboard: http://127.0.0.1:5555/  (operator UI)\n");
    s.push_str("  /merge-gate     — open merge requests + voter tally\n");
    s.push_str("  /persona-health — your supervisor pool state\n");
    s.push_str("  /thoughts       — your own journaled reasoning\n");
    s.push_str("ADR command: `hex adr list` or `hex adr search <term>` (operator runs this).\n");
    s.push_str(&format!("Repo root: {}\n", f.repo_root.display()));

    if !f.adrs.is_empty() {
        s.push_str(&format!(
            "\nRecent ADRs (newest first, top {}):\n",
            f.adrs.len().min(max_adrs)
        ));
        for adr in f.adrs.iter().take(max_adrs) {
            s.push_str(&format!(
                "- ADR-{} [{}] {} — {}\n",
                adr.id,
                adr.status,
                adr.title,
                adr.path
            ));
        }
    }

    s.push_str(
        "\n--- ANTI-FABRICATION RULE ---\n\
         Never claim you have sent, attached, queued, or routed a document to \
         a 'secure channel' or 'internal system'. The CEO has direct repo and \
         dashboard access. To share a document, give the OPERATOR-RUNNABLE \
         file path (e.g. `docs/adrs/ADR-2605081126-...md`), or the dashboard \
         hashroute (e.g. `#/merge-gate`). If the asked-for artifact does not \
         appear in REPO FACTS above, say so plainly and ask which artifact \
         the CEO wants — do not invent one.\n",
    );
    s
}
