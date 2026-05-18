//! `hex adr review` — ADR consistency review (ADR-041).
//!
//! Runs 5 structural checks locally without requiring the nexus daemon.

use std::path::PathBuf;

use colored::Colorize;

/// Run the ADR review command.
///
/// - `adr_id`: Optional specific ADR to review (e.g. "ADR-040")
/// - `strict`: If true, exit with code 1 on any WARNING+ findings
pub async fn run(adr_id: Option<String>, strict: bool) -> anyhow::Result<()> {
    // Find project root (walk up to find docs/adrs/)
    let project_dir = find_project_root()?;

    // We inline the adapter logic here to avoid a hex-nexus dependency.
    // This mirrors hex_nexus::adapters::adr_review::AdrReviewAdapter.
    let adapter = LocalAdrReviewer::new(project_dir);

    match adr_id {
        Some(id) => {
            let report = adapter.review_adr(&id).await?;
            print_report(&report);
            if strict && has_actionable_findings(&report.findings) {
                std::process::exit(1);
            }
        }
        None => {
            let reports = adapter.review_all().await?;
            let mut total_findings = 0;
            let mut has_actionable = false;

            // Print a summary header
            println!("{} ADR Review Report", "\u{2b21}".cyan());
            println!();

            // Aggregate findings across all reports
            let mut all_findings = Vec::new();
            for report in &reports {
                all_findings.extend(report.findings.clone());
            }

            // Deduplicate by (check, adr_a, adr_b, description)
            all_findings.sort_by(|a, b| {
                (&a.check, &a.adr_a, &a.adr_b, &a.description)
                    .cmp(&(&b.check, &b.adr_a, &b.adr_b, &b.description))
            });
            all_findings.dedup_by(|a, b| {
                a.check == b.check
                    && a.adr_a == b.adr_a
                    && a.adr_b == b.adr_b
                    && a.description == b.description
            });

            if all_findings.is_empty() {
                println!("  {} All ADRs pass structural review", "\u{2713}".green());
            } else {
                // Group by check type
                let check_order = [
                    "duplicate_numbering",
                    "stale_reference",
                    "supersession_chain",
                    "scope_conflict",
                    "metadata_validation",
                ];

                for check_name in &check_order {
                    let check_findings: Vec<_> = all_findings
                        .iter()
                        .filter(|f| f.check == *check_name)
                        .collect();

                    if check_findings.is_empty() {
                        continue;
                    }

                    let label = match *check_name {
                        "duplicate_numbering" => "Duplicate Numbering",
                        "stale_reference" => "Stale References",
                        "supersession_chain" => "Supersession Chain",
                        "scope_conflict" => "Scope Conflicts",
                        "metadata_validation" => "Metadata Validation",
                        _ => check_name,
                    };

                    println!("  {} {}", "\u{25cf}".bold(), label.bold());

                    for finding in &check_findings {
                        let severity_str = match finding.severity {
                            Severity::Critical => "CRITICAL".red().bold().to_string(),
                            Severity::Warning => "WARNING".yellow().to_string(),
                            Severity::Info => "INFO".dimmed().to_string(),
                        };

                        let target = if let Some(ref b) = finding.adr_b {
                            format!("{} <-> {}", finding.adr_a, b)
                        } else {
                            finding.adr_a.clone()
                        };

                        println!("    [{severity_str}] {target}");
                        println!("      {}", finding.description);
                        println!("      {}: {}", "Fix".dimmed(), finding.recommendation);

                        if let (Some(ref file), Some(line)) = (&finding.file, finding.line) {
                            println!("      {} {}:{}", "at".dimmed(), file, line);
                        }
                        println!();
                        total_findings += 1;

                        if finding.severity != Severity::Info {
                            has_actionable = true;
                        }
                    }
                }

                // Verdicts summary
                let blocking = reports.iter().filter(|r| r.verdict == ReviewVerdict::Blocking).count();
                let needs_action = reports.iter().filter(|r| r.verdict == ReviewVerdict::NeedsAction).count();
                let pass = reports.iter().filter(|r| r.verdict == ReviewVerdict::Pass).count();

                println!("  {} findings across {} ADRs", total_findings, reports.len());
                if blocking > 0 {
                    println!("  {} {} BLOCKING", "\u{2717}".red(), blocking);
                }
                if needs_action > 0 {
                    println!("  {} {} NEEDS_ACTION", "!".yellow(), needs_action);
                }
                println!("  {} {} PASS", "\u{2713}".green(), pass);
            }

            if strict && has_actionable {
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

fn print_report(report: &ReviewReport) {
    println!("{} ADR Review: {}", "\u{2b21}".cyan(), report.reviewed_adr.bold());
    println!("  Verdict: {}", match report.verdict {
        ReviewVerdict::Pass => "PASS".green().to_string(),
        ReviewVerdict::NeedsAction => "NEEDS_ACTION".yellow().to_string(),
        ReviewVerdict::Blocking => "BLOCKING".red().bold().to_string(),
    });
    println!();

    if report.findings.is_empty() {
        println!("  {} No issues found", "\u{2713}".green());
        return;
    }

    for finding in &report.findings {
        let severity_str = match finding.severity {
            Severity::Critical => "CRITICAL".red().bold().to_string(),
            Severity::Warning => "WARNING".yellow().to_string(),
            Severity::Info => "INFO".dimmed().to_string(),
        };

        let target = if let Some(ref b) = finding.adr_b {
            format!("{} <-> {}", finding.adr_a, b)
        } else {
            finding.adr_a.clone()
        };

        println!("  [{severity_str}] {} — {}", target, finding.check);
        println!("    {}", finding.description);
        println!("    {}: {}", "Fix".dimmed(), finding.recommendation);

        if let (Some(ref file), Some(line)) = (&finding.file, finding.line) {
            println!("    {} {}:{}", "at".dimmed(), file, line);
        }
        println!();
    }

    println!("  {} finding(s)", report.findings.len());
}

fn has_actionable_findings(findings: &[ReviewFinding]) -> bool {
    findings.iter().any(|f| f.severity != Severity::Info)
}

fn find_project_root() -> anyhow::Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    let mut dir = cwd.as_path();
    loop {
        if dir.join("docs").join("adrs").is_dir() {
            return Ok(dir.to_path_buf());
        }
        dir = dir.parent().ok_or_else(|| {
            anyhow::anyhow!("No docs/adrs/ directory found in any parent directory")
        })?;
    }
}

// ── Local types mirroring hex_nexus::ports::adr_review ──

#[derive(Debug, Clone, PartialEq, Eq)]
enum Severity {
    Critical,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReviewVerdict {
    Pass,
    NeedsAction,
    Blocking,
}

#[derive(Debug, Clone)]
struct ReviewFinding {
    severity: Severity,
    check: String,
    adr_a: String,
    adr_b: Option<String>,
    description: String,
    recommendation: String,
    file: Option<String>,
    line: Option<u32>,
}

#[derive(Debug, Clone)]
struct ReviewReport {
    reviewed_adr: String,
    #[allow(dead_code)]
    timestamp: String,
    findings: Vec<ReviewFinding>,
    verdict: ReviewVerdict,
}

#[derive(Debug, Clone)]
struct AdrMetadata {
    number: String,
    id: String,
    #[allow(dead_code)]
    title: String,
    status: String,
    date: String,
    informed_by: Vec<String>,
    supersedes: Option<String>,
    authors: String,
    content: String,
    path: PathBuf,
}

// ── Local reviewer (no hex-nexus dependency) ──

struct LocalAdrReviewer {
    project_dir: PathBuf,
}

impl LocalAdrReviewer {
    fn new(project_dir: PathBuf) -> Self {
        Self { project_dir }
    }

    fn adr_dir(&self) -> PathBuf {
        self.project_dir.join("docs").join("adrs")
    }

    async fn collect_adrs(&self) -> anyhow::Result<Vec<AdrMetadata>> {
        let adr_dir = self.adr_dir();
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&adr_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
            // Exclude scaffolding files that live in docs/adrs/ but
            // aren't decision records themselves (README index,
            // TEMPLATE skeleton). They have no real Status/Date
            // header so the metadata checks always flag them.
            .filter(|p| {
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                let upper = stem.to_uppercase();
                upper != "README" && upper != "TEMPLATE"
            })
            .collect();
        entries.sort();

        let mut adrs = Vec::new();
        for path in entries {
            let content = tokio::fs::read_to_string(&path).await?;
            if let Some(meta) = Self::parse_adr_metadata(&content, &path) {
                adrs.push(meta);
            }
        }
        Ok(adrs)
    }

    fn parse_adr_metadata(content: &str, path: &std::path::Path) -> Option<AdrMetadata> {
        let filename = path.file_stem()?.to_str()?;

        // Accept both legacy (`ADR-047`) and timestamp
        // (`ADR-2026-04-12-0202`) ID shapes. The old impl took only
        // the first dash-separated segment, so every 2026-prefixed
        // ADR collapsed to number=`2026` — triggering 161 spurious
        // "duplicate numbering" findings in hex adr review. Walk
        // digits-then-interior-dashes the same way `adr_id_from_filename`
        // and `extract_adr_id` do.
        let number = filename
            .strip_prefix("ADR-")
            .or_else(|| filename.strip_prefix("adr-"))
            .map(|rest| {
                let chars: Vec<char> = rest.chars().collect();
                let mut out = String::new();
                let mut i = 0;
                while i < chars.len() {
                    let c = chars[i];
                    if c.is_ascii_digit() {
                        out.push(c);
                        i += 1;
                    } else if c == '-'
                        && i + 1 < chars.len()
                        && chars[i + 1].is_ascii_digit()
                    {
                        out.push(c);
                        i += 1;
                    } else {
                        break;
                    }
                }
                out
            })
            .unwrap_or_default();

        let id = if number.is_empty() {
            filename.to_string()
        } else {
            format!("ADR-{number}")
        };

        let title = content
            .lines()
            .find(|l| l.trim().starts_with("# "))
            .map(|l| l.trim().trim_start_matches("# ").to_string())
            .unwrap_or_else(|| filename.to_string());

        let mut status = String::new();
        let mut date = String::new();
        let mut authors = String::new();
        let mut informed_by = Vec::new();
        let mut supersedes = None;

        // Track fenced code blocks (```…```) so we don't pick up `Status:`
        // mentions inside a Rust struct field or comment block. The old
        // parser flagged ~9 ADRs with bogus "non-standard status" findings
        // (e.g. ADR-012 ingested the schema definition line and ended up
        // with status = "From '## Status:' or '**Status:**' lines …",
        // which contains the literal substring 'superseded' and tripped
        // the supersession-chain detector).
        //
        // Also: only accept the FIRST status line, so a later code-fenced
        // line can't override the header.
        let mut in_fence = false;
        // When a previous line was a heading-only declaration like
        // `## Status`, the value lives on the next non-empty line.
        // `awaiting_<field>` flags ride to the following iterations.
        let mut awaiting_status = false;
        let mut awaiting_date = false;
        let mut awaiting_authors = false;
        for line in content.lines() {
            let trimmed_raw = line.trim();
            if trimmed_raw.starts_with("```") || trimmed_raw.starts_with("~~~") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence {
                continue;
            }
            let trimmed = trimmed_raw.to_lowercase();

            // Heading-only follow-on: the previous line was `## Status`
            // (no inline value); grab the first non-empty line that
            // follows.
            if (awaiting_status || awaiting_date || awaiting_authors)
                && !trimmed_raw.is_empty()
            {
                if awaiting_status && status.is_empty() {
                    status = trimmed_raw.to_string();
                }
                if awaiting_date && date.is_empty() {
                    date = trimmed_raw.to_string();
                }
                if awaiting_authors && authors.is_empty() {
                    authors = trimmed_raw.to_string();
                }
                awaiting_status = false;
                awaiting_date = false;
                awaiting_authors = false;
                continue;
            }
            if trimmed_raw.is_empty() {
                continue;
            }

            // Heading-only forms (`## Status`, `## Date`, `## Authors`)
            // arm the awaiting_* flag for the next iteration.
            if status.is_empty() && Self::heading_only(&trimmed, "status") {
                awaiting_status = true;
                continue;
            }
            if date.is_empty() && Self::heading_only(&trimmed, "date") {
                awaiting_date = true;
                continue;
            }
            if authors.is_empty() && Self::heading_only(&trimmed, "authors") {
                awaiting_authors = true;
                continue;
            }

            // Accept three bold styles for every field:
            //   - **Key**: value       (bullet, colon outside)
            //   **Key**: value         (no bullet, colon outside)
            //   **Key:** value         (no bullet, colon inside the bold — *the
            //                           style used by ~60% of existing ADRs,
            //                           including ADR-012; missing this style
            //                           is what made the parser walk past the
            //                           real header and grab a later code or
            //                           list-item line.)
            //   Key: value             (plain, no bold)
            if status.is_empty() && Self::field_line(&trimmed, "status") {
                status = Self::extract_field_value(line);
            } else if date.is_empty() && Self::field_line(&trimmed, "date") {
                date = Self::extract_field_value(line);
            } else if authors.is_empty() && Self::field_line(&trimmed, "authors") {
                authors = Self::extract_field_value(line);
            } else if informed_by.is_empty() && Self::field_line(&trimmed, "informed by") {
                let val = Self::extract_field_value(line);
                informed_by = Self::extract_adr_refs(&val);
            } else if supersedes.is_none() && Self::field_line(&trimmed, "supersedes") {
                let val = Self::extract_field_value(line);
                let refs = Self::extract_adr_refs(&val);
                supersedes = refs.into_iter().next();
            }
        }

        Some(AdrMetadata {
            number,
            id,
            title,
            status,
            date,
            informed_by,
            supersedes,
            authors,
            content: content.to_string(),
            path: path.to_path_buf(),
        })
    }

    fn extract_field_value(line: &str) -> String {
        if let Some(idx) = line.find(':') {
            line[idx + 1..].trim().to_string()
        } else {
            String::new()
        }
    }

    /// Field-line matcher accepting every ADR header style observed in
    /// the corpus:
    ///   - **Key**: value
    ///   **Key**: value
    ///   **Key:** value     (colon inside bold — ~60% of corpus)
    ///   Key: value
    ///   ## Key: value      (heading with inline value — ~6 ADRs use this)
    ///   ## Key             (heading-only; value is on the next line —
    ///                       handled separately at the parse-state level)
    fn field_line(trimmed_lower: &str, key: &str) -> bool {
        trimmed_lower.starts_with(&format!("- **{key}**:"))
            || trimmed_lower.starts_with(&format!("**{key}**:"))
            || trimmed_lower.starts_with(&format!("- **{key}:**"))
            || trimmed_lower.starts_with(&format!("**{key}:**"))
            || trimmed_lower.starts_with(&format!("## {key}:"))
            || trimmed_lower.starts_with(&format!("### {key}:"))
            || trimmed_lower.starts_with(&format!("{key}:"))
    }

    /// True if the line is a heading-only declaration of `key` —
    /// the value will be on a subsequent non-empty line. Matches:
    ///   `## Status`
    ///   `## Status` (with trailing whitespace)
    fn heading_only(trimmed_lower: &str, key: &str) -> bool {
        let h2 = format!("## {key}");
        let h3 = format!("### {key}");
        trimmed_lower == h2 || trimmed_lower == h3
    }

    /// True if the file is tracked in the repo's git history. Used to
    /// suppress "missing Date" findings when the commit log already
    /// supplies it. Mirrors `commands::adr::doctor::file_git_age_days`.
    fn is_git_tracked(path: &std::path::Path) -> bool {
        let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
        let filename = match path.file_name().and_then(|s| s.to_str()) {
            Some(f) => f,
            None => return false,
        };
        std::process::Command::new("git")
            .arg("-C")
            .arg(parent)
            .args(["log", "-1", "--format=%H", "--"])
            .arg(filename)
            .output()
            .map(|o| o.status.success() && !o.stdout.is_empty())
            .unwrap_or(false)
    }

    fn extract_adr_refs(text: &str) -> Vec<String> {
        let mut refs = Vec::new();
        let mut skip_until = 0;
        for (i, _) in text.char_indices() {
            if i < skip_until {
                continue;
            }
            let window = &text[i..];
            if window.starts_with("ADR-") || window.starts_with("adr-") {
                let rest = &window[4..];
                let num_end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
                if num_end > 0 {
                    refs.push(format!("ADR-{}", &rest[..num_end]));
                    skip_until = i + 4 + num_end;
                }
            }
        }
        refs
    }

    fn extract_keywords(content: &str) -> std::collections::HashSet<String> {
        use std::collections::HashSet;

        let stop_words: HashSet<&str> = [
            // ── English function words ────────────────────────────────
            "the", "a", "an", "is", "are", "was", "were", "be", "been", "being",
            "have", "has", "had", "do", "does", "did", "will", "would", "could",
            "should", "may", "might", "must", "shall", "can", "need", "dare",
            "to", "of", "in", "for", "on", "with", "at", "by", "from", "as",
            "into", "through", "during", "before", "after", "above", "below",
            "between", "out", "off", "over", "under", "again", "further", "then",
            "once", "and", "but", "or", "nor", "not", "so", "yet", "both", "each",
            "all", "any", "few", "more", "most", "other", "some", "such", "no",
            "only", "own", "same", "than", "too", "very", "this", "that", "these",
            "those", "it", "its", "if", "when", "where", "how", "what", "which",
            "who", "whom", "why", "we", "they", "them", "their", "our", "you",
            "i", "me", "my", "he", "she", "his", "her", "up", "about",
            // ── Project vocabulary that EVERY ADR mentions ──────────
            // Without these, the scope_conflict detector flags every
            // pair as "high overlap" just because both ADRs say "hex"
            // and "module" — the signal is noise.
            "hex", "adr", "adrs", "module", "src", "path", "paths",
            "assets", "components", "change", "changes", "decision",
            "context", "etc", "new", "required", "needs", "filter",
            "kanban", "board", "cto", "ceo", "cpo", "ciso", "coo",
            "agent", "agents", "code", "test", "tests", "data", "type",
            "types", "field", "fields", "value", "values", "status",
            "state", "states", "id", "ids", "config", "file", "files",
            "system", "node", "nodes", "name", "names", "key", "keys",
            "use", "used", "uses", "using", "via", "per", "see", "rule",
            "rules", "check", "checks", "case", "cases", "way", "ways",
            "set", "sets", "list", "lists", "one", "two", "three",
            "first", "second", "next", "last", "now", "current",
            "old", "current", "future", "new", "today", "still",
            "instead", "also", "since", "without", "within", "across",
            "every", "ever", "always", "never", "often", "sometimes",
            "while", "until", "still", "already", "almost", "even",
            "though", "however", "therefore", "thus", "hence",
            "phase", "phases", "step", "steps", "tier", "tiers",
            "layer", "layers", "make", "made", "let", "lets", "let's",
            "get", "got", "gets", "getting", "go", "goes", "going",
            "want", "wants", "wanted", "like", "likes", "liked",
        ].into_iter().collect();

        let mut keywords = HashSet::new();
        let mut in_context = false;
        let mut lines_after_context = 0;

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("# ") {
                Self::add_keywords_from_line(trimmed, &stop_words, &mut keywords);
                continue;
            }
            if trimmed.starts_with("## Context") || trimmed.starts_with("## Decision") {
                in_context = true;
                lines_after_context = 0;
                continue;
            }
            if in_context {
                if trimmed.starts_with("## ") && !trimmed.starts_with("## Context") && !trimmed.starts_with("## Decision") {
                    in_context = false;
                    continue;
                }
                lines_after_context += 1;
                if lines_after_context <= 30 {
                    Self::add_keywords_from_line(trimmed, &stop_words, &mut keywords);
                }
            }
        }
        keywords
    }

    fn add_keywords_from_line(line: &str, stop_words: &std::collections::HashSet<&str>, keywords: &mut std::collections::HashSet<String>) {
        for word in line.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_') {
            let w = word.trim().to_lowercase();
            if w.len() >= 3 && !stop_words.contains(w.as_str()) && !w.chars().all(|c| c.is_ascii_digit()) {
                keywords.insert(w);
            }
        }
    }

    async fn review_adr(&self, adr_id: &str) -> anyhow::Result<ReviewReport> {
        let all = self.collect_adrs().await?;

        let target = all
            .iter()
            .find(|a| {
                a.id.eq_ignore_ascii_case(adr_id)
                    || a.path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_lowercase().contains(&adr_id.to_lowercase()))
                        .unwrap_or(false)
            })
            .ok_or_else(|| anyhow::anyhow!("ADR not found: {adr_id}"))?;

        let mut findings = Vec::new();
        findings.extend(Self::check_scope_conflict(target, &all));
        findings.extend(Self::check_supersession_chain(target, &all));
        for f in Self::check_duplicate_numbering(&all) {
            if f.adr_a == target.id {
                findings.push(f);
            }
        }
        for f in Self::check_stale_references(&all, &self.project_dir) {
            if f.adr_a == target.id {
                findings.push(f);
            }
        }
        findings.extend(Self::check_metadata(target));

        let verdict = Self::compute_verdict(&findings);
        let timestamp = chrono::Utc::now().to_rfc3339();

        Ok(ReviewReport {
            reviewed_adr: target.id.clone(),
            timestamp,
            findings,
            verdict,
        })
    }

    async fn review_all(&self) -> anyhow::Result<Vec<ReviewReport>> {
        let all = self.collect_adrs().await?;
        let mut reports = Vec::new();

        let dup_findings = Self::check_duplicate_numbering(&all);
        let stale_findings = Self::check_stale_references(&all, &self.project_dir);

        for adr in &all {
            let mut findings = Vec::new();
            findings.extend(Self::check_scope_conflict(adr, &all));
            findings.extend(Self::check_supersession_chain(adr, &all));
            for f in &dup_findings {
                if f.adr_a == adr.id {
                    findings.push(f.clone());
                }
            }
            for f in &stale_findings {
                if f.adr_a == adr.id {
                    findings.push(f.clone());
                }
            }
            findings.extend(Self::check_metadata(adr));

            let verdict = Self::compute_verdict(&findings);
            let timestamp = chrono::Utc::now().to_rfc3339();

            reports.push(ReviewReport {
                reviewed_adr: adr.id.clone(),
                timestamp,
                findings,
                verdict,
            });
        }

        Ok(reports)
    }

    fn check_scope_conflict(target: &AdrMetadata, all: &[AdrMetadata]) -> Vec<ReviewFinding> {
        let target_keywords = Self::extract_keywords(&target.content);
        let mut findings = Vec::new();

        for other in all {
            if other.id == target.id {
                continue;
            }
            let other_keywords = Self::extract_keywords(&other.content);
            let shared: Vec<_> = target_keywords.intersection(&other_keywords).collect();
            // Threshold raised from >3 to >8: even with the wider
            // stopword list, ADRs in the same problem area routinely
            // share ~4-6 nouns. >8 catches genuine scope-overlap pairs
            // (two ADRs about the same subsystem) without flagging
            // every "they both talk about caching" coincidence.
            if shared.len() > 8 {
                let mut shared_sorted: Vec<_> = shared.into_iter().cloned().collect();
                shared_sorted.sort();
                shared_sorted.truncate(8);
                // Scope conflict is advisory, not a violation. Lowered
                // from Warning to Info so it doesn't roll into the
                // NEEDS_ACTION verdict — every pair of ADRs in the
                // same problem domain will share keywords by design.
                findings.push(ReviewFinding {
                    severity: Severity::Info,
                    check: "scope_conflict".to_string(),
                    adr_a: target.id.clone(),
                    adr_b: Some(other.id.clone()),
                    description: format!(
                        "Shared domain keywords ({} overlap): {}",
                        shared_sorted.len(),
                        shared_sorted.join(", ")
                    ),
                    recommendation: format!(
                        "{} should reference {} and clarify scope boundaries",
                        target.id, other.id
                    ),
                    file: None,
                    line: None,
                });
            }
        }
        findings
    }

    fn check_supersession_chain(target: &AdrMetadata, all: &[AdrMetadata]) -> Vec<ReviewFinding> {
        let mut findings = Vec::new();

        for ref_id in &target.informed_by {
            if let Some(referenced) = all.iter().find(|a| a.id == *ref_id) {
                let ref_status = referenced.status.to_lowercase();
                if target.supersedes.as_deref() == Some(ref_id.as_str())
                    && !ref_status.contains("superseded")
                {
                    findings.push(ReviewFinding {
                        severity: Severity::Warning,
                        check: "supersession_chain".to_string(),
                        adr_a: target.id.clone(),
                        adr_b: Some(ref_id.clone()),
                        description: format!(
                            "{} supersedes {} but {} status is '{}', not 'Superseded'",
                            target.id, ref_id, ref_id, referenced.status
                        ),
                        recommendation: format!("Update {} status to 'Superseded by {}'", ref_id, target.id),
                        file: Some(referenced.path.display().to_string()),
                        line: None,
                    });
                }
            }
        }

        if target.status.to_lowercase().contains("superseded") {
            let refs_in_status = Self::extract_adr_refs(&target.status);
            if refs_in_status.is_empty() {
                let has_successor_ref = target.content.lines().take(20).any(|l| {
                    let lower = l.to_lowercase();
                    lower.contains("superseded by") && l.contains("ADR-")
                });
                if !has_successor_ref {
                    findings.push(ReviewFinding {
                        severity: Severity::Info,
                        check: "supersession_chain".to_string(),
                        adr_a: target.id.clone(),
                        adr_b: None,
                        description: format!(
                            "{} is marked Superseded but doesn't reference its successor",
                            target.id
                        ),
                        recommendation: format!(
                            "Update {} status to 'Superseded by ADR-NNN'",
                            target.id
                        ),
                        file: Some(target.path.display().to_string()),
                        line: None,
                    });
                }
            }
        }

        findings
    }

    fn check_duplicate_numbering(all: &[AdrMetadata]) -> Vec<ReviewFinding> {
        use std::collections::HashMap;
        let mut findings = Vec::new();
        let mut by_number: HashMap<&str, Vec<&AdrMetadata>> = HashMap::new();

        for adr in all {
            if !adr.number.is_empty() {
                by_number.entry(&adr.number).or_default().push(adr);
            }
        }

        for (num, adrs) in &by_number {
            if adrs.len() > 1 {
                let names: Vec<_> = adrs.iter().map(|a| {
                    a.path.file_name().unwrap().to_string_lossy().to_string()
                }).collect();
                findings.push(ReviewFinding {
                    severity: Severity::Critical,
                    check: "duplicate_numbering".to_string(),
                    adr_a: format!("ADR-{num}"),
                    adr_b: None,
                    description: format!("Duplicate ADR number {num}: {}", names.join(", ")),
                    recommendation: "Renumber one of the duplicate ADRs".to_string(),
                    file: None,
                    line: None,
                });
            }
        }
        findings
    }

    fn check_stale_references(all: &[AdrMetadata], project_dir: &std::path::Path) -> Vec<ReviewFinding> {
        use std::collections::HashMap;
        let mut findings = Vec::new();

        let stale_ids: HashMap<String, &AdrMetadata> = all
            .iter()
            .filter(|a| {
                let s = a.status.to_lowercase();
                s.contains("superseded") || s.contains("abandoned")
            })
            .map(|a| (a.id.clone(), a))
            .collect();

        if stale_ids.is_empty() {
            return findings;
        }

        let mut files_to_scan: Vec<PathBuf> = Vec::new();
        let claude_md = project_dir.join("CLAUDE.md");
        if claude_md.is_file() {
            files_to_scan.push(claude_md);
        }

        for dir_name in &["skills", ".claude/skills", "agents", ".claude/agents"] {
            let dir = project_dir.join(dir_name);
            if dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_file() {
                            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
                            if ext == "md" || ext == "yml" || ext == "yaml" {
                                files_to_scan.push(p);
                            }
                        }
                    }
                }
            }
        }

        for file_path in &files_to_scan {
            let content = match std::fs::read_to_string(file_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            for (line_num, line) in content.lines().enumerate() {
                let refs = Self::extract_adr_refs(line);
                for adr_ref in refs {
                    if let Some(stale_adr) = stale_ids.get(&adr_ref) {
                        let rel_path = file_path
                            .strip_prefix(project_dir)
                            .unwrap_or(file_path)
                            .display()
                            .to_string();
                        findings.push(ReviewFinding {
                            severity: Severity::Critical,
                            check: "stale_reference".to_string(),
                            adr_a: adr_ref.clone(),
                            adr_b: None,
                            description: format!(
                                "{} references {} which has status '{}'",
                                rel_path, adr_ref, stale_adr.status
                            ),
                            recommendation: format!("Update {} to reference the successor ADR", rel_path),
                            file: Some(rel_path),
                            line: Some((line_num + 1) as u32),
                        });
                    }
                }
            }
        }
        findings
    }

    fn check_metadata(adr: &AdrMetadata) -> Vec<ReviewFinding> {
        let mut findings = Vec::new();
        let valid_statuses = [
            "proposed", "accepted", "superseded", "abandoned", "deferred", "deprecated",
        ];

        if adr.status.is_empty() {
            findings.push(ReviewFinding {
                severity: Severity::Warning,
                check: "metadata_validation".to_string(),
                adr_a: adr.id.clone(),
                adr_b: None,
                description: format!("{} is missing a Status field", adr.id),
                recommendation: "Add '- **Status**: Proposed' to the ADR header".to_string(),
                file: Some(adr.path.display().to_string()),
                line: None,
            });
        } else {
            let status_lower = adr.status.to_lowercase();
            let is_valid = valid_statuses.iter().any(|s| status_lower.contains(s));
            if !is_valid {
                findings.push(ReviewFinding {
                    severity: Severity::Info,
                    check: "metadata_validation".to_string(),
                    adr_a: adr.id.clone(),
                    adr_b: None,
                    description: format!(
                        "{} has non-standard status '{}'. Expected one of: {}",
                        adr.id, adr.status, valid_statuses.join(", ")
                    ),
                    recommendation: "Use a standard ADR status value".to_string(),
                    file: Some(adr.path.display().to_string()),
                    line: None,
                });
            }
        }

        // Missing Date is only a finding when the file ISN'T in git
        // history. For tracked files the commit log is the canonical
        // date — adding a hand-typed Date line just creates churn and
        // a second source of truth. This mirrors the suppression in
        // adr::doctor::missing_date_suppressed_when_file_tracked_in_git.
        if adr.date.is_empty() && !Self::is_git_tracked(&adr.path) {
            findings.push(ReviewFinding {
                severity: Severity::Info,
                check: "metadata_validation".to_string(),
                adr_a: adr.id.clone(),
                adr_b: None,
                description: format!("{} is missing a Date field", adr.id),
                recommendation: "Add '- **Date**: YYYY-MM-DD' to the ADR header".to_string(),
                file: Some(adr.path.display().to_string()),
                line: None,
            });
        }

        // Authors is intentionally NOT required. The hex ADR corpus has
        // never tracked individual authors — the git log is the
        // attribution layer. Flagging missing Authors on every ADR
        // produced 210 spurious findings; project convention is "no
        // Authors field" for everything except the rare cases where
        // it adds context (factory-authored vs human-authored). Detect
        // a MALFORMED Authors line if needed, but missing-by-default
        // is the documented convention.
        //
        // To re-enable: set HEX_ADR_REVIEW_REQUIRE_AUTHORS=1.
        if adr.authors.is_empty()
            && std::env::var("HEX_ADR_REVIEW_REQUIRE_AUTHORS").is_ok()
        {
            findings.push(ReviewFinding {
                severity: Severity::Info,
                check: "metadata_validation".to_string(),
                adr_a: adr.id.clone(),
                adr_b: None,
                description: format!("{} is missing an Authors field", adr.id),
                recommendation: "Add '- **Authors**: <name>' to the ADR header".to_string(),
                file: Some(adr.path.display().to_string()),
                line: None,
            });
        }
        findings
    }

    fn compute_verdict(findings: &[ReviewFinding]) -> ReviewVerdict {
        if findings.iter().any(|f| f.severity == Severity::Critical) {
            ReviewVerdict::Blocking
        } else if findings.iter().any(|f| f.severity == Severity::Warning) {
            ReviewVerdict::NeedsAction
        } else {
            ReviewVerdict::Pass
        }
    }
}
