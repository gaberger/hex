//! Architecture Fingerprint Extractor — ADR-2603301200.
//!
//! Generates a token-efficient summary of a project's architectural intent
//! from local sources (manifest files, workplan, ADRs). The fingerprint is
//! stored in SpacetimeDB and injected into every inference system prompt to
//! prevent models from hallucinating wrong frameworks or output types.

use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use tokio::fs;
use tracing::{debug, warn};

// ── Output types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureFingerprint {
    pub project_id: String,
    pub language: String,
    pub framework: String,
    /// "cli" | "web-api" | "library" | "framework" | "standalone"
    pub output_type: String,
    /// "hexagonal" | "layered" | "standalone"
    pub architecture_style: String,
    /// Up to 5 hard constraints
    pub constraints: Vec<String>,
    /// Workspace crate names (Rust workspaces only)
    #[serde(default)]
    pub workspace_crates: Vec<String>,
    /// Up to 3 active ADR summaries: {id, summary}
    pub active_adrs: Vec<AdrSummary>,
    pub workplan_objective: String,
    /// Estimated token count of the formatted injection block
    pub fingerprint_tokens: u32,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdrSummary {
    pub id: String,
    pub summary: String,
}

impl ArchitectureFingerprint {
    /// Render as the injection block prepended to inference system prompts.
    pub fn to_injection_block(&self) -> String {
        let mut lines = vec![
            "## Project Architecture Context".to_string(),
            format!(
                "Language: {} | Style: {} | Framework: {} | Output: {}",
                self.language, self.architecture_style, self.framework, self.output_type
            ),
        ];

        if !self.workplan_objective.is_empty() {
            lines.push(format!("Objective: {}", self.workplan_objective));
        }

        // For Rust workspaces, list the crate names so the LLM knows where code belongs
        if !self.workspace_crates.is_empty() {
            lines.push(format!("Crates: {}", self.workspace_crates.join(", ")));
        }

        if !self.constraints.is_empty() {
            lines.push("Constraints:".to_string());
            for c in &self.constraints {
                lines.push(format!("- {}", c));
            }
        }

        if !self.active_adrs.is_empty() {
            lines.push("Active ADRs:".to_string());
            for a in &self.active_adrs {
                lines.push(format!("- {}: {}", a.id, a.summary));
            }
        }

        lines.push("---".to_string());
        lines.join("\n")
    }
}

// ── Extractor ────────────────────────────────────────────────────────────────

pub struct FingerprintExtractor;

impl FingerprintExtractor {
    /// Extract an architecture fingerprint from a project directory.
    ///
    /// `project_id` — the hex project UUID.
    /// `project_root` — absolute path to the project.
    /// `workplan_path` — optional path to the workplan JSON file.
    pub async fn extract(
        project_id: &str,
        project_root: &Path,
        workplan_path: Option<&Path>,
    ) -> ArchitectureFingerprint {
        let (language, framework) = detect_language_framework(project_root).await;
        let architecture_style = detect_architecture_style(project_root, &language);

        let (mut workplan_objective, mut constraints) =
            extract_workplan_context(project_root, workplan_path).await;

        // Fallback objective: read from CLAUDE.md or README.md first paragraph
        if workplan_objective.is_empty() {
            workplan_objective = extract_objective_from_docs(project_root).await;
        }

        let output_type = infer_output_type(&workplan_objective, &language, &framework, project_root);

        // If workplan gave no constraints, derive from architecture_style + output_type
        if constraints.is_empty() {
            constraints = derive_default_constraints(&architecture_style, &output_type, &language, &framework);
        }
        constraints.truncate(5);

        let active_adrs = extract_adr_summaries(project_root).await;
        let workspace_crates = extract_workspace_crates(project_root).await;

        let fp = ArchitectureFingerprint {
            project_id: project_id.to_string(),
            language,
            framework,
            output_type,
            architecture_style,
            constraints,
            workspace_crates,
            active_adrs,
            workplan_objective,
            fingerprint_tokens: 0, // computed below
            generated_at: chrono_now(),
        };

        // Compute token estimate and enforce budget
        enforce_token_budget(fp)
    }
}

// ── Language + framework detection ───────────────────────────────────────────

