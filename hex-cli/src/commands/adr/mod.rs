pub mod doctor;

use std::path::{Path, PathBuf};

use clap::Subcommand;
use colored::Colorize;
use tabled::Tabled;

use crate::fmt::{status_badge, truncate, HexTable};
use super::spec::{find_specs_dir, find_workplans_dir, collect_workplans, workplan_specs_path};

#[derive(Subcommand)]
pub enum AdrAction {
    /// List all ADRs with status
    List,
    /// Show ADR lifecycle summary
    Status {
        /// Emit findings as JSON for the improver detector pipeline
        /// (`{findings: [{adr_id, status, kind, severity}]}`). Each
        /// finding flags a lifecycle issue: Proposed >30 days, Abandoned
        /// without replacement, or Superseded without backlink.
        #[arg(long)]
        json: bool,
    },
    /// Search ADRs by keyword
    Search {
        /// Search query
        query: String,
    },
    /// Detect stale/abandoned ADRs
    Abandoned,
    /// Review ADRs for consistency issues (ADR-041)
    Review {
        /// Specific ADR to review (e.g. ADR-040). Omit for all.
        adr_id: Option<String>,
        /// Exit non-zero if any WARNING+ findings (for CI)
        #[arg(long)]
        strict: bool,
    },
    /// Show the ADR schema, template, and next available number
    Schema,
    /// Show behavioral specs linked to an ADR via workplans
    Specs {
        /// ADR identifier (e.g. ADR-2026-03-24-0130 or partial match like 2603240130)
        adr_id: String,
    },
    /// Show which Accepted ADRs govern a path/area via `Applies-To` (ADR-2605301228).
    ///
    /// Deterministic backbone of the conflict gate: before changing a file,
    /// ask which binding decisions constrain it. Matches the query against each
    /// Accepted, non-superseded ADR's `## Applies-To:` declarations.
    Governing {
        /// File path or area to check (e.g. "hex-nexus/src/orchestration/org_responder.rs" or "inference routing")
        path: String,
    },
    /// Self-consistency checker over docs/adrs/ (ADR-2026-04-27-0800).
    ///
    /// Detects unparseable status, duplicate IDs, dangling Depends-on links,
    /// stale Proposed ADRs, unlinked Superseded, and so on. Each finding is
    /// tagged with an auto-fix tier (A/B/C) so the sched daemon can dispatch
    /// the appropriate self-fix path. Exit 0 clean, 1 warnings only, 2 any
    /// error (or any finding under `--strict`).
    Doctor {
        /// Apply tier-aware auto-fixes (ADR-2026-04-27-0800 §1a). Tier-A
        /// findings get shadow-promoted onto a `sched/auto-fix/...`
        /// branch; Tier-B findings get a draft notes file committed on
        /// a sibling branch for human review. The branches are *not*
        /// merged to `main` — pass `--fix-and-merge` for that. Tier-C
        /// findings are never mutated.
        #[arg(long)]
        fix: bool,
        /// Like `--fix`, but also merge the Tier-A auto-fix branch back
        /// to `main` via `git merge --no-ff`. Tier-B findings are still
        /// left for human review. Implies `--fix`.
        #[arg(long = "fix-and-merge")]
        fix_and_merge: bool,
        /// Emit findings as a structured JSON envelope on stdout. Schema:
        /// `{ findings: [...], summary: { total, errors, warnings, tier_a/b/c } }`.
        #[arg(long)]
        json: bool,
        /// Promote warnings to errors — exit 2 on any finding (CI gate).
        #[arg(long)]
        strict: bool,
    },
    /// Set an ADR to Accepted (the decision is approved).
    Accept {
        /// ADR id (e.g. ADR-2026-06-05-1200 or 2606051200)
        adr_id: String,
        /// One-line rationale recorded in the commit/audit trail.
        #[arg(long, short, default_value = "operator: accepted via `hex adr accept`")]
        rationale: String,
    },
    /// Set an ADR to Completed — GATED on its workplan being reconciled done.
    ///
    /// Confirms the implementation (a workplan referencing this ADR exists and is
    /// done) before flipping Accepted → Completed. `Completed` authorizes adapters
    /// like `Accepted`. Use `--force` to override the gate.
    Complete {
        /// ADR id
        adr_id: String,
        #[arg(long, short, default_value = "operator: completed via `hex adr complete`")]
        rationale: String,
        /// Skip the implementation-confirmed gate (operator override).
        #[arg(long)]
        force: bool,
    },
    /// Set an ADR to Superseded by a later one (adds a `Superseded-By:` backlink).
    Supersede {
        /// ADR id being superseded
        adr_id: String,
        /// The replacement ADR id (must exist).
        #[arg(long)]
        by: String,
        #[arg(long, short, default_value = "operator: superseded via `hex adr supersede`")]
        rationale: String,
    },
    /// Run the in-nexus adr-steward agent: advance Accepted ADRs that hex has
    /// confirmed implemented (Implementation-Present) to Completed. Runs in nexus,
    /// records to the agent feed, shows in the dashboard.
    Steward {
        /// Report candidates without mutating.
        #[arg(long)]
        dry_run: bool,
    },
    /// Regenerate `docs/adrs/INDEX.md` — a generated map of the ledger grouped by
    /// epoch → status, with `Superseded-By` links (ADR-2606071243). `README.md` is
    /// human prose and is never touched. Epoch comes from an explicit `**Epoch:**`
    /// field, else is derived deterministically from the decision date.
    Reindex {
        /// Print the would-be INDEX.md to stdout instead of writing the file.
        #[arg(long)]
        dry_run: bool,
        /// Emit the parsed registry as JSON (for the dashboard / GROUND retrieval).
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(action: AdrAction) -> anyhow::Result<()> {
    match action {
        AdrAction::List => list().await,
        AdrAction::Accept { adr_id, rationale } => set_adr_status(&adr_id, "Accepted", None, &rationale).await,
        AdrAction::Complete { adr_id, rationale, force } => complete_adr(&adr_id, &rationale, force).await,
        AdrAction::Supersede { adr_id, by, rationale } => supersede_adr(&adr_id, &by, &rationale).await,
        AdrAction::Steward { dry_run } => steward_sweep(dry_run).await,
        AdrAction::Status { json } => status(json).await,
        AdrAction::Search { query } => search(&query).await,
        AdrAction::Abandoned => abandoned().await,
        AdrAction::Review { adr_id, strict } => super::adr_review::run(adr_id, strict).await,
        AdrAction::Schema => schema().await,
        AdrAction::Specs { adr_id } => specs_for_adr(&adr_id).await,
        AdrAction::Governing { path } => governing(&path).await,
        AdrAction::Doctor {
            fix,
            fix_and_merge,
            json,
            strict,
        } => doctor_run(fix, fix_and_merge, json, strict).await,
        AdrAction::Reindex { dry_run, json } => reindex(dry_run, json).await,
    }
}

// ── Direct lifecycle verbs (no agent dispatch — the reliable path) ────────────

/// Locate the single ADR file matching an id (accepts `ADR-...`, the bare
/// timestamp, or any unambiguous substring of the filename).
fn find_adr_file(adr_id: &str) -> anyhow::Result<PathBuf> {
    let dir = find_adr_dir().ok_or_else(|| anyhow::anyhow!("No docs/adrs/ directory found"))?;
    let needle = adr_id
        .trim()
        .trim_start_matches("ADR-")
        .trim_start_matches("adr-")
        .to_lowercase();
    let mut matches: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(&dir)?.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
        if name.contains(&needle) {
            matches.push(p);
        }
    }
    match matches.len() {
        0 => anyhow::bail!("no ADR file matches '{}' under docs/adrs/", adr_id),
        1 => Ok(matches.remove(0)),
        n => anyhow::bail!("'{}' matches {} ADRs — be more specific", adr_id, n),
    }
}

/// Rewrite the `Status:` header line (preserving its style), optionally inserting
/// a `Superseded-By:` backlink. Mirrors the adr_status_set tool — the file is the
/// single source of truth.
async fn set_adr_status(
    adr_id: &str,
    new_status: &str,
    superseded_by: Option<&str>,
    rationale: &str,
) -> anyhow::Result<()> {
    let path = find_adr_file(adr_id)?;
    let content = std::fs::read_to_string(&path)?;
    let old = parse_adr_status(&content).to_string();
    let already_has_sb = content.to_lowercase().contains("superseded-by:");

    let mut out = String::with_capacity(content.len() + 80);
    let mut found = false;
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let lower = trimmed.to_lowercase();
        let nl = if line.ends_with('\n') { "\n" } else { "" };
        let indent = &line[..line.len() - trimmed.len()];
        let (label, is_status) = if lower.starts_with("**status:**") {
            ("**Status:** ", true)
        } else if lower.starts_with("status:") && !lower.starts_with("status_") {
            ("Status: ", true)
        } else {
            ("", false)
        };
        if !found && is_status {
            out.push_str(indent);
            out.push_str(label);
            out.push_str(new_status);
            out.push_str(nl);
            found = true;
            if let (Some(by), false) = (superseded_by, already_has_sb) {
                out.push_str(indent);
                out.push_str(if label.starts_with("**") { "**Superseded-By:** " } else { "Superseded-By: " });
                out.push_str(by);
                out.push('\n');
            }
        } else {
            out.push_str(line);
        }
    }
    if !found {
        anyhow::bail!("no `Status:` header line in {} — unexpected ADR format", path.display());
    }
    std::fs::write(&path, out)?;
    let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or(adr_id);
    println!("{} {} : {} → {}", "\u{2b21}".cyan(), id, old.dimmed(), new_status.green().bold());
    if let Some(by) = superseded_by {
        println!("  Superseded-By: {}", by.yellow());
    }
    println!("  {}", rationale.dimmed());
    println!("  {} file updated (uncommitted — commit to record the transition in history)", "\u{2192}".dimmed());
    Ok(())
}

