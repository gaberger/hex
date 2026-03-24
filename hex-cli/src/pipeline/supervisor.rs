//! Supervisor — context-aware agent orchestration for `hex dev` pipeline.
//!
//! The `Supervisor` assembles [`AgentContext`] structs tailored to each agent
//! role (coder, reviewer, tester, documenter, UX, fixer).  Context builders
//! read source files from disk, attach boundary rules, and include upstream
//! output so that inference calls carry exactly the information the agent needs.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use tracing::{debug, warn};

// ── Constants ────────────────────────────────────────────────────────────

/// Maximum bytes to include per source file in agent context.
const MAX_FILE_BYTES: usize = 4096;

/// Hex boundary rules — shared with fix_agent and validate_phase.
const BOUNDARY_RULES_TIER0: &str = "\
Tier 0 (Domain + Ports):
1. domain/ must only import from domain/
2. ports/ may import from domain/ but nothing else
3. No external dependencies allowed in domain/";

const BOUNDARY_RULES_TIER1: &str = "\
Tier 1 (Secondary Adapters):
1. adapters/secondary/ may import from ports/ only
2. Adapters must NEVER import other adapters
3. All relative imports MUST use .js extensions (NodeNext module resolution)";

const BOUNDARY_RULES_TIER2: &str = "\
Tier 2 (Primary Adapters):
1. adapters/primary/ may import from ports/ only
2. Adapters must NEVER import other adapters
3. All relative imports MUST use .js extensions (NodeNext module resolution)";

const BOUNDARY_RULES_TIER3: &str = "\
Tier 3 (Use Cases):
1. usecases/ may import from domain/ and ports/ only
2. Must not import from adapters/
3. All relative imports MUST use .js extensions (NodeNext module resolution)";

const BOUNDARY_RULES_TIER4: &str = "\
Tier 4+ (Composition Root / Integration):
1. composition-root is the ONLY file that imports from adapters
2. All relative imports MUST use .js extensions (NodeNext module resolution)
3. Domain, ports, usecases, and adapters rules still apply within their directories";

// ── AgentContext ─────────────────────────────────────────────────────────

/// Everything an agent needs to perform its task — assembled by [`Supervisor`].
#[derive(Debug, Clone)]
pub struct AgentContext {
    /// Prompt template name (e.g., `"agent-coder"`).
    pub prompt_template: String,
    /// Source files to include in context: `(relative_path, content)`.
    pub source_files: Vec<(String, String)>,
    /// Port interface files relevant to this task: `(relative_path, content)`.
    pub port_interfaces: Vec<(String, String)>,
    /// Architecture boundary rules for the target tier.
    pub boundary_rules: String,
    /// The workplan step being worked on.
    pub workplan_step: Option<String>,
    /// Output from an upstream agent (e.g., reviewer issues for fixer).
    pub upstream_output: Option<String>,
    /// Task-specific metadata.
    pub metadata: HashMap<String, String>,
}

// ── Supervisor ───────────────────────────────────────────────────────────

/// Assembles [`AgentContext`] structs for each agent role.
///
/// The supervisor knows the output directory layout and reads source files
/// from disk so inference calls carry exactly the context each agent needs.
pub struct Supervisor {
    output_dir: String,
    language: String,
    nexus_url: String,
    model_override: Option<String>,
    provider_pref: Option<String>,
}

impl Supervisor {
    /// Create a new supervisor targeting `output_dir` with the given language.
    ///
    /// `nexus_url` defaults to `HEX_NEXUS_URL` env var or `http://localhost:5555`.
    pub fn new(output_dir: &str, language: &str) -> Self {
        let nexus_url = std::env::var("HEX_NEXUS_URL")
            .unwrap_or_else(|_| "http://localhost:5555".to_string());
        let model_override = std::env::var("HEX_MODEL").ok();
        let provider_pref = std::env::var("HEX_PROVIDER").ok();

        Self {
            output_dir: output_dir.to_string(),
            language: language.to_string(),
            nexus_url,
            model_override,
            provider_pref,
        }
    }

    /// Nexus base URL (for callers that need to make REST calls).
    pub fn nexus_url(&self) -> &str {
        &self.nexus_url
    }

    /// Model override from `HEX_MODEL` env var, if set.
    pub fn model_override(&self) -> Option<&str> {
        self.model_override.as_deref()
    }

    /// Provider preference from `HEX_PROVIDER` env var, if set.
    pub fn provider_pref(&self) -> Option<&str> {
        self.provider_pref.as_deref()
    }

    // ── File readers ─────────────────────────────────────────────────────