async fn detect_language_framework(root: &Path) -> (String, String) {
    // Go
    if let Ok(content) = fs::read_to_string(root.join("go.mod")).await {
        let framework = detect_go_framework(&content);
        return ("go".to_string(), framework);
    }

    // Rust
    if let Ok(content) = fs::read_to_string(root.join("Cargo.toml")).await {
        let framework = detect_rust_framework(&content);
        return ("rust".to_string(), framework);
    }

    // TypeScript / Node
    if let Ok(content) = fs::read_to_string(root.join("package.json")).await {
        let framework = detect_ts_framework(&content);
        return ("typescript".to_string(), framework);
    }

    ("unknown".to_string(), "none".to_string())
}

fn detect_go_framework(go_mod: &str) -> String {
    for line in go_mod.lines() {
        let trimmed = line.trim();
        if trimmed.contains("github.com/gin-gonic/gin") { return "gin".to_string(); }
        if trimmed.contains("github.com/labstack/echo")  { return "echo".to_string(); }
        if trimmed.contains("github.com/gofiber/fiber")  { return "fiber".to_string(); }
        if trimmed.contains("github.com/gorilla/mux")    { return "gorilla/mux".to_string(); }
    }
    "stdlib".to_string()
}

fn detect_rust_framework(cargo_toml: &str) -> String {
    let lower = cargo_toml.to_lowercase();
    if lower.contains("axum")       { return "axum".to_string(); }
    if lower.contains("actix-web")  { return "actix-web".to_string(); }
    if lower.contains("warp")       { return "warp".to_string(); }
    if lower.contains("tonic")      { return "tonic".to_string(); }
    if lower.contains("rocket")     { return "rocket".to_string(); }
    // For workspace root Cargo.toml that lists members, signal it's a workspace
    if lower.contains("[workspace]") { return "workspace".to_string(); }
    "none".to_string()
}

fn detect_ts_framework(pkg_json: &str) -> String {
    let lower = pkg_json.to_lowercase();
    if lower.contains("\"next\"")        { return "nextjs".to_string(); }
    if lower.contains("\"express\"")     { return "express".to_string(); }
    if lower.contains("\"fastify\"")     { return "fastify".to_string(); }
    if lower.contains("\"hono\"")        { return "hono".to_string(); }
    if lower.contains("\"@nestjs/core\"") { return "nestjs".to_string(); }
    "none".to_string()
}

// ── Architecture style ────────────────────────────────────────────────────────

fn detect_architecture_style(root: &Path, language: &str) -> String {
    // Hexagonal: has src/core/domain/ (TypeScript hex convention)
    if root.join("src").join("core").join("domain").is_dir() {
        return "hexagonal".to_string();
    }
    // Layered: Go convention — has internal/ or cmd/
    if language == "go"
        && (root.join("internal").is_dir() || root.join("cmd").is_dir())
    {
        return "layered".to_string();
    }
    // Rust workspace layered: has multiple crates
    if language == "rust" && root.join("Cargo.toml").is_file() {
        if let Ok(content) = std::fs::read_to_string(root.join("Cargo.toml")) {
            if content.contains("[workspace]") {
                return "layered".to_string();
            }
        }
    }
    "standalone".to_string()
}

// ── Workplan extraction ───────────────────────────────────────────────────────

async fn extract_workplan_context(
    root: &Path,
    workplan_path: Option<&Path>,
) -> (String, Vec<String>) {
    // Try explicit path first, then scan docs/workplans/
    let paths_to_try: Vec<PathBuf> = if let Some(p) = workplan_path {
        vec![p.to_path_buf()]
    } else {
        find_latest_workplan(root).await
    };

    for path in &paths_to_try {
        if let Ok(content) = fs::read_to_string(path).await {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                let objective = val["description"]
                    .as_str()
                    .or_else(|| {
                        val["phases"].as_array()
                            .and_then(|p| p.first())
                            .and_then(|p| p["tasks"].as_array())
                            .and_then(|t| t.first())
                            .and_then(|t| t["description"].as_str())
                    })
                    .unwrap_or("")
                    .chars().take(400).collect::<String>();

                let constraints: Vec<String> = val["phases"]
                    .as_array()
                    .and_then(|phases| phases.first())
                    .and_then(|p| p["tasks"].as_array())
                    .and_then(|tasks| tasks.first())
                    .and_then(|t| t["constraints"].as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str())
                            .map(|s| s.to_string())
                            .collect()
                    })
                    .unwrap_or_default();

                if !objective.is_empty() {
                    debug!(path = %path.display(), "extracted workplan context");
                    return (objective, constraints);
                }
            }
        }
    }

    (String::new(), Vec::new())
}

