//! Filesystem-based ADR review adapter (ADR-041).
//!
//! Performs 5 structural checks WITHOUT LLM inference:
//!   1. Scope conflict detection (shared keyword analysis)
//!   2. Supersession chain validation
//!   3. Duplicate numbering detection
//!   4. Stale reference scan (CLAUDE.md, skills, agents)
//!   5. Metadata validation (Status, Date, Authors)

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::ports::adr_review::*;

/// Parsed metadata from an ADR markdown file.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct AdrMetadata {
    /// e.g. "041"
    number: String,
    /// e.g. "ADR-041"
    id: String,
    /// The # heading title
    title: String,
    /// e.g. "Proposed", "Accepted", etc.
    status: String,
    /// Date field value
    date: String,
    /// "Informed by" references (e.g. ["ADR-040", "ADR-035"])
    informed_by: Vec<String>,
    /// "Supersedes" reference (e.g. Some("ADR-024"))
    supersedes: Option<String>,
    /// Authors field
    authors: String,
    /// Full file content for keyword extraction
    content: String,
    /// File path
    path: PathBuf,
}

pub struct AdrReviewAdapter {
    project_dir: PathBuf,
}

impl AdrReviewAdapter {
    pub fn new(project_dir: PathBuf) -> Self {
        Self { project_dir }
    }

    fn adr_dir(&self) -> PathBuf {
        self.project_dir.join("docs").join("adrs")
    }

    /// Collect and parse all ADR files.
    async fn collect_adrs(&self) -> Result<Vec<AdrMetadata>, String> {
        let adr_dir = self.adr_dir();
        if !adr_dir.is_dir() {
            return Err(format!("ADR directory not found: {}", adr_dir.display()));
        }

        let mut entries: Vec<PathBuf> = std::fs::read_dir(&adr_dir)
            .map_err(|e| format!("Failed to read ADR dir: {e}"))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect();
        entries.sort();

        let mut adrs = Vec::new();
        for path in entries {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
            if let Some(meta) = Self::parse_adr_metadata(&content, &path) {
                adrs.push(meta);
            }
        }
        Ok(adrs)
    }