    /// Read source files from the directories belonging to `tier`.
    ///
    /// - Tier 0: `src/core/domain/` + `src/core/ports/`
    /// - Tier 1: `src/adapters/secondary/`
    /// - Tier 2: `src/adapters/primary/`
    /// - Tier 3: `src/core/usecases/`
    /// - Tier 4+: `src/` (all)
    fn files_for_tier(&self, tier: u32) -> Vec<(String, String)> {
        let base = PathBuf::from(&self.output_dir).join("src");
        let dirs: Vec<PathBuf> = match tier {
            0 => vec![
                base.join("core").join("domain"),
                base.join("core").join("ports"),
            ],
            1 => vec![base.join("adapters").join("secondary")],
            2 => vec![base.join("adapters").join("primary")],
            3 => vec![base.join("core").join("usecases")],
            _ => vec![base],
        };

        let mut files = Vec::new();
        for dir in dirs {
            Self::collect_files(&dir, &self.output_dir, &mut files);
        }
        files
    }

    /// Read port interface files from `src/core/ports/`.
    fn port_files(&self) -> Vec<(String, String)> {
        let ports_dir = PathBuf::from(&self.output_dir)
            .join("src")
            .join("core")
            .join("ports");
        let mut files = Vec::new();
        Self::collect_files(&ports_dir, &self.output_dir, &mut files);
        files
    }

    /// Get boundary rules text for a tier.
    fn rules_for_tier(&self, tier: u32) -> String {
        match tier {
            0 => BOUNDARY_RULES_TIER0.to_string(),
            1 => BOUNDARY_RULES_TIER1.to_string(),
            2 => BOUNDARY_RULES_TIER2.to_string(),
            3 => BOUNDARY_RULES_TIER3.to_string(),
            _ => BOUNDARY_RULES_TIER4.to_string(),
        }
    }

    /// Read a single file from disk, truncating to [`MAX_FILE_BYTES`].
    /// Returns `None` if the file cannot be read.
    fn read_file_truncated(path: &Path) -> Option<String> {
        match fs::read_to_string(path) {
            Ok(content) => {
                if content.len() > MAX_FILE_BYTES {
                    debug!(
                        path = %path.display(),
                        "truncating file from {} to {} bytes",
                        content.len(),
                        MAX_FILE_BYTES,
                    );
                    let truncated = &content[..MAX_FILE_BYTES];
                    // Find last newline to avoid cutting mid-line
                    let end = truncated.rfind('\n').unwrap_or(MAX_FILE_BYTES);
                    Some(format!("{}\n// ... truncated ({} bytes total)", &content[..end], content.len()))
                } else {
                    Some(content)
                }
            }
            Err(e) => {
                warn!(path = %path.display(), error = %e, "skipping unreadable file");
                None
            }
        }
    }

