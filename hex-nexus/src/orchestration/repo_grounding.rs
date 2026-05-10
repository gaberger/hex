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
    pub personas: Vec<PersonaSummary>,
    pub generated_at: Instant,
    pub repo_root: PathBuf,
}

#[derive(Clone)]
pub struct PersonaSummary {
    pub role: String,
    pub tier: String,    // executive | lead | ic | scaffolding
    pub one_line: String, // first non-comment description-ish line, if any
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
    let persona_dir = repo_root.join("hex-cli/assets/agents/hex/hex");
    let personas = if persona_dir.is_dir() {
        scan_personas(&persona_dir)
    } else {
        Vec::new()
    };
    RepoFacts {
        adrs,
        personas,
        generated_at: Instant::now(),
        repo_root: repo_root.to_path_buf(),
    }
}

fn scan_personas(dir: &Path) -> Vec<PersonaSummary> {
    let exec = ["ceo", "cto", "cpo", "coo", "ciso", "chief-visionary", "chief-architect"];
    let lead = ["engineering-lead", "product-lead", "sre-lead"];
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<PersonaSummary> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("yml") {
            continue;
        }
        let role = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let tier = if exec.contains(&role.as_str()) {
            "executive"
        } else if lead.contains(&role.as_str()) {
            "lead"
        } else if role.starts_with("hex-") || role == "scaffold-validator" {
            "scaffolding"
        } else {
            "ic"
        }
        .to_string();
        let one_line = read_yaml_description(&path);
        out.push(PersonaSummary {
            role,
            tier,
            one_line,
        });
    }
    // Sort: executives first, then leads, then everything else; alpha within tier.
    out.sort_by(|a, b| {
        let order = |t: &str| match t {
            "executive" => 0,
            "lead" => 1,
            "ic" => 2,
            _ => 3,
        };
        order(&a.tier)
            .cmp(&order(&b.tier))
            .then_with(|| a.role.cmp(&b.role))
    });
    out
}

fn read_yaml_description(path: &Path) -> String {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    for line in content.lines().take(60) {
        let lt = line.trim_start();
        if let Some(rest) = lt.strip_prefix("description:") {
            return rest.trim().trim_matches('"').trim_matches('\'').to_string();
        }
    }
    String::new()
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

    if !f.personas.is_empty() {
        s.push_str("\nOrg roster (the only roles that exist — do NOT invent others):\n");
        for p in &f.personas {
            // Skip 'ceo' (the operator) and any role that is not user-addressable
            // (scaffolding agents) so the executive narrative stays tight.
            if p.role == "ceo" || p.tier == "scaffolding" {
                continue;
            }
            let desc = if p.one_line.is_empty() {
                String::new()
            } else {
                let mut d = p.one_line.clone();
                if d.len() > 110 {
                    d.truncate(110);
                    d.push('…');
                }
                format!(" — {}", d)
            };
            s.push_str(&format!("- {} [{}]{}\n", p.role, p.tier, desc));
        }
    }

    s.push_str(
        "\n--- ANTI-FABRICATION RULE (HARD) ---\n\
         1. Never claim you have sent, attached, queued, or routed a document \
            to a 'secure channel', 'internal system', 'audit log', or any \
            other invented delivery surface. The CEO has direct repo and \
            dashboard access. To share something, give the OPERATOR-RUNNABLE \
            file path (e.g. `docs/adrs/ADR-2605081126-...md`) or the dashboard \
            hashroute (e.g. `#/merge-gate`).\n\
         2. Never invent roles, titles, names, or teams that do not appear \
            in 'Org roster' above. There is no 'head of engineering' — the \
            engineering lead is `engineering-lead`. There is no 'product \
            team' separate from `product-lead` and `cpo`. If the right \
            person to delegate to is not in the roster, say so and ask the \
            CEO who to bring in instead of fabricating one.\n\
         3. If the asked-for artifact does not appear in REPO FACTS, say so \
            plainly and ask which artifact the CEO wants — do not invent one.\n\
         4. Never promise time-bound followups ('I'll update you in 5 \
            minutes', 'shortly', 'immediately') unless you can name the \
            concrete next action and the file/dashboard surface it will \
            land on.\n",
    );
    s
}