async fn find_latest_workplan(root: &Path) -> Vec<PathBuf> {
    let workplan_dir = root.join("docs").join("workplans");
    let mut paths = Vec::new();
    if let Ok(mut entries) = fs::read_dir(&workplan_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("json") {
                paths.push(p);
            }
        }
    }
    // Sort descending by filename (most recent first by date prefix)
    paths.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    paths
}

/// Extract workspace crate names from a Rust workspace Cargo.toml.
///
/// Reads the `[workspace] members` list and returns the directory names as crate names.
async fn extract_workspace_crates(root: &Path) -> Vec<String> {
    let cargo_path = root.join("Cargo.toml");
    let content = match fs::read_to_string(&cargo_path).await {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    if !content.contains("[workspace]") {
        return Vec::new();
    }
    let mut crates = Vec::new();
    let mut in_members = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("members") && trimmed.contains('=') {
            in_members = true;
        }
        if in_members {
            // Extract quoted crate names: "hex-cli", "hex-nexus", etc.
            let mut pos = 0;
            while let Some(start) = trimmed[pos..].find('"') {
                let abs_start = pos + start + 1;
                if let Some(end) = trimmed[abs_start..].find('"') {
                    let name = &trimmed[abs_start..abs_start + end];
                    if !name.is_empty() && !name.contains('*') {
                        crates.push(name.to_string());
                    }
                    pos = abs_start + end + 1;
                } else {
                    break;
                }
            }
            // End of members list once we've seen the closing bracket
            if trimmed.contains(']') {
                break;
            }
        }
    }
    crates.truncate(8); // cap to avoid token bloat
    crates
}

/// Extract a short project objective from CLAUDE.md or README.md.
///
/// Looks for the first non-empty, non-heading line that describes what the project is.
async fn extract_objective_from_docs(root: &Path) -> String {
    // Try CLAUDE.md first — it has the most concise project description
    for filename in &["CLAUDE.md", "README.md"] {
        let path = root.join(filename);
        if let Ok(content) = fs::read_to_string(&path).await {
            for line in content.lines() {
                let trimmed = line.trim();
                // Skip headings, blank lines, html comments, and code blocks
                if trimmed.is_empty()
                    || trimmed.starts_with('#')
                    || trimmed.starts_with("<!--")
                    || trimmed.starts_with("```")
                    || trimmed.starts_with('>')
                    || trimmed.len() < 20
                {
                    continue;
                }
                let obj: String = trimmed.chars().take(400).collect();
                if !obj.is_empty() {
                    return obj;
                }
            }
        }
    }
    String::new()
}

// ── Output type inference ─────────────────────────────────────────────────────

fn infer_output_type(objective: &str, language: &str, framework: &str, root: &Path) -> String {
    let lower = objective.to_lowercase();

    // Rust workspace with hexagonal src/ = framework/toolchain
    if framework == "workspace" && root.join("src").join("core").join("domain").is_dir() {
        return "framework".to_string();
    }

    // Web framework implies web API
    if matches!(framework, "gin" | "echo" | "fiber" | "axum" | "actix-web" | "express" | "fastify" | "hono" | "nestjs") {
        return "web-api".to_string();
    }

    if lower.contains(" cli") || lower.contains("command") || lower.contains("terminal")
        || lower.contains("tool") || lower.contains("command-line")
    {
        return "cli".to_string();
    }
    if lower.contains("api") || lower.contains("server") || lower.contains("rest")
        || lower.contains("http") || lower.contains("endpoint")
    {
        return "web-api".to_string();
    }
    if lower.contains("library") || lower.contains("crate") || lower.contains("package")
        || lower.contains("sdk") || lower.contains("framework")
    {
        return "library".to_string();
    }

    // Go binaries default to CLI unless framework says otherwise
    if language == "go" { return "cli".to_string(); }

    "standalone".to_string()
}

// ── Default constraints ───────────────────────────────────────────────────────