    /// Recursively collect source files from `dir`, storing `(relative_path, content)`.
    fn collect_files(dir: &Path, base: &str, out: &mut Vec<(String, String)>) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                Self::collect_files(&path, base, out);
            } else if path.is_file() {
                // Only include source files
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if matches!(ext, "ts" | "js" | "rs" | "tsx" | "jsx") {
                    if let Some(content) = Self::read_file_truncated(&path) {
                        let rel = path
                            .strip_prefix(base)
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|_| path.display().to_string());
                        out.push((rel, content));
                    }
                }
            }
        }
    }

    /// Read a single source file, returning `(relative_path, content)`.
    fn read_single_file(&self, file_path: &str) -> Vec<(String, String)> {
        let path = PathBuf::from(&self.output_dir).join(file_path);
        match Self::read_file_truncated(&path) {
            Some(content) => vec![(file_path.to_string(), content)],
            None => vec![],
        }
    }

    // ── Context builders ─────────────────────────────────────────────────

    /// Build context for a **coder** agent working on a workplan step at `tier`.
    pub fn build_coder_context(&self, step_desc: &str, tier: u32) -> AgentContext {
        let mut metadata = HashMap::new();
        metadata.insert("language".into(), self.language.clone());
        metadata.insert("tier".into(), tier.to_string());

        AgentContext {
            prompt_template: "agent-coder".into(),
            source_files: self.files_for_tier(tier),
            port_interfaces: self.port_files(),
            boundary_rules: self.rules_for_tier(tier),
            workplan_step: Some(step_desc.to_string()),
            upstream_output: None,
            metadata,
        }
    }

    /// Build context for a **reviewer** agent examining a specific file.
    pub fn build_reviewer_context(&self, file_path: &str) -> AgentContext {
        let tier = Self::infer_tier_from_path(file_path);
        let mut metadata = HashMap::new();
        metadata.insert("language".into(), self.language.clone());
        metadata.insert("review_target".into(), file_path.to_string());

        AgentContext {
            prompt_template: "agent-reviewer".into(),
            source_files: self.read_single_file(file_path),
            port_interfaces: self.port_files(),
            boundary_rules: self.rules_for_tier(tier),
            workplan_step: None,
            upstream_output: None,
            metadata,
        }
    }

    /// Build context for a **tester** agent writing tests for a specific file.
    pub fn build_tester_context(&self, file_path: &str) -> AgentContext {
        let tier = Self::infer_tier_from_path(file_path);
        let mut metadata = HashMap::new();
        metadata.insert("language".into(), self.language.clone());
        metadata.insert("test_target".into(), file_path.to_string());

        AgentContext {
            prompt_template: "agent-tester".into(),
            source_files: self.read_single_file(file_path),
            port_interfaces: self.port_files(),
            boundary_rules: self.rules_for_tier(tier),
            workplan_step: None,
            upstream_output: None,
            metadata,
        }
    }

    /// Build context for a **documenter** agent writing/updating documentation.
    pub fn build_documenter_context(&self, adr_content: &str, workplan_summary: &str) -> AgentContext {
        let mut metadata = HashMap::new();
        metadata.insert("language".into(), self.language.clone());
        metadata.insert("adr_content".into(), adr_content.to_string());
        metadata.insert("workplan_summary".into(), workplan_summary.to_string());

        AgentContext {
            prompt_template: "agent-documenter".into(),
            source_files: vec![],
            port_interfaces: self.port_files(),
            boundary_rules: String::new(),
            workplan_step: None,
            upstream_output: None,
            metadata,
        }
    }

    /// Build context for a **UX** agent improving a UI component.
    pub fn build_ux_context(&self, file_path: &str, user_desc: &str) -> AgentContext {
        let mut metadata = HashMap::new();
        metadata.insert("language".into(), self.language.clone());
        metadata.insert("ux_target".into(), file_path.to_string());
        metadata.insert("user_description".into(), user_desc.to_string());

        AgentContext {
            prompt_template: "agent-ux".into(),
            source_files: self.read_single_file(file_path),
            port_interfaces: self.port_files(),
            boundary_rules: self.rules_for_tier(2), // primary adapter rules
            workplan_step: None,
            upstream_output: None,
            metadata,
        }
    }

    /// Build context for a **fixer** agent resolving an issue in a file.
    pub fn build_fixer_context(&self, file_path: &str, issue_desc: &str) -> AgentContext {
        let tier = Self::infer_tier_from_path(file_path);
        let mut metadata = HashMap::new();
        metadata.insert("language".into(), self.language.clone());
        metadata.insert("fix_target".into(), file_path.to_string());

        AgentContext {
            prompt_template: "agent-fixer".into(),
            source_files: self.read_single_file(file_path),
            port_interfaces: self.port_files(),
            boundary_rules: self.rules_for_tier(tier),
            workplan_step: None,
            upstream_output: Some(issue_desc.to_string()),
            metadata,
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    /// Infer the hex tier from a file path based on directory conventions.
    fn infer_tier_from_path(file_path: &str) -> u32 {
        if file_path.contains("domain") || file_path.contains("ports") {
            0
        } else if file_path.contains("adapters/secondary") || file_path.contains("adapters\\secondary") {
            1
        } else if file_path.contains("adapters/primary") || file_path.contains("adapters\\primary") {
            2
        } else if file_path.contains("usecases") {
            3
        } else {
            4
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_infer_tier_from_path() {
        assert_eq!(Supervisor::infer_tier_from_path("src/core/domain/value-objects.ts"), 0);
        assert_eq!(Supervisor::infer_tier_from_path("src/core/ports/inference.ts"), 0);
        assert_eq!(Supervisor::infer_tier_from_path("src/adapters/secondary/fs.ts"), 1);
        assert_eq!(Supervisor::infer_tier_from_path("src/adapters/primary/cli.ts"), 2);
        assert_eq!(Supervisor::infer_tier_from_path("src/core/usecases/analyze.ts"), 3);
        assert_eq!(Supervisor::infer_tier_from_path("src/composition-root.ts"), 4);
    }

    #[test]
    fn test_rules_for_tier() {
        let sup = Supervisor::new("/tmp/test", "typescript");
        assert!(sup.rules_for_tier(0).contains("domain/"));
        assert!(sup.rules_for_tier(1).contains("Secondary"));
        assert!(sup.rules_for_tier(2).contains("Primary"));
        assert!(sup.rules_for_tier(3).contains("Use Cases"));
        assert!(sup.rules_for_tier(4).contains("Composition Root"));
    }

    #[test]
    fn test_build_coder_context() {
        let sup = Supervisor::new("/tmp/nonexistent", "typescript");
        let ctx = sup.build_coder_context("Implement port interface", 0);
        assert_eq!(ctx.prompt_template, "agent-coder");
        assert_eq!(ctx.workplan_step, Some("Implement port interface".into()));
        assert!(ctx.boundary_rules.contains("domain/"));
        assert_eq!(ctx.metadata.get("tier"), Some(&"0".to_string()));
    }

    #[test]
    fn test_build_fixer_context_has_upstream() {
        let sup = Supervisor::new("/tmp/nonexistent", "typescript");
        let ctx = sup.build_fixer_context("src/core/domain/entities.ts", "Missing export");
        assert_eq!(ctx.prompt_template, "agent-fixer");
        assert_eq!(ctx.upstream_output, Some("Missing export".into()));
    }
}