/// Is the ADR's implementation confirmed? True iff a workplan referencing it
/// exists and is reconciled done. Returns (confirmed, detail).
fn adr_workplan_confirmed(adr_id: &str) -> (bool, String) {
    let needle = adr_id.trim().trim_start_matches("ADR-").trim_start_matches("adr-").to_lowercase();
    let Some(dir) = find_workplans_dir() else {
        return (false, "no docs/workplans/ directory".into());
    };
    let mut found_any = false;
    let mut best = String::new();
    for d in [dir.clone(), dir.join("drafts")] {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
            if !(text.to_lowercase().contains(&needle) || fname.contains(&needle)) {
                continue;
            }
            found_any = true;
            if let Ok(j) = serde_json::from_str::<serde_json::Value>(&text) {
                let status = j.get("status").and_then(|v| v.as_str()).unwrap_or("");
                if matches!(status, "completed" | "done") {
                    return (true, format!("{} status={}", fname, status));
                }
                best = format!("{} status={} (not done)", fname, status);
            }
        }
    }
    if found_any {
        (false, best)
    } else {
        (false, format!("no workplan references {}", needle))
    }
}

/// `hex adr complete` — gate on implementation confirmed, then set Completed.
async fn complete_adr(adr_id: &str, rationale: &str, force: bool) -> anyhow::Result<()> {
    if force {
        println!("  {} --force: skipping the implementation-confirmed gate", "!".yellow());
    } else {
        let (ok, detail) = adr_workplan_confirmed(adr_id);
        if !ok {
            anyhow::bail!(
                "not confirmed implemented — {}.\n  Accepted → Completed requires a workplan reconciled done + evidence. \
                 Re-run with --force to override.",
                detail
            );
        }
        println!("  {} gate passed: {}", "\u{2713}".green(), detail);
    }
    set_adr_status(adr_id, "Completed", None, rationale).await
}

/// `hex adr supersede` — verify the replacement exists, then set Superseded + backlink.
async fn supersede_adr(adr_id: &str, by: &str, rationale: &str) -> anyhow::Result<()> {
    find_adr_file(by).map_err(|_| anyhow::anyhow!("replacement ADR '{}' not found — create it first", by))?;
    set_adr_status(adr_id, "Superseded", Some(by), rationale).await
}

/// `hex adr steward` — trigger the in-nexus adr-steward agent (runs in nexus,
/// records to the agent feed, shows in the dashboard).
async fn steward_sweep(dry_run: bool) -> anyhow::Result<()> {
    let nexus = crate::nexus_client::NexusClient::from_env();
    nexus.ensure_running().await?;
    let path = if dry_run {
        "/api/agent/adr-steward/sweep?dry_run=true"
    } else {
        "/api/agent/adr-steward/sweep"
    };
    println!("{} adr-steward (in-nexus): scanning ADRs for Accepted + Implementation-Present …", "\u{2b21}".cyan());
    let r = nexus.post_long(path, &serde_json::json!({})).await?;
    let completed = r.get("completed").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    let accepted = r.get("accepted").and_then(|v| v.as_u64()).unwrap_or(0);
    let scanned = r.get("scanned").and_then(|v| v.as_u64()).unwrap_or(0);
    let committed = r.get("committed").and_then(|v| v.as_str());
    if dry_run {
        println!(
            "{} {} candidate(s) Accepted→Completed (of {} accepted / {} scanned) — dry run, no changes",
            "\u{2192}".dimmed(),
            completed,
            accepted,
            scanned
        );
    } else {
        println!(
            "{} advanced {} ADR(s) Accepted→Completed (of {} accepted / {} scanned){}",
            "\u{2713}".green().bold(),
            completed,
            accepted,
            scanned,
            committed.map(|h| format!(" \u{b7} commit {}", h.yellow())).unwrap_or_default()
        );
    }
    if let Some(err) = r.get("error").and_then(|v| v.as_str()) {
        println!("  {} {}", "error:".red(), err);
    }
    println!("  {} recorded to the agent-runs feed — visible in the dashboard", "\u{2192}".dimmed());
    Ok(())
}