fn derive_default_constraints(architecture_style: &str, output_type: &str, language: &str, framework: &str) -> Vec<String> {
    // Hexagonal architecture always gets boundary rules — they're the defining constraints
    if architecture_style == "hexagonal" {
        return vec![
            "domain/ imports nothing outside domain/".to_string(),
            "ports/ may only import from domain/".to_string(),
            "adapters/ may only import from ports/ (never other adapters)".to_string(),
            "composition-root is the ONLY file that imports adapters".to_string(),
            "All relative imports MUST use .js extensions (NodeNext)".to_string(),
        ];
    }
    match output_type {
        "cli" => vec![
            format!("CLI tool — no web framework, no HTTP server (language: {})", language),
            "Output to stdout; read input from stdin or command-line args".to_string(),
            "Use os.Exit(0) on success, os.Exit(1) on error".to_string(),
        ],
        "web-api" => vec![
            format!("Web API using {} (language: {})", framework, language),
            "Return JSON responses with appropriate HTTP status codes".to_string(),
            "Handle errors with structured JSON error bodies".to_string(),
        ],
        "library" | "framework" => vec![
            format!("Library/framework — no main() entry point unless in a binary crate (language: {})", language),
            "Export a clean public API; keep implementation private".to_string(),
        ],
        _ => vec![
            format!("Standalone {} project", language),
        ],
    }
}

// ── ADR extraction ────────────────────────────────────────────────────────────

async fn extract_adr_summaries(root: &Path) -> Vec<AdrSummary> {
    let adr_dir = root.join("docs").join("adrs");
    let mut adr_files: Vec<PathBuf> = Vec::new();

    if let Ok(mut entries) = fs::read_dir(&adr_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("md")
                && p.file_name().and_then(|n| n.to_str())
                    .map(|n| n.starts_with("ADR-"))
                    .unwrap_or(false)
            {
                adr_files.push(p);
            }
        }
    }

    // Most recent first
    adr_files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    adr_files.truncate(3);

    let mut summaries = Vec::new();
    for path in &adr_files {
        let id = path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("ADR-unknown")
            .to_string();

        if let Ok(content) = fs::read_to_string(path).await {
            let summary = extract_decision_first_sentence(&content);
            summaries.push(AdrSummary { id, summary });
        }
    }

    summaries
}

/// Extract the first sentence from the ## Decision section of an ADR.
fn extract_decision_first_sentence(content: &str) -> String {
    let mut in_decision = false;
    for line in content.lines() {
        if line.trim_start().starts_with("## Decision") {
            in_decision = true;
            continue;
        }
        if in_decision {
            if line.starts_with("## ") {
                break; // next section
            }
            let trimmed = line.trim();
            if trimmed.is_empty() { continue; }
            // Take up to first sentence end or 120 chars
            let sentence: String = trimmed.chars().take(200).collect();
            let end = sentence.find(". ").map(|i| i + 1).unwrap_or(sentence.len());
            return sentence[..end].to_string();
        }
    }
    String::new()
}

// ── Token budget enforcement ──────────────────────────────────────────────────

fn enforce_token_budget(mut fp: ArchitectureFingerprint) -> ArchitectureFingerprint {
    const BUDGET: u32 = 512;

    let estimate = |f: &ArchitectureFingerprint| -> u32 {
        (f.to_injection_block().len() / 4) as u32
    };

    fp.fingerprint_tokens = estimate(&fp);

    if fp.fingerprint_tokens > BUDGET {
        // Drop ADRs first
        fp.active_adrs.clear();
        fp.fingerprint_tokens = estimate(&fp);
    }

    if fp.fingerprint_tokens > BUDGET {
        // Truncate constraints
        while fp.constraints.len() > 2 && estimate(&fp) > BUDGET {
            fp.constraints.pop();
        }
        fp.fingerprint_tokens = estimate(&fp);
    }

    if fp.fingerprint_tokens > BUDGET {
        warn!(tokens = fp.fingerprint_tokens, "fingerprint still over budget after trimming");
    }

    fp
}

// ── Utility ───────────────────────────────────────────────────────────────────

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Format as ISO 8601 (simple implementation without chrono dep)
    let (y, mo, d, h, mi, s) = unix_to_parts(secs);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, mi, s)
}