    /// Parse ADR metadata from markdown content.
    fn parse_adr_metadata(content: &str, path: &Path) -> Option<AdrMetadata> {
        let filename = path.file_stem()?.to_str()?;

        // Extract ADR number from filename. Accepts both shapes:
        //   legacy:    ADR-041-some-title         -> 041
        //   timestamp: ADR-2026-04-12-0202-title  -> 2026-04-12-0202
        // The old impl took only the first dash-segment, so every
        // 2026-prefixed ADR collapsed to `2026` and the duplicate-
        // numbering detector flagged ~160 false positives. Match the
        // walk-digits-and-interior-dashes shape used in hex-cli.
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

        // Extract title from first # heading
        let title = content
            .lines()
            .find(|l| l.trim().starts_with("# "))
            .map(|l| l.trim().trim_start_matches("# ").to_string())
            .unwrap_or_else(|| filename.to_string());

        // Parse frontmatter-style fields (- **Key**: Value)
        let mut status = String::new();
        let mut date = String::new();
        let mut authors = String::new();
        let mut informed_by = Vec::new();
        let mut supersedes = None;

        // Skip lines inside fenced code blocks (``` or ~~~) and only
        // honor the FIRST header value per field. Otherwise a later
        // Rust struct field or comment that mentions `Status:` overrides
        // the real header. Matches the hex-cli adr_review parser fix.
        let mut in_fence = false;
        // Heading-only follow-on tracking (see hex-cli mirror).
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

            // Accept the standard header styles (see hex-cli mirror).
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

    /// Extract the value portion after the first colon in a field line.
    fn extract_field_value(line: &str) -> String {
        if let Some(idx) = line.find(':') {
            line[idx + 1..].trim().to_string()
        } else {
            String::new()
        }
    }

    /// Field-line matcher accepting every ADR header style observed
    /// in the corpus. Mirror of hex-cli/src/commands/adr_review.rs.
    fn field_line(trimmed_lower: &str, key: &str) -> bool {
        trimmed_lower.starts_with(&format!("- **{key}**:"))
            || trimmed_lower.starts_with(&format!("**{key}**:"))
            || trimmed_lower.starts_with(&format!("- **{key}:**"))
            || trimmed_lower.starts_with(&format!("**{key}:**"))
            || trimmed_lower.starts_with(&format!("## {key}:"))
            || trimmed_lower.starts_with(&format!("### {key}:"))
            || trimmed_lower.starts_with(&format!("{key}:"))
    }

    /// Heading-only declaration of `key` — value is on the next line.
    fn heading_only(trimmed_lower: &str, key: &str) -> bool {
        let h2 = format!("## {key}");
        let h3 = format!("### {key}");
        trimmed_lower == h2 || trimmed_lower == h3
    }

    /// Extract ADR-NNN references from a string.
    fn extract_adr_refs(text: &str) -> Vec<String> {
        let mut refs = Vec::new();
        let mut i = 0;
        let bytes = text.as_bytes();
        while i < bytes.len() {
            if i + 4 <= bytes.len() {
                let window = &text[i..];
                if window.starts_with("ADR-") || window.starts_with("adr-") {
                    let rest = &window[4..];
                    let num_end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
                    if num_end > 0 {
                        let num = &rest[..num_end];
                        refs.push(format!("ADR-{num}"));
                        i += 4 + num_end;
                        continue;
                    }
                }
            }
            i += 1;
        }
        refs
    }

    /// Extract domain keywords from ADR title and context section.
    fn extract_keywords(content: &str) -> HashSet<String> {
        // Mirror of hex-cli stop-word set — keep these in sync.
        let stop_words: HashSet<&str> = [
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
            // Project vocabulary — every ADR mentions these.
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
            "old", "future", "today", "still", "instead", "also",
            "since", "without", "within", "across", "every", "ever",
            "always", "never", "often", "sometimes", "while", "until",
            "already", "almost", "even", "though", "however",
            "therefore", "thus", "hence", "phase", "phases", "step",
            "steps", "tier", "tiers", "layer", "layers", "make",
            "made", "let", "lets", "get", "got", "gets", "getting",
            "go", "goes", "going", "want", "wants", "wanted", "like",
            "likes", "liked",
        ].into_iter().collect();

        let mut keywords = HashSet::new();

        // Extract from title (first # heading) and context section
        let mut in_context = false;
        let mut lines_after_context = 0;

        for line in content.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("# ") {
                // Title line - high value keywords
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

    fn add_keywords_from_line(line: &str, stop_words: &HashSet<&str>, keywords: &mut HashSet<String>) {
        for word in line.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_') {
            let w = word.trim().to_lowercase();
            if w.len() >= 3 && !stop_words.contains(w.as_str()) && !w.chars().all(|c| c.is_ascii_digit()) {
                keywords.insert(w);
            }
        }
    }

    // ── Check 1: Scope Conflict ──

    fn check_scope_conflict(target: &AdrMetadata, all: &[AdrMetadata]) -> Vec<ReviewFinding> {
        let target_keywords = Self::extract_keywords(&target.content);
        let mut findings = Vec::new();

        for other in all {
            if other.id == target.id {
                continue;
            }
            let other_keywords = Self::extract_keywords(&other.content);
            let shared: Vec<_> = target_keywords.intersection(&other_keywords).collect();

            // Threshold + severity matched to hex-cli mirror.
            if shared.len() > 8 {
                let mut shared_sorted: Vec<_> = shared.into_iter().cloned().collect();
                shared_sorted.sort();
                shared_sorted.truncate(8);
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

    // ── Check 2: Supersession Chain ──

    fn check_supersession_chain(target: &AdrMetadata, all: &[AdrMetadata]) -> Vec<ReviewFinding> {
        let mut findings = Vec::new();

        // If target says "Informed by: ADR-X", check if ADR-X is marked as superseded
        for ref_id in &target.informed_by {
            if let Some(referenced) = all.iter().find(|a| a.id == *ref_id) {
                let ref_status = referenced.status.to_lowercase();
                // If target explicitly supersedes ref_id, the referenced ADR should be marked superseded
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

        // If target is marked superseded, check it mentions the successor
        if target.status.to_lowercase().contains("superseded") {
            // Look for "Superseded by ADR-NNN" in the status or content
            let refs_in_status = Self::extract_adr_refs(&target.status);
            if refs_in_status.is_empty() {
                // Check first 20 lines for supersession reference
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

    // ── Check 3: Duplicate Numbering ──

    fn check_duplicate_numbering(all: &[AdrMetadata]) -> Vec<ReviewFinding> {
        let mut findings = Vec::new();
        let mut by_number: HashMap<&str, Vec<&AdrMetadata>> = HashMap::new();

        for adr in all {
            if !adr.number.is_empty() {
                by_number.entry(&adr.number).or_default().push(adr);
            }
        }

        for (num, adrs) in &by_number {
            if adrs.len() > 1 {
                let names: Vec<_> = adrs.iter().map(|a| a.path.file_name().unwrap().to_string_lossy().to_string()).collect();
                findings.push(ReviewFinding {
                    severity: Severity::Critical,
                    check: "duplicate_numbering".to_string(),
                    adr_a: format!("ADR-{num}"),
                    adr_b: None,
                    description: format!(
                        "Duplicate ADR number {num}: {}",
                        names.join(", ")
                    ),
                    recommendation: "Renumber one of the duplicate ADRs".to_string(),
                    file: None,
                    line: None,
                });
            }
        }

        findings
    }

    // ── Check 4: Stale References ──

    fn check_stale_references(all: &[AdrMetadata], project_dir: &Path) -> Vec<ReviewFinding> {
        let mut findings = Vec::new();

        // Build a map of superseded/abandoned ADR IDs
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

        // Files to scan for ADR references
        let files_to_scan: Vec<PathBuf> = [
            project_dir.join("CLAUDE.md"),
        ]
        .into_iter()
        .filter(|p| p.is_file())
        .collect();

        // Also scan skills/*.md and agents/*.yml
        for dir_name in &["skills", ".claude/skills", "agents", ".claude/agents"] {
            let dir = project_dir.join(dir_name);
            if dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_file() {
                            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
                            if ext == "md" || ext == "yml" || ext == "yaml" {
                                // files_to_scan already defined as Vec, just push
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
                            recommendation: format!(
                                "Update {} to reference the successor ADR",
                                rel_path
                            ),
                            file: Some(rel_path),
                            line: Some((line_num + 1) as u32),
                        });
                    }
                }
            }
        }

        findings
    }

    // ── Check 5: Metadata Validation ──

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
            // Check if the status contains one of the valid values
            let is_valid = valid_statuses.iter().any(|s| status_lower.contains(s));
            if !is_valid {
                findings.push(ReviewFinding {
                    severity: Severity::Info,
                    check: "metadata_validation".to_string(),
                    adr_a: adr.id.clone(),
                    adr_b: None,
                    description: format!(
                        "{} has non-standard status '{}'. Expected one of: {}",
                        adr.id,
                        adr.status,
                        valid_statuses.join(", ")
                    ),
                    recommendation: "Use a standard ADR status value".to_string(),
                    file: Some(adr.path.display().to_string()),
                    line: None,
                });
            }
        }

        // Missing Date is suppressed when the file is git-tracked — the
        // commit log is the canonical date. Matches the hex-cli mirror.
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

        // Authors is opt-in (env: HEX_ADR_REVIEW_REQUIRE_AUTHORS=1). The
        // hex ADR corpus uses the git log as the attribution layer
        // rather than a hand-typed Authors header.
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

    /// True if the file is tracked in the repo's git history.
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

    /// Compute verdict from findings.
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

#[async_trait]
impl IAdrReviewPort for AdrReviewAdapter {
    async fn review_adr(&self, adr_id: &str) -> Result<ReviewReport, String> {
        let all = self.collect_adrs().await?;

        // Find the target ADR by ID (e.g. "ADR-041") or partial match
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
            .ok_or_else(|| format!("ADR not found: {adr_id}"))?;

        let mut findings = Vec::new();

        // Check 1: Scope conflict
        findings.extend(Self::check_scope_conflict(target, &all));

        // Check 2: Supersession chain
        findings.extend(Self::check_supersession_chain(target, &all));

        // Check 3: Duplicate numbering (global, but filtered to target)
        for f in Self::check_duplicate_numbering(&all) {
            if f.adr_a == target.id {
                findings.push(f);
            }
        }

        // Check 4: Stale references mentioning this ADR
        for f in Self::check_stale_references(&all, &self.project_dir) {
            if f.adr_a == target.id {
                findings.push(f);
            }
        }

        // Check 5: Metadata validation
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

    async fn review_all(&self) -> Result<Vec<ReviewReport>, String> {
        let all = self.collect_adrs().await?;
        let mut reports = Vec::new();

        // Global checks (run once)
        let dup_findings = Self::check_duplicate_numbering(&all);
        let stale_findings = Self::check_stale_references(&all, &self.project_dir);

        for adr in &all {
            let mut findings = Vec::new();

            // Check 1: Scope conflict
            findings.extend(Self::check_scope_conflict(adr, &all));

            // Check 2: Supersession chain
            findings.extend(Self::check_supersession_chain(adr, &all));

            // Check 3: Duplicate numbering (filtered)
            for f in &dup_findings {
                if f.adr_a == adr.id {
                    findings.push(f.clone());
                }
            }

            // Check 4: Stale references (filtered)
            for f in &stale_findings {
                if f.adr_a == adr.id {
                    findings.push(f.clone());
                }
            }

            // Check 5: Metadata
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
}