/// Drive the doctor subcommand: detection → optional tier-aware fix →
/// output → exit code.
///
/// Detection is shared with the sched daemon (ADR-2026-04-27-0800 §1) via
/// `doctor::run`; the dispatch + rendering live here.
///
///   - `--fix`           → Tier A shadow-promote (no merge),
///                          Tier B draft on a worktree branch,
///                          Tier C no-op.
///   - `--fix-and-merge` → as above but Tier A also `git merge --no-ff`s
///                          the auto-fix branch back to main. Tier B is
///                          still left for human review (per §1a).
///
/// `--fix-and-merge` implies `--fix`. Without either, doctor stays in
/// detection-only mode and the dispatcher is never invoked.
async fn doctor_run(
    fix: bool,
    fix_and_merge: bool,
    json: bool,
    strict: bool,
) -> anyhow::Result<()> {
    let findings = doctor::run().await?;

    let dispatch: Option<Vec<doctor::DispatchResult>> = if fix || fix_and_merge {
        let cfg = doctor::ShadowPromoteConfig::live()?;
        Some(doctor::dispatch_fix(&findings, &cfg, fix_and_merge))
    } else {
        None
    };

    if json {
        println!("{}", doctor::to_json_with_dispatch(&findings, dispatch.as_deref())?);
    } else {
        print_doctor_human(&findings);
        if let Some(results) = &dispatch {
            print_dispatch_human(results);
        }
    }

    let code = doctor::exit_code(&findings, strict);
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

/// Human-readable rendering of a `doctor::dispatch_fix` result set. JSON
/// mode bypasses this and uses [`doctor::to_json_with_dispatch`] so the
/// schema stays the daemon's contract.
fn print_dispatch_human(results: &[doctor::DispatchResult]) {
    use doctor::{DispatchResult, Outcome};

    let applied = results.iter().filter(|r| r.was_applied()).count();
    let aborted = results.iter().filter(|r| r.was_aborted()).count();
    let notified = results.iter().filter(|r| matches!(r, DispatchResult::C)).count();

    println!();
    println!("{} Auto-fix dispatch results", "\u{2b21}".cyan());
    for r in results {
        match r {
            DispatchResult::A { outcome } => match outcome {
                Outcome::Applied { branch, commit } => println!(
                    "  [A] {} branch={} commit={}",
                    "applied".green(),
                    branch,
                    &commit[..commit.len().min(8)],
                ),
                Outcome::Aborted { reason } => {
                    println!("  [A] {} {}", "aborted".yellow(), reason)
                }
            },
            DispatchResult::B { outcome } => match outcome {
                Outcome::Applied { branch, commit } => println!(
                    "  [B] {} branch={} commit={} (left for review)",
                    "drafted".green(),
                    branch,
                    &commit[..commit.len().min(8)],
                ),
                Outcome::Aborted { reason } => {
                    println!("  [B] {} {}", "aborted".yellow(), reason)
                }
            },
            DispatchResult::C => {
                println!("  [C] {} (notify-only)", "skipped".dimmed());
            }
        }
    }
    println!();
    println!(
        "  {} applied, {} aborted, {} notify-only",
        applied, aborted, notified
    );
}

/// Human-readable rendering of a `doctor::run` finding set. JSON mode bypasses
/// this and uses [`doctor::to_json`] so the schema stays the daemon's contract.
fn print_doctor_human(findings: &[doctor::Finding]) {
    use doctor::{AutoFixTier, Severity};

    println!("{} ADR Doctor — registry self-consistency", "\u{2b21}".cyan());
    println!();

    if findings.is_empty() {
        println!("  {}", "No findings — registry is consistent.".green());
        return;
    }

    for f in findings {
        let sev = match f.severity {
            Severity::Error => "ERROR".red().to_string(),
            Severity::Warning => "WARN".yellow().to_string(),
        };
        let tier = match f.tier {
            AutoFixTier::A => "A".green().to_string(),
            AutoFixTier::B => "B".yellow().to_string(),
            AutoFixTier::C => "C".dimmed().to_string(),
        };
        println!(
            "  [{}] {} {} ({:?}) — {}",
            tier,
            sev,
            f.adr_id.bold(),
            f.kind,
            f.detail
        );
    }
    println!();

    let errors = findings.iter().filter(|f| f.severity == Severity::Error).count();
    let warnings = findings.iter().filter(|f| f.severity == Severity::Warning).count();
    let tier_a = findings.iter().filter(|f| f.tier == AutoFixTier::A).count();
    let tier_b = findings.iter().filter(|f| f.tier == AutoFixTier::B).count();
    let tier_c = findings.iter().filter(|f| f.tier == AutoFixTier::C).count();
    println!(
        "  {} error(s), {} warning(s) · tier A:{} B:{} C:{}",
        errors, warnings, tier_a, tier_b, tier_c
    );
}

/// Discover the ADR directory, searching from the current directory upward.
fn find_adr_dir() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let mut dir = cwd.as_path();
    loop {
        let candidate = dir.join("docs").join("adrs");
        if candidate.is_dir() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
}

/// Parse the status from an ADR markdown file.
///
/// Handles three formats:
///   - YAML frontmatter: `status: Accepted`
///   - Bold markdown:    `**Status:** Accepted`
///   - Heading form:     `## Status\nAccepted` (value on next non-empty line)
///
/// Strict-by-design rejections (verified by tests):
///   - `**Status**: Accepted` (colon outside bold)
///   - `- **Status**: Accepted` (bullet-prefixed)
fn parse_adr_status(content: &str) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        let lower = trimmed.to_lowercase();

        // Extract the value via one of the three accepted formats.
        let val: String = if lower.starts_with("**status:**") {
            // **Status:** Accepted
            trimmed["**Status:**".len()..].trim().to_string()
        } else if lower.starts_with("status:") && !lower.starts_with("status_") {
            // status: Accepted (YAML frontmatter)
            trimmed["status:".len()..].trim().to_string()
        } else if lower == "## status" || lower == "## status:" {
            // ## Status (heading) — value is on the next non-empty line.
            let mut j = i + 1;
            while j < lines.len() && lines[j].trim().is_empty() {
                j += 1;
            }
            if j >= lines.len() {
                i += 1;
                continue;
            }
            // Strip surrounding bold markers, e.g. "**Accepted** | Open" → "Accepted | Open"
            lines[j].trim().trim_matches('*').trim().to_string()
        } else {
            i += 1;
            continue;
        };

        return match val.to_lowercase().as_str() {
            s if s.contains("proposed") => "proposed",
            // "completed" before "accepted": a Completed ADR's line may mention both.
            s if s.contains("completed") => "completed",
            s if s.contains("accepted") => "accepted",
            s if s.contains("rejected") => "rejected",
            s if s.contains("deprecated") => "deprecated",
            s if s.contains("abandoned") => "abandoned",
            s if s.contains("superseded") => "superseded",
            _ => "unknown",
        };
    }
    "unknown"
}

/// Collect all ADR files from the directory.
async fn collect_adrs(dir: &Path) -> anyhow::Result<Vec<(PathBuf, String)>> {
    let mut adrs = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            // Only include files that start with "ADR-" (skip TEMPLATE.md, README.md, etc.)
            let fname = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
            if !fname.starts_with("ADR-") {
                continue;
            }
            let content = tokio::fs::read_to_string(&path).await?;
            adrs.push((path, content));
        }
    }
    adrs.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(adrs)
}