fn unix_to_parts(mut ts: u64) -> (u64, u64, u64, u64, u64, u64) {
    let s = ts % 60; ts /= 60;
    let mi = ts % 60; ts /= 60;
    let h = ts % 24; ts /= 24;
    // Days since epoch → approximate date (good enough for audit purposes)
    let mut year = 1970u64;
    loop {
        let days_in_year = if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) { 366 } else { 365 };
        if ts < days_in_year { break; }
        ts -= days_in_year;
        year += 1;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [31u64, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u64;
    for &days in &month_days {
        if ts < days { break; }
        ts -= days;
        month += 1;
    }
    (year, month, ts + 1, h, mi, s)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_stdlib_detected() {
        let go_mod = "module example.com/fizzbuzz\n\ngo 1.21\n";
        assert_eq!(detect_go_framework(go_mod), "stdlib");
    }

    #[test]
    fn go_gin_detected() {
        let go_mod = "module example.com/api\n\nrequire github.com/gin-gonic/gin v1.9.0\n";
        assert_eq!(detect_go_framework(go_mod), "gin");
    }

    #[test]
    fn rust_axum_detected() {
        let toml = "[dependencies]\naxum = \"0.7\"\ntokio = { version = \"1\" }\n";
        assert_eq!(detect_rust_framework(toml), "axum");
    }

    #[test]
    fn output_type_cli_from_objective() {
        let p = std::path::Path::new("/tmp");
        assert_eq!(infer_output_type("build a fizzbuzz CLI in Go", "go", "stdlib", p), "cli");
        assert_eq!(infer_output_type("build a command-line tool", "go", "stdlib", p), "cli");
    }

    #[test]
    fn output_type_web_api_from_framework() {
        let p = std::path::Path::new("/tmp");
        assert_eq!(infer_output_type("some project", "go", "gin", p), "web-api");
        assert_eq!(infer_output_type("some project", "rust", "axum", p), "web-api");
    }

    #[test]
    fn output_type_web_api_from_objective() {
        let p = std::path::Path::new("/tmp");
        assert_eq!(infer_output_type("build a REST API for todos", "go", "stdlib", p), "web-api");
    }

    #[test]
    fn decision_sentence_extraction() {
        let adr = "# ADR-001\n\n## Context\n\nSome context.\n\n## Decision\n\nWe will use hexagonal architecture. More details follow.\n\n## Consequences\n\n...";
        assert_eq!(extract_decision_first_sentence(adr), "We will use hexagonal architecture.");
    }

    #[test]
    fn injection_block_format() {
        let fp = ArchitectureFingerprint {
            project_id: "test".into(),
            language: "go".into(),
            framework: "stdlib".into(),
            output_type: "cli".into(),
            architecture_style: "standalone".into(),
            constraints: vec!["No HTTP server".into()],
            workspace_crates: vec![],
            active_adrs: vec![],
            workplan_objective: "build a fizzbuzz CLI".into(),
            fingerprint_tokens: 50,
            generated_at: "2026-03-30T12:00:00Z".into(),
        };
        let block = fp.to_injection_block();
        assert!(block.contains("## Project Architecture Context"));
        assert!(block.contains("Language: go"));
        assert!(block.contains("Objective: build a fizzbuzz CLI"));
        assert!(block.contains("No HTTP server"));
        assert!(block.ends_with("---"));
    }

    #[test]
    fn token_budget_enforced() {
        // Build a fingerprint that is over budget
        let fat_constraints: Vec<String> = (0..10)
            .map(|i| format!("This is a very long constraint number {} that takes up many tokens in the context window", i))
            .collect();
        let fat_adrs: Vec<AdrSummary> = (0..5)
            .map(|i| AdrSummary { id: format!("ADR-{}", i), summary: "A".repeat(200) })
            .collect();
        let fp = ArchitectureFingerprint {
            project_id: "test".into(),
            language: "go".into(),
            framework: "stdlib".into(),
            output_type: "cli".into(),
            architecture_style: "standalone".into(),
            constraints: fat_constraints,
            workspace_crates: vec![],
            active_adrs: fat_adrs,
            workplan_objective: "test".into(),
            fingerprint_tokens: 0,
            generated_at: "2026-03-30T12:00:00Z".into(),
        };
        let trimmed = enforce_token_budget(fp);
        assert!(trimmed.fingerprint_tokens <= 512 || trimmed.active_adrs.is_empty());
    }
}