/// Parse the `Enforced-By` field from an ADR markdown.
///
/// Looks for a line starting with `## Enforced-By:` or a frontmatter field
/// `enforced-by:`. Returns Some(description) if found, None otherwise.
fn parse_enforced_by(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        // Check for heading style: ## Enforced-By: <tool>
        if let Some(rest) = trimmed.strip_prefix("## Enforced-By:") {
            let val = rest.trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
        // Check for frontmatter style: enforced-by: <tool>
        let lower = trimmed.to_lowercase();
        if lower.starts_with("enforced-by:") {
            let val = trimmed["enforced-by:".len()..].trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

/// Extract the ADR ID from a filename stem. Handles three forms:
///   "ADR-059-foo"                    → "ADR-059"            (legacy sequential)
///   "ADR-2026-03-22-1500-foo"        → "ADR-2026-03-22-1500" (hyphenated timestamp)
///   "ADR-2026-03-22-1500-foo"             → "ADR-2026-03-22-1500"     (legacy 10-digit)
///
/// The naive split-on-hyphen previously returned "ADR-2026" for every
/// hyphenated file, causing `hex adr doctor` to report 154 duplicates.
fn extract_adr_id(filename: &str) -> String {
    let rest = match filename.strip_prefix("ADR-").or_else(|| filename.strip_prefix("adr-")) {
        Some(r) => r,
        None => return filename.to_string(),
    };

    // Hyphenated timestamp: YYYY-MM-DD-HHMM (4-2-2-4)
    let parts: Vec<&str> = rest.splitn(5, '-').collect();
    if parts.len() >= 4
        && parts[0].len() == 4 && parts[0].chars().all(|c| c.is_ascii_digit())
        && parts[1].len() == 2 && parts[1].chars().all(|c| c.is_ascii_digit())
        && parts[2].len() == 2 && parts[2].chars().all(|c| c.is_ascii_digit())
        && parts[3].len() == 4 && parts[3].chars().all(|c| c.is_ascii_digit())
    {
        return format!("ADR-{}-{}-{}-{}", parts[0], parts[1], parts[2], parts[3]);
    }

    // Date-only timestamp: YYYY-MM-DD (4-2-2, no HHMM) — e.g.
    // ADR-2026-05-09-cost-ops-runbook. Checked after the date-time form so a
    // full timestamp still wins; without this these collapse to "ADR-2026".
    if parts.len() >= 3
        && parts[0].len() == 4 && parts[0].chars().all(|c| c.is_ascii_digit())
        && parts[1].len() == 2 && parts[1].chars().all(|c| c.is_ascii_digit())
        && parts[2].len() == 2 && parts[2].chars().all(|c| c.is_ascii_digit())
    {
        return format!("ADR-{}-{}-{}", parts[0], parts[1], parts[2]);
    }

    // Fall back to leading-digit run (covers ADR-059 + ADR-2026-03-22-1500 legacy).
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        return format!("ADR-{}", digits);
    }
    filename.to_string()
}

/// Extract the title from an ADR file (first # heading or filename).
fn extract_title(path: &Path, content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("# ") {
            return title.to_string();
        }
    }
    // Fallback to filename
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string()
}

// ── Epoch model + generated index (ADR-2606071243) ───────────────────────
//
// An *epoch* is a named era of the system's design philosophy. The canonical
// epochs and their date boundaries are owned by ADR-2606071243. Membership is
// taken from an explicit `**Epoch:**` field when present, else derived
// deterministically from the decision date so the index is immediately useful
// over the existing corpus without hand-editing 245 files.

/// Ordered most-recent-first; `EPOCHS[0]` is the current epoch. The `start` is
/// the inclusive lower bound (YYYY-MM-DD, lexicographically comparable because
/// zero-padded); an ADR belongs to the first epoch whose `start` it is `>=`.
const EPOCHS: &[(&str, &str, &str)] = &[
    // (key, inclusive-start-date, one-line identity)
    ("single-agent", "2026-06-06", "One gateway-mediated agent loop; code-graph context as the differentiator"),
    ("org-sim",      "2026-04-01", "Multi-agent organization simulation: personas + SOP + autonomous spawn"),
    ("foundation",   "0000-00-00", "Hexagonal microkernel + SpacetimeDB state core + FS-bridge daemon"),
];

/// Parse a bold (`**Field:**`), heading (`## Field:`), or YAML (`field:`)
/// frontmatter value, first match wins. The heading form is how the legacy
/// `ADR-NNN` corpus carries `## Date:` / `## Status`.
fn parse_adr_field(content: &str, field: &str) -> Option<String> {
    let bold = format!("**{}:**", field.to_lowercase());
    let heading = format!("## {}:", field.to_lowercase());
    let yaml = format!("{}:", field.to_lowercase());
    let yaml_underscore = format!("{}_", field.to_lowercase());
    for line in content.lines() {
        let trimmed = line.trim().trim_start_matches("- ").trim_start();
        let lower = trimmed.to_lowercase();
        let val = if lower.starts_with(&bold) {
            Some(trimmed[bold.len()..].trim())
        } else if lower.starts_with(&heading) {
            Some(trimmed[heading.len()..].trim())
        } else if lower.starts_with(&yaml) && !lower.starts_with(&yaml_underscore) {
            Some(trimmed[yaml.len()..].trim())
        } else {
            None
        };
        if let Some(v) = val {
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// The `Superseded-By` target id, accepting every spelling the corpus uses
/// (mirrors `doctor::has_superseded_by`).
fn parse_superseded_by(content: &str) -> Option<String> {
    for line in content.lines() {
        let stripped = line.trim().trim_start_matches("- ").trim_start();
        let lower = stripped.to_lowercase();
        for prefix in [
            "**superseded by:**",
            "**superseded by**:",
            "**superseded-by:**",
            "**superseded-by**:",
            "superseded by:",
            "superseded-by:",
        ] {
            if lower.starts_with(prefix) {
                let v = stripped[prefix.len()..].trim().trim_matches('*').trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// Best-effort decision date (YYYY-MM-DD) for an ADR: the `**Date:**` field if
/// present, else derived from the id (`ADR-2026-03-22-1500` → `2026-03-22`,
/// `ADR-2606071243` → `2026-06-07`). Legacy `ADR-NNN` ids carry no date → None.
fn adr_date(id: &str, content: &str) -> Option<String> {
    if let Some(d) = parse_adr_field(content, "date") {
        // Take the leading YYYY-MM-DD if the field has a suffix.
        let head: String = d.chars().take(10).collect();
        if head.len() == 10 && head.as_bytes()[4] == b'-' {
            return Some(head);
        }
    }
    let rest = id.trim_start_matches("ADR-").trim_start_matches("adr-");
    let parts: Vec<&str> = rest.splitn(4, '-').collect();
    // Hyphenated timestamp: YYYY-MM-DD-...
    if parts.len() >= 3
        && parts[0].len() == 4
        && parts[1].len() == 2
        && parts[2].len() >= 2
        && parts[0].chars().chain(parts[1].chars()).all(|c| c.is_ascii_digit())
    {
        return Some(format!("{}-{}-{}", parts[0], parts[1], &parts[2][..2]));
    }
    // Compact YYMMDDHHMM (10 digits, no hyphens): ADR-2606071243 → 2026-06-07.
    if rest.len() >= 6 && rest.chars().take(6).all(|c| c.is_ascii_digit()) && !rest.contains('-') {
        let b = rest.as_bytes();
        return Some(format!(
            "20{}{}-{}{}-{}{}",
            b[0] as char, b[1] as char, b[2] as char, b[3] as char, b[4] as char, b[5] as char,
        ));
    }
    None
}

/// Resolve an ADR's epoch: explicit `**Epoch:**` field wins; else date-bucket;
/// else `unassigned`.
fn adr_epoch(id: &str, content: &str) -> String {
    if let Some(e) = parse_adr_field(content, "epoch") {
        let key = e.split_whitespace().next().unwrap_or("").to_lowercase();
        if EPOCHS.iter().any(|(k, _, _)| *k == key) {
            return key;
        }
    }
    if let Some(date) = adr_date(id, content) {
        return EPOCHS
            .iter()
            .find(|(_, start, _)| date.as_str() >= *start)
            .map(|(k, _, _)| k.to_string())
            .unwrap_or_else(|| "unassigned".to_string());
    }
    // Legacy sequential id (`ADR-NNN`, ≤4 digits, no timestamp) with no date
    // field predates the timestamp scheme → foundation by construction. A
    // compact `ADR-YYMMDDHHMM` (10 digits) would already have resolved a date.
    let digits: String = id
        .trim_start_matches("ADR-")
        .trim_start_matches("adr-")
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if !digits.is_empty() && digits.len() <= 4 {
        return "foundation".to_string();
    }
    "unassigned".to_string()
}

struct IndexEntry {
    id: String,
    title: String,
    status: String,
    epoch: String,
    date: String,
    superseded_by: Option<String>,
}

async fn collect_index_entries(dir: &Path) -> anyhow::Result<Vec<IndexEntry>> {
    let mut entries: Vec<IndexEntry> = Vec::new();
    for (path, content) in collect_adrs(dir).await? {
        let fname = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
        let id = extract_adr_id(fname);
        entries.push(IndexEntry {
            title: extract_title(&path, &content),
            status: parse_adr_status(&content).to_string(),
            epoch: adr_epoch(&id, &content),
            date: adr_date(&id, &content).unwrap_or_default(),
            superseded_by: parse_superseded_by(&content),
            id,
        });
    }
    // Stable, useful order: by date desc (newest first), id as tiebreak.
    entries.sort_by(|a, b| b.date.cmp(&a.date).then(a.id.cmp(&b.id)));
    Ok(entries)
}

/// Epoch ordering for display: current first, then older, `unassigned` last.
fn epoch_rank(key: &str) -> usize {
    EPOCHS
        .iter()
        .position(|(k, _, _)| *k == key)
        .unwrap_or(EPOCHS.len())
}

fn render_index(entries: &[IndexEntry]) -> String {
    let mut out = String::new();
    out.push_str("# ADR Index\n\n");
    out.push_str(
        "> **Generated by `hex adr reindex` — do not edit by hand.**\n\
         > This is a map of the decision *ledger*, grouped by epoch (era of design\n\
         > philosophy, per ADR-2606071243). For the current-state architecture, read\n\
         > [`ARCHITECTURE.md`](../../ARCHITECTURE.md); for *why* a decision was made,\n\
         > read the ADR itself.\n\n",
    );

    // Summary line.
    out.push_str(&format!("**{} ADRs** across {} epochs.\n\n", entries.len(), {
        let mut seen: Vec<&str> = entries.iter().map(|e| e.epoch.as_str()).collect();
        seen.sort();
        seen.dedup();
        seen.len()
    }));

    // Distinct epochs present, in display order.
    let mut epochs: Vec<&str> = entries.iter().map(|e| e.epoch.as_str()).collect();
    epochs.sort_by_key(|k| epoch_rank(k));
    epochs.dedup();

    for epoch in epochs {
        let identity = EPOCHS
            .iter()
            .find(|(k, _, _)| *k == epoch)
            .map(|(_, _, id)| *id)
            .unwrap_or("ADRs with no resolvable epoch — assign `**Epoch:**` or a `**Date:**`");
        let current = epoch_rank(epoch) == 0;
        out.push_str(&format!(
            "## Epoch: `{}`{}\n\n_{}_\n\n",
            epoch,
            if current { " — **current**" } else { "" },
            identity,
        ));
        out.push_str("| ADR | Status | Title | Superseded-By |\n");
        out.push_str("|-----|--------|-------|---------------|\n");
        for e in entries.iter().filter(|e| e.epoch == epoch) {
            let sb = e.superseded_by.as_deref().unwrap_or("");
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                e.id,
                e.status,
                e.title.replace('|', "\\|"),
                sb,
            ));
        }
        out.push('\n');
    }
    out
}

async fn reindex(dry_run: bool, json: bool) -> anyhow::Result<()> {
    let dir = find_adr_dir().ok_or_else(|| anyhow::anyhow!("No docs/adrs/ directory found"))?;
    let entries = collect_index_entries(&dir).await?;

    if json {
        let items: Vec<String> = entries
            .iter()
            .map(|e| {
                format!(
                    "    {{\"id\": {:?}, \"status\": {:?}, \"epoch\": {:?}, \"date\": {:?}, \"superseded_by\": {}, \"title\": {:?}}}",
                    e.id,
                    e.status,
                    e.epoch,
                    e.date,
                    match &e.superseded_by {
                        Some(s) => format!("{:?}", s),
                        None => "null".to_string(),
                    },
                    e.title,
                )
            })
            .collect();
        println!("{{\n  \"count\": {},\n  \"adrs\": [\n{}\n  ]\n}}", entries.len(), items.join(",\n"));
        return Ok(());
    }

    let rendered = render_index(&entries);
    if dry_run {
        print!("{rendered}");
        return Ok(());
    }

    let index_path = dir.join("INDEX.md");
    tokio::fs::write(&index_path, &rendered).await?;
    let unassigned = entries.iter().filter(|e| e.epoch == "unassigned").count();
    println!(
        "{} {} — {} ADRs indexed{}",
        "✓".green(),
        index_path.display().to_string().cyan(),
        entries.len(),
        if unassigned > 0 {
            format!(", {} unassigned (add `**Epoch:**` or `**Date:**`)", unassigned).yellow().to_string()
        } else {
            String::new()
        },
    );
    Ok(())
}

// ── Tabled row structs ──────────────────────────────────────────────────

#[derive(Tabled)]
struct AdrListRow {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Enforcement")]
    enforcement: String,
    #[tabled(rename = "Title")]
    title: String,
}

#[derive(Tabled)]
struct AdrStatusRow {
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Count")]
    count: usize,
}

#[derive(Tabled)]
struct AdrSearchRow {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Title")]
    title: String,
    #[tabled(rename = "Context")]
    context: String,
}

#[derive(Tabled)]
struct AdrAbandonedRow {
    #[tabled(rename = "")]
    indicator: String,
    #[tabled(rename = "Title")]
    title: String,
    #[tabled(rename = "Status")]
    status: String,
}

async fn list() -> anyhow::Result<()> {
    let adr_dir = find_adr_dir().ok_or_else(|| anyhow::anyhow!("No docs/adrs/ directory found"))?;
    let adrs = collect_adrs(&adr_dir).await?;

    if adrs.is_empty() {
        println!("{} No ADRs found in {}", "\u{2b21}".dimmed(), adr_dir.display());
        return Ok(());
    }

    println!("{} Architecture Decision Records", "\u{2b21}".cyan());
    println!();

    let rows: Vec<AdrListRow> = adrs
        .iter()
        .map(|(path, content)| {
            let filename = path.file_stem().and_then(|s| s.to_str()).unwrap_or("???");
            let id = extract_adr_id(filename);
            let status = parse_adr_status(content);
            let title = extract_title(path, content);
            let enforced = parse_enforced_by(content);

            let enforcement = match &enforced {
                Some(_) => "\u{2713} enforced".green().to_string(),
                None => "\u{2014} honor system".dimmed().to_string(),
            };

            AdrListRow {
                id,
                status: status_badge(status),
                enforcement,
                title: truncate(&title, 60),
            }
        })
        .collect();

    println!("{}", HexTable::render(&rows));
    println!();
    println!("  {} ADR(s) total", adrs.len());

    // Warn about any ADRs with unparseable status — likely wrong frontmatter format
    let unknown: Vec<&str> = adrs
        .iter()
        .filter(|(_, content)| parse_adr_status(content) == "unknown")
        .filter_map(|(path, _)| path.file_name()?.to_str())
        .collect();
    if !unknown.is_empty() {
        println!();
        println!(
            "  {} {} ADR(s) have unparseable status — frontmatter must use `**Status:** <value>`:",
            "\u{26a0}".yellow(),
            unknown.len()
        );
        for name in &unknown {
            println!("    {}", name.yellow());
        }
    }

    Ok(())
}

/// Days since a file's first git commit. Returns `None` when the file
/// isn't tracked or git isn't available — callers treat that as "no age
/// info" (skip age-based filtering rather than risk false negatives).
fn file_first_commit_age_days(path: &std::path::Path) -> Option<i64> {
    let out = std::process::Command::new("git")
        .args([
            "log",
            "--diff-filter=A",
            "--format=%ct",
            "--",
            &path.to_string_lossy(),
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let ts: i64 = stdout.trim().lines().last()?.parse().ok()?;
    let now = chrono::Utc::now().timestamp();
    Some(((now - ts).max(0)) / 86400)
}

async fn status(json: bool) -> anyhow::Result<()> {
    let adr_dir = find_adr_dir().ok_or_else(|| anyhow::anyhow!("No docs/adrs/ directory found"))?;
    let adrs = collect_adrs(&adr_dir).await?;

    if json {
        // Emit findings shape consumed by the improver's `adr_lifecycle`
        // detector. Filter out in-flight Proposed ADRs (<30 days since
        // first commit) — those aren't drift, they're work in progress.
        // Superseded/Deprecated/Abandoned/Unparseable always surface
        // because their status doesn't decay with age.
        const PROPOSED_STALE_DAYS: i64 = 30;
        let mut findings = Vec::new();
        for (path, content) in &adrs {
            let s = parse_adr_status(content);
            let lower = s.to_lowercase();
            let kind = if lower.starts_with("proposed") {
                Some("proposed")
            } else if lower.starts_with("abandoned") {
                Some("abandoned")
            } else if lower.starts_with("superseded") {
                Some("superseded")
            } else if lower.starts_with("deprecated") {
                Some("deprecated")
            } else if lower == "unknown" {
                Some("unparseable_status")
            } else {
                None
            };
            if let Some(k) = kind {
                if k == "proposed" {
                    let age = file_first_commit_age_days(path).unwrap_or(0);
                    if age < PROPOSED_STALE_DAYS {
                        continue;
                    }
                }
                let filename = path.file_stem().and_then(|x| x.to_str()).unwrap_or("");
                let adr_id = extract_adr_id(filename);
                let severity = if k == "unparseable_status" || k == "abandoned" { "error" } else { "warning" };
                findings.push(serde_json::json!({
                    "adr_id": adr_id,
                    "status": s,
                    "kind": k,
                    "severity": severity,
                }));
            }
        }
        println!("{}", serde_json::json!({"findings": findings}));
        return Ok(());
    }

    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (_, content) in &adrs {
        let s = parse_adr_status(content);
        *counts.entry(s).or_insert(0) += 1;
    }

    println!("{} ADR Lifecycle Summary", "\u{2b21}".cyan());
    println!();

    let statuses = ["proposed", "accepted", "deprecated", "superseded", "abandoned", "unknown"];
    let rows: Vec<AdrStatusRow> = statuses
        .iter()
        .filter_map(|s| {
            counts.get(s).map(|&count| AdrStatusRow {
                status: status_badge(s),
                count,
            })
        })
        .collect();

    println!("{}", HexTable::compact(&rows));
    println!();
    println!("  {} total", adrs.len());
    Ok(())
}

async fn search(query: &str) -> anyhow::Result<()> {
    let adr_dir = find_adr_dir().ok_or_else(|| anyhow::anyhow!("No docs/adrs/ directory found"))?;
    let adrs = collect_adrs(&adr_dir).await?;

    let query_lower = query.to_lowercase();
    let mut matches = Vec::new();

    for (path, content) in &adrs {
        if content.to_lowercase().contains(&query_lower) {
            let title = extract_title(path, content);
            let status = parse_adr_status(content);

            // Find matching lines for context
            let mut context_lines = Vec::new();
            for line in content.lines() {
                if line.to_lowercase().contains(&query_lower) {
                    context_lines.push(line.trim().to_string());
                    if context_lines.len() >= 3 {
                        break;
                    }
                }
            }

            matches.push((path, title, status, context_lines));
        }
    }

    println!(
        "{} Search results for '{}'",
        "\u{2b21}".cyan(),
        query.bold()
    );
    println!();

    if matches.is_empty() {
        println!("  {}", "No matches found".dimmed());
    } else {
        let rows: Vec<AdrSearchRow> = matches
            .iter()
            .map(|(path, title, status, context)| {
                let filename = path.file_stem().and_then(|s| s.to_str()).unwrap_or("???");
                let id = extract_adr_id(filename);
                AdrSearchRow {
                    id,
                    status: status_badge(status),
                    title: truncate(title, 50),
                    context: truncate(&context.join(" | "), 60),
                }
            })
            .collect();

        println!("{}", HexTable::render(&rows));
        println!();
        println!("  {} match(es)", matches.len());
    }

    Ok(())
}

async fn schema() -> anyhow::Result<()> {
    let adr_dir = find_adr_dir().ok_or_else(|| anyhow::anyhow!("No docs/adrs/ directory found"))?;

    // Generate timestamp-based ID (YYMMDDHHMM) — no reservation needed
    let timestamp_id = generate_timestamp_adr_id();
    let now = chrono::Local::now();
    let human_readable = now.format("%Y-%m-%d %H:%M").to_string();

    println!("{} ADR Schema (for inference engines)", "\u{2b21}".cyan());
    println!();
    println!("  {:<20} {}", "Next ID:".bold(), format!("ADR-{}", timestamp_id).green());
    println!("  {:<20} {}", "Readable:".bold(), human_readable.dimmed());
    println!("  {:<20} {}", "Format:".bold(), "YYMMDDHHMM (timestamp, no reservation needed)".dimmed());
    println!("  {:<20} {}", "Directory:".bold(), adr_dir.display());
    println!("  {:<20} ADR-{{YYMMDDHHMM}}-{{kebab-slug}}.md", "Filename pattern:".bold());
    println!();

    println!("{}", "── Valid statuses ──".bold());
    println!("  Proposed | Accepted | Deprecated | Superseded | Abandoned");
    println!();

    println!("{}", "── Required sections ──".bold());
    println!("  # ADR-{{NNN}}: {{Title}}");
    println!("  **Status:** {{status}}");
    println!("  **Date:** {{YYYY-MM-DD}}");
    println!("  **Drivers:** {{what triggered this}}");
    println!("  ## Context");
    println!("  ## Decision");
    println!("  ## Consequences");
    println!("  ## Implementation");
    println!("  ## References");
    println!();

    println!("{}", "── Template ──".bold());
    // Read and display the template
    let template_path = adr_dir.join("TEMPLATE.md");
    if template_path.exists() {
        let template = tokio::fs::read_to_string(&template_path).await?;
        // Replace the placeholder number with the actual next number
        let filled = template.replace("{YYMMDDHHMM}", &timestamp_id)
            .replace("{NNN}", &timestamp_id);
        println!("{}", filled);
    } else {
        println!("  {} TEMPLATE.md not found", "\u{26a0}".yellow());
    }

    // Output machine-readable JSON for inference engines
    println!("{}", "── Machine-readable (JSON) ──".bold());
    let schema_json = serde_json::json!({
        "next_id": format!("ADR-{}", timestamp_id),
        "id_format": "YYMMDDHHMM",
        "id_readable": human_readable,
        "directory": adr_dir.to_string_lossy(),
        "filename_pattern": "ADR-{YYMMDDHHMM}-{kebab-slug}.md",
        "valid_statuses": ["Proposed", "Accepted", "Completed", "Deprecated", "Superseded", "Abandoned", "Rejected"],
        "required_sections": ["Context", "Decision", "Consequences", "Implementation", "References"],
        "frontmatter_fields": {
            "Status": "required — one of valid_statuses",
            "Date": "required — YYYY-MM-DD",
            "Drivers": "required — what triggered this decision",
            "Supersedes": "optional — ADR-YYMMDDHHMM if replacing an earlier decision"
        }
    });
    println!("{}", serde_json::to_string_pretty(&schema_json)?);

    Ok(())
}

/// Generate a timestamp-based ADR ID in YYMMDDHHMM format (ADR-2026-03-22-1500).
/// This eliminates race conditions from sequential max+1 numbering.
fn generate_timestamp_adr_id() -> String {
    let now = chrono::Local::now();
    now.format("%y%m%d%H%M").to_string()
}

async fn abandoned() -> anyhow::Result<()> {
    let adr_dir = find_adr_dir().ok_or_else(|| anyhow::anyhow!("No docs/adrs/ directory found"))?;
    let adrs = collect_adrs(&adr_dir).await?;

    println!("{} Stale/Abandoned ADR Detection", "\u{2b21}".cyan());
    println!();

    let rows: Vec<AdrAbandonedRow> = adrs
        .iter()
        .filter_map(|(path, content)| {
            let status = parse_adr_status(content);
            let title = extract_title(path, content);

            let is_stale = status == "proposed" || status == "abandoned";
            if is_stale {
                let indicator = if status == "abandoned" {
                    "\u{2717}".red().to_string()
                } else {
                    "?".yellow().to_string()
                };
                Some(AdrAbandonedRow {
                    indicator,
                    title: truncate(&title, 60),
                    status: status_badge(status),
                })
            } else {
                None
            }
        })
        .collect();

    if rows.is_empty() {
        println!("  {}", "No abandoned or stale ADRs found".green());
    } else {
        println!("{}", HexTable::compact(&rows));
        println!();
        println!("  {} ADR(s) need attention", rows.len());
    }

    Ok(())
}

// ── Governance: Applies-To / supersession (ADR-2605301228) ────────────────────

/// Parse the `Applies-To` field — comma-separated areas/globs an ADR governs.
/// Supports `## Applies-To: a, b` heading, `**Applies-To:** a, b` bold, and
/// `applies-to: a, b` / `applies_to: [..]` frontmatter forms.
fn parse_applies_to(content: &str) -> Vec<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();
        let val: Option<&str> = if let Some(r) = trimmed.strip_prefix("## Applies-To:") {
            Some(r)
        } else if lower.starts_with("**applies-to:**") {
            Some(&trimmed["**Applies-To:**".len()..])
        } else if lower.starts_with("applies-to:") {
            Some(&trimmed["applies-to:".len()..])
        } else if lower.starts_with("applies_to:") {
            Some(&trimmed["applies_to:".len()..])
        } else {
            None
        };
        if let Some(v) = val {
            return v
                .split(',')
                .map(|s| s.trim().trim_matches(|c| c == '"' || c == '[' || c == ']' || c == '`').trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }
    Vec::new()
}

/// True if this ADR carries a non-empty `Superseded-By` backlink, so it must
/// NOT be surfaced as a governing decision (the supersession filter — prevents
/// stale decisions resurfacing). The status==superseded case is covered too.
fn is_superseded(content: &str) -> bool {
    if parse_adr_status(content) == "superseded" {
        return true;
    }
    for line in content.lines() {
        let lower = line.trim().to_lowercase();
        if lower.starts_with("**superseded-by:**")
            || lower.starts_with("superseded-by:")
            || lower.starts_with("superseded_by:")
        {
            let v = line
                .splitn(2, ':')
                .nth(1)
                .map(|s| s.trim().trim_matches(|c| c == '*' || c == '"').trim())
                .unwrap_or("");
            let vl = v.to_lowercase();
            if !v.is_empty() && vl != "null" && vl != "none" && v != "\u{2014}" {
                return true;
            }
        }
    }
    false
}

/// Deterministic area match: an `Applies-To` token governs the query if either
/// contains the other (case-insensitive), after stripping trailing glob stars.
fn area_matches(applies: &str, query: &str) -> bool {
    let a = applies
        .trim()
        .trim_end_matches("/**")
        .trim_end_matches("**")
        .trim_end_matches('*')
        .trim_matches('/')
        .to_lowercase();
    let q = query.trim().to_lowercase();
    if a.is_empty() || q.is_empty() {
        return false;
    }
    q.contains(&a) || a.contains(&q)
}

/// `hex adr governing <path>` — list Accepted, non-superseded ADRs whose
/// `Applies-To` matches the path/area. The deterministic backbone of the
/// ADR conflict gate (ADR-2605301228).
async fn governing(path: &str) -> anyhow::Result<()> {
    let adr_dir = find_adr_dir().ok_or_else(|| anyhow::anyhow!("No docs/adrs/ directory found"))?;
    let adrs = collect_adrs(&adr_dir).await?;

    println!("{} ADRs governing '{}'", "\u{2b21}".cyan(), path.bold());
    println!();

    let mut hits: Vec<(String, String, Vec<String>)> = Vec::new();
    let mut accepted_with_applies = 0usize;
    for (p, content) in &adrs {
        // Only Accepted decisions are binding; proposals and superseded are not.
        if parse_adr_status(content) != "accepted" || is_superseded(content) {
            continue;
        }
        let applies = parse_applies_to(content);
        if !applies.is_empty() {
            accepted_with_applies += 1;
        }
        let matched: Vec<String> = applies
            .iter()
            .filter(|a| area_matches(a, path))
            .cloned()
            .collect();
        if !matched.is_empty() {
            let fname = p.file_stem().and_then(|s| s.to_str()).unwrap_or("???");
            hits.push((extract_adr_id(fname), extract_title(p, content), matched));
        }
    }

    if hits.is_empty() {
        println!("  {} No Accepted ADR's Applies-To matches this path.", "\u{2014}".dimmed());
        if accepted_with_applies == 0 {
            println!(
                "  {} No Accepted ADR declares `## Applies-To:` yet — backfill needed (ADR-2605301228).",
                "\u{2139}".dimmed()
            );
        }
    } else {
        for (id, title, areas) in &hits {
            println!("  {} {} — {}", "\u{26a0}".yellow(), id.bold(), truncate(title, 58));
            println!("      governs: {}", areas.join(", ").dimmed());
        }
        println!();
        println!(
            "  {} {} governing ADR(s) — review before changing this path.",
            "\u{2b21}".cyan(),
            hits.len()
        );
    }
    Ok(())
}

// ── Spec-linkage types ───────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct SpecFile {
    feature: String,
    #[serde(default)]
    description: String,
    specs: Vec<SpecScenario>,
}

#[derive(serde::Deserialize)]
struct SpecScenario {
    id: String,
    #[serde(default)]
    category: String,
    description: String,
    #[serde(default)]
    negative_spec: bool,
}

#[derive(Tabled)]
struct SpecRow {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Cat")]
    category: String,
    #[tabled(rename = "Neg")]
    neg: String,
    #[tabled(rename = "Description")]
    description: String,
}

/// `hex adr specs <ADR-id>` — find specs linked to an ADR through workplans.
async fn specs_for_adr(adr_id: &str) -> anyhow::Result<()> {
    let workplans_dir = find_workplans_dir()
        .ok_or_else(|| anyhow::anyhow!("No docs/workplans/ directory found"))?;
    let all_wps = collect_workplans(&workplans_dir).await?;

    let query = adr_id.to_uppercase();

    // Find workplans that reference this ADR
    let linked: Vec<_> = all_wps
        .iter()
        .filter(|(_, wp)| wp.adr.to_uppercase().contains(&query))
        .collect();

    println!("{} Specs linked to {}", "\u{2b21}".cyan(), adr_id.bold());
    println!();

    if linked.is_empty() {
        println!(
            "  {} No workplan references ADR '{}'",
            "\u{26a0}".yellow(),
            adr_id
        );
        println!();
        println!(
            "  {} Workplans link specs to ADRs via the `\"adr\"` field in docs/workplans/*.json",
            "\u{2139}".dimmed()
        );
        return Ok(());
    }

    // Find project root for resolving relative spec paths
    let project_root = workplans_dir
        .parent()  // docs/
        .and_then(|p| p.parent())  // project root
        .map(|p| p.to_path_buf());

    for (wp_path, wp) in &linked {
        let wp_name = wp_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&wp.id);

        println!(
            "  {} workplan: {}",
            "\u{25b6}".green(),
            wp_name.bold()
        );
        if !wp.title.is_empty() {
            println!("    {}", wp.title.dimmed());
        }

        // Resolve and load the linked spec file
        let spec_path_val = match &wp.specs {
            Some(v) => v.clone(),
            None => {
                println!("    {} (no specs field in workplan)", "\u{2014}".dimmed());
                println!();
                continue;
            }
        };

        let spec_rel = match workplan_specs_path(&spec_path_val) {
            Some(p) => p,
            None => {
                println!("    {} (specs field is not a path string)", "\u{2014}".dimmed());
                println!();
                continue;
            }
        };

        println!("    spec: {}", spec_rel.dimmed());
        println!();

        // Try to load the spec file
        let spec_abs = project_root
            .as_ref()
            .map(|root| root.join(&spec_rel))
            .filter(|p| p.exists());

        // Also try find_specs_dir() + filename
        let spec_abs = spec_abs.or_else(|| {
            find_specs_dir().and_then(|d| {
                let fname = Path::new(&spec_rel).file_name()?;
                Some(d.join(fname))
            })
        });

        match spec_abs {
            Some(abs) if abs.exists() => {
                let raw = tokio::fs::read_to_string(&abs).await?;
                match serde_json::from_str::<SpecFile>(&raw) {
                    Ok(spec) => {
                        println!(
                            "    {} — {} ({} scenarios)",
                            spec.feature.bold(),
                            spec.description.dimmed(),
                            spec.specs.len()
                        );
                        println!();

                        let rows: Vec<SpecRow> = spec.specs.iter().map(|s| SpecRow {
                            id: s.id.clone(),
                            category: s.category.clone(),
                            neg: if s.negative_spec { "\u{2212}".red().to_string() } else { String::new() },
                            description: truncate(&s.description, 55),
                        }).collect();

                        println!("{}", HexTable::compact(&rows));
                        println!();
                        println!(
                            "    {} Run `hex spec show {}` for Given/When/Then detail",
                            "\u{2139}".dimmed(),
                            spec.feature
                        );
                    }
                    Err(e) => {
                        println!("    {} Failed to parse spec: {}", "\u{2717}".red(), e);
                    }
                }
            }
            _ => {
                println!("    {} Spec file not found at '{}'", "\u{26a0}".yellow(), spec_rel);
            }
        }

        println!();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_status_accepted() {
        assert_eq!(parse_adr_status("---\nstatus: Accepted\n---\n"), "accepted");
    }

    #[test]
    fn parse_status_proposed() {
        assert_eq!(parse_adr_status("---\nstatus: Proposed\n---\n"), "proposed");
    }

    #[test]
    fn parse_status_bold_markdown() {
        assert_eq!(parse_adr_status("# ADR-001\n\n**Status:** Accepted\n**Date:** 2026-01-01\n"), "accepted");
    }

    #[test]
    fn parse_status_bold_proposed() {
        assert_eq!(parse_adr_status("# ADR\n**Status:** Proposed\n"), "proposed");
    }

    #[test]
    fn parse_status_missing() {
        assert_eq!(parse_adr_status("# ADR-001: No status here\n\nJust text.\n"), "unknown");
    }

    #[test]
    fn parse_status_wrong_format_colon_outside_bold() {
        // **Status**: Accepted  ← colon outside ** — must NOT parse (agent wrote wrong format)
        assert_eq!(parse_adr_status("# ADR-001\n\n**Status**: Accepted\n"), "unknown");
    }

    #[test]
    fn parse_status_bullet_prefix_not_parsed() {
        // - **Status**: Accepted  ← bullet + colon outside — must NOT parse
        assert_eq!(parse_adr_status("# ADR-001\n\n- **Status**: Accepted\n"), "unknown");
    }

    #[test]
    fn parse_status_heading_form_plain() {
        // ## Status\nAccepted — value on the next line
        assert_eq!(parse_adr_status("# ADR-001\n\n## Status\nAccepted\n"), "accepted");
    }

    #[test]
    fn parse_status_heading_form_with_bold_value() {
        // ## Status\n**Accepted** | Open — surrounding bold markers must be stripped
        assert_eq!(
            parse_adr_status("# ADR-001\n\n## Status\n**Accepted** | Open\n"),
            "accepted"
        );
    }

    #[test]
    fn parse_status_heading_form_with_date_suffix() {
        // ## Status\n**Accepted** — 2026-04-10
        assert_eq!(
            parse_adr_status("# ADR-001\n\n## Status\n**Accepted** — 2026-04-10\n"),
            "accepted"
        );
    }

    #[test]
    fn parse_status_heading_form_blank_line_before_value() {
        // ## Status\n\nProposed — blank line between heading and value
        assert_eq!(parse_adr_status("# ADR-001\n\n## Status\n\nProposed\n"), "proposed");
    }

    #[test]
    fn parse_status_heading_form_with_colon() {
        // ## Status: heading with trailing colon
        assert_eq!(parse_adr_status("# ADR-001\n\n## Status:\nAccepted\n"), "accepted");
    }

    #[test]
    fn parse_status_case_insensitive() {
        assert_eq!(parse_adr_status("---\nstatus: ACCEPTED\n---\n"), "accepted");
    }

    #[test]
    fn extract_title_from_heading() {
        let path = std::path::Path::new("ADR-001-test.md");
        assert_eq!(extract_title(path, "# ADR-001: My Title\n"), "ADR-001: My Title");
    }

    #[test]
    fn extract_title_fallback_to_filename() {
        let path = std::path::Path::new("ADR-001-test.md");
        assert_eq!(extract_title(path, "No heading here\n"), "ADR-001-test");
    }

    #[test]
    fn parse_enforced_by_heading() {
        let content = "# ADR\n\n## Enforced-By: hex analyze\n";
        assert_eq!(parse_enforced_by(content), Some("hex analyze".to_string()));
    }

    #[test]
    fn parse_enforced_by_missing() {
        assert_eq!(parse_enforced_by("# ADR\n\nNo enforcement.\n"), None);
    }

    // ── Timestamp ID tests (ADR-2026-03-22-1500) ──

    #[test]
    fn extract_adr_id_legacy() {
        assert_eq!(extract_adr_id("ADR-059-canonical-project-identity"), "ADR-059");
    }

    #[test]
    fn extract_adr_id_timestamp() {
        assert_eq!(extract_adr_id("ADR-2026-03-22-1500-timestamp-ADR-numbering"), "ADR-2026-03-22-1500");
    }

    #[test]
    fn extract_adr_id_case_insensitive() {
        assert_eq!(extract_adr_id("adr-001-foo"), "ADR-001");
    }

    #[test]
    fn extract_adr_id_no_prefix() {
        assert_eq!(extract_adr_id("TEMPLATE"), "TEMPLATE");
    }

    #[test]
    fn generate_timestamp_id_format() {
        let id = generate_timestamp_adr_id();
        // Should be exactly 10 digits (YYMMDDHHMM)
        assert_eq!(id.len(), 10, "Timestamp ID should be 10 digits, got: {}", id);
        assert!(id.chars().all(|c| c.is_ascii_digit()), "Should be all digits: {}", id);
    }

    #[test]
    fn extract_title_timestamp_adr() {
        let path = std::path::Path::new("ADR-2026-03-22-1500-test.md");
        assert_eq!(
            extract_title(path, "# ADR-2026-03-22-1500: My Title\n"),
            "ADR-2026-03-22-1500: My Title"
        );
    }

    // ── Epoch model + index field parsing (ADR-2606071243) ───────────────

    #[test]
    fn epoch_explicit_field_wins() {
        // Date says foundation, but an explicit Epoch field overrides.
        let c = "**Epoch:** single-agent\n## Date: 2026-03-15\n";
        assert_eq!(adr_epoch("ADR-2026-03-22-1500", c), "single-agent");
    }

    #[test]
    fn epoch_derived_from_body_date_over_filename() {
        // Filename is July (single-agent), but the decision Date is org-sim era.
        let c = "**Date:** 2026-06-05\n";
        assert_eq!(adr_epoch("ADR-2026-07-10-1000", c), "org-sim");
    }

    #[test]
    fn epoch_boundaries_by_date() {
        assert_eq!(adr_epoch("ADR-x", "**Date:** 2026-03-15\n"), "foundation");
        assert_eq!(adr_epoch("ADR-x", "**Date:** 2026-04-01\n"), "org-sim");
        assert_eq!(adr_epoch("ADR-x", "**Date:** 2026-06-06\n"), "single-agent");
    }

    #[test]
    fn epoch_legacy_numbered_no_date_is_foundation() {
        // Legacy ADR-NNN with no parseable date predates the timestamp scheme.
        assert_eq!(adr_epoch("ADR-027", ""), "foundation");
        assert_eq!(adr_epoch("ADR-7", ""), "foundation");
    }

    #[test]
    fn epoch_unresolvable_is_unassigned() {
        assert_eq!(adr_epoch("ADR-extensible-validation", ""), "unassigned");
    }

    #[test]
    fn date_parses_heading_form() {
        // Legacy `## Date:` heading form, with a trailing rationale suffix.
        let c = "## Date: 2026-03-15 (rationale expanded 2026-05-17)\n";
        assert_eq!(adr_date("ADR-001", c).as_deref(), Some("2026-03-15"));
    }

    #[test]
    fn date_from_compact_timestamp_id() {
        assert_eq!(adr_date("ADR-2606071243", "").as_deref(), Some("2026-06-07"));
    }

    #[test]
    fn superseded_by_accepts_hyphenated_bold_form() {
        // The canonical TEMPLATE.md form that doctor previously missed.
        let c = "**Superseded-By:** ADR-2606061359\n";
        assert_eq!(parse_superseded_by(c).as_deref(), Some("ADR-2606061359"));
    }

    #[test]
    fn superseded_by_accepts_spaced_form() {
        assert_eq!(
            parse_superseded_by("**Superseded by:** ADR-001\n").as_deref(),
            Some("ADR-001")
        );
    }
}
