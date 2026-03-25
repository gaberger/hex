//! Supervisor — context-aware agent orchestration for `hex dev` pipeline.
//!
//! The `Supervisor` assembles [`AgentContext`] structs tailored to each agent
//! role (coder, reviewer, tester, documenter, UX, fixer).  Context builders
//! read source files from disk, attach boundary rules, and include upstream
//! output so that inference calls carry exactly the information the agent needs.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context as _, Result};
use tracing::{debug, info, warn};

use crate::pipeline::agent_def::{AgentDefinition, SwarmComposition};
use crate::pipeline::agents::{DocumenterAgent, ReviewerAgent, TesterAgent, UxReviewerAgent};
use crate::pipeline::cli_runner::CliRunner;
use crate::pipeline::code_phase::CodePhase;
use crate::pipeline::fix_agent::{FixAgent, FixTaskInput};
use crate::pipeline::model_selection::ModelSelector;
use crate::pipeline::workflow_engine::WorkflowEngine;
use crate::pipeline::objectives::{
    agent_for_objective, evaluate_all, objectives_for_tier, Objective, ObjectiveState,
};
use crate::pipeline::workplan_phase::{WorkplanData, WorkplanStep};
use crate::session::{DevSession, ToolCall};

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
    /// HexFlo swarm ID for task tracking (best-effort).
    swarm_id: Option<String>,
    /// Agent identity for task assignment (best-effort).
    agent_id: Option<String>,
    /// Dev session for logging tool calls (shared, behind Mutex for interior mutability).
    session: Option<Arc<Mutex<DevSession>>>,
    /// Cached agent definitions loaded from YAML (ADR-2603240130).
    agent_defs: HashMap<String, AgentDefinition>,
    /// Cached swarm composition (loaded once, used for model defaults).
    swarm_comp: Option<SwarmComposition>,
    /// Spawned worker processes, behind Mutex for interior mutability (killed on drop).
    workers: Mutex<Vec<(String, std::process::Child)>>,
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

        // Load YAML definitions at construction (ADR-2603240130)
        let agent_defs = AgentDefinition::load_all();
        let swarm_comp = SwarmComposition::load("dev-pipeline");

        info!(
            agents = agent_defs.len(),
            swarm = swarm_comp.is_some(),
            "loaded YAML definitions"
        );

        Self {
            output_dir: output_dir.to_string(),
            language: language.to_string(),
            nexus_url,
            model_override,
            provider_pref,
            swarm_id: None,
            agent_id: None,
            session: None,
            agent_defs,
            swarm_comp,
            workers: Mutex::new(Vec::new()),
        }
    }

    /// Enable HexFlo task tracking for this supervisor.
    ///
    /// When set, `execute_agent()` will create a HexFlo task before each agent
    /// invocation and mark it completed afterward (best-effort — failures are
    /// logged but do not block the pipeline).
    pub fn with_tracking(
        mut self,
        swarm_id: impl Into<Option<String>>,
        agent_id: impl Into<Option<String>>,
    ) -> Self {
        self.swarm_id = swarm_id.into();
        self.agent_id = agent_id.into();
        self
    }

    /// Attach a dev session for per-role performance logging.
    ///
    /// Tool calls are appended to the session after each agent execution,
    /// recording model, tokens, cost, and duration per role.
    pub fn with_session(mut self, session: Arc<Mutex<DevSession>>) -> Self {
        self.session = Some(session);
        self
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
        let lang = self.language.as_str();

        // For Rust/Go single-binary projects, files live under src/ root
        // not in the hexagonal sub-directories.  Fall through to scan all of src/.
        let dirs: Vec<PathBuf> = match lang {
            "rust" | "go" => vec![base.clone()],
            _ => match tier {
                0 => vec![
                    base.join("core").join("domain"),
                    base.join("core").join("ports"),
                ],
                1 => vec![base.join("adapters").join("secondary")],
                2 => vec![base.join("adapters").join("primary")],
                3 => vec![base.join("core").join("usecases")],
                _ => vec![base],
            },
        };

        let mut files = Vec::new();
        for dir in dirs {
            Self::collect_files_for_language(&dir, &self.output_dir, Some(lang), &mut files);
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

    /// Read a single file from disk, truncating to `token_budget` bytes when provided,
    /// or falling back to [`MAX_FILE_BYTES`] (4096) when `None`.
    /// Returns `None` if the file cannot be read.
    fn read_file_truncated(path: &Path, token_budget: Option<usize>) -> Option<String> {
        let limit = token_budget.unwrap_or(MAX_FILE_BYTES);
        match fs::read_to_string(path) {
            Ok(content) => {
                if content.len() > limit {
                    debug!(
                        path = %path.display(),
                        "truncating file from {} to {} bytes",
                        content.len(),
                        limit,
                    );
                    let truncated = &content[..limit];
                    // Find last newline to avoid cutting mid-line
                    let end = truncated.rfind('\n').unwrap_or(limit);
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
        Self::collect_files_for_language(dir, base, None, out);
    }

    fn collect_files_for_language(
        dir: &Path,
        base: &str,
        language: Option<&str>,
        out: &mut Vec<(String, String)>,
    ) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                Self::collect_files_for_language(&path, base, language, out);
            } else if path.is_file() {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                // Filter extensions by language when known, to avoid picking up
                // stale files from a previous run with a different language.
                let allowed = match language {
                    Some("rust") => matches!(ext, "rs" | "toml"),
                    Some("go") => matches!(ext, "go"),
                    Some("typescript") | Some("javascript") => {
                        matches!(ext, "ts" | "js" | "tsx" | "jsx")
                    }
                    _ => matches!(ext, "ts" | "js" | "rs" | "tsx" | "jsx"),
                };
                if allowed {
                    if let Some(content) = Self::read_file_truncated(&path, None) {
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
        match Self::read_file_truncated(&path, None) {
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

    // ── YAML-driven context (ADR-2603240130) ──────────────────────────────

    /// Build agent context from a YAML agent definition's `load_strategy`.
    ///
    /// Resolves `{{target_adapter}}` and `{{current_edit_file}}` placeholders
    /// from the workplan step, then loads files at the specified level:
    /// - **L0**: file listing only (names, no content)
    /// - **L1**: AST summary (via `hex summarize` — token-efficient)
    /// - **L2**: function/method signatures only
    /// - **L3**: full file content
    ///
    /// Falls back to `build_coder_context` if the agent has no load_strategy.
    pub fn build_context_from_yaml(
        &self,
        agent_def: &crate::pipeline::agent_def::AgentDefinition,
        step_desc: &str,
        tier: u32,
        target_adapter: Option<&str>,
        current_edit_file: Option<&str>,
    ) -> AgentContext {
        let ctx_config = match &agent_def.context {
            Some(c) if !c.load_strategy.is_empty() => c,
            _ => return self.build_coder_context(step_desc, tier),
        };

        let mut source_files = Vec::new();
        let mut port_interfaces = Vec::new();
        let base = PathBuf::from(&self.output_dir);

        for entry in &ctx_config.load_strategy {
            // Skip on_demand entries during initial context build
            if entry.load.as_deref() == Some("on_demand") {
                continue;
            }

            // Resolve placeholders in scope
            let scope = entry
                .scope
                .replace("{{target_adapter}}", target_adapter.unwrap_or(""))
                .replace("{{current_edit_file}}", current_edit_file.unwrap_or(""));

            // Collect matching files
            let files = self.glob_files(&base, &scope);

            let yaml_budget: Option<usize> = ctx_config.token_budget.as_ref().map(|b| b.max as usize);

        for (rel_path, full_path) in &files {
                let content = match entry.level.as_str() {
                    "L0" => {
                        // File listing only — name, no content
                        format!("// {}", rel_path)
                    }
                    "L1" => {
                        // AST summary — read file, produce compact summary
                        // For now: first N lines as a practical approximation
                        // (full tree-sitter integration comes later via hex summarize)
                        Self::read_file_truncated(full_path, yaml_budget)
                            .map(|c| Self::summarize_l1(&c))
                            .unwrap_or_default()
                    }
                    "L2" => {
                        // Signatures only — extract function/struct/interface lines
                        Self::read_file_truncated(full_path, yaml_budget)
                            .map(|c| Self::extract_signatures(&c))
                            .unwrap_or_default()
                    }
                    "L3" | _ => {
                        // Full content
                        Self::read_file_truncated(full_path, yaml_budget).unwrap_or_default()
                    }
                };

                if !content.is_empty() {
                    // Route port files to port_interfaces, others to source_files
                    if rel_path.contains("ports") {
                        port_interfaces.push((rel_path.clone(), content));
                    } else {
                        source_files.push((rel_path.clone(), content));
                    }
                }
            }
        }

        let mut metadata = HashMap::new();
        metadata.insert("language".into(), self.language.clone());
        metadata.insert("tier".into(), tier.to_string());
        metadata.insert("context_source".into(), "yaml".into());
        if let Some(adapter) = target_adapter {
            metadata.insert("target_adapter".into(), adapter.to_string());
        }

        // Apply token budget if specified
        if let Some(ref budget) = ctx_config.token_budget {
            metadata.insert("token_budget_max".into(), budget.max.to_string());
        }

        AgentContext {
            prompt_template: format!("agent-{}", agent_def.agent_type),
            source_files,
            port_interfaces,
            boundary_rules: self.rules_for_tier(tier),
            workplan_step: Some(step_desc.to_string()),
            upstream_output: None,
            metadata,
        }
    }

    /// Glob-match files under `base` using a simplified glob pattern.
    /// Supports `**` (recursive) and `*` (single-level).
    fn glob_files(&self, base: &Path, pattern: &str) -> Vec<(String, PathBuf)> {
        let mut results = Vec::new();

        // Handle empty pattern
        if pattern.is_empty() {
            return results;
        }

        // Convert glob to a directory + extension filter
        let target = base.join(pattern.replace("/**", "").replace("/*", ""));
        if target.is_file() {
            let rel = target
                .strip_prefix(&self.output_dir)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| pattern.to_string());
            results.push((rel, target));
        } else if target.is_dir() {
            Self::collect_files_with_paths(&target, &self.output_dir, &mut results);
        }

        results
    }

    /// Like `collect_files` but returns (relative_path, full_path) without reading content.
    fn collect_files_with_paths(dir: &Path, base: &str, out: &mut Vec<(String, PathBuf)>) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                Self::collect_files_with_paths(&path, base, out);
            } else if path.is_file() {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if matches!(ext, "ts" | "js" | "rs" | "tsx" | "jsx") {
                    let rel = path
                        .strip_prefix(base)
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|_| path.display().to_string());
                    out.push((rel, path));
                }
            }
        }
    }

    /// L1 summary: extract exports, type definitions, and function signatures.
    /// Compact approximation until tree-sitter integration is wired in.
    fn summarize_l1(content: &str) -> String {
        content
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                trimmed.starts_with("export ")
                    || trimmed.starts_with("pub ")
                    || trimmed.starts_with("interface ")
                    || trimmed.starts_with("type ")
                    || trimmed.starts_with("struct ")
                    || trimmed.starts_with("enum ")
                    || trimmed.starts_with("trait ")
                    || trimmed.starts_with("fn ")
                    || trimmed.starts_with("class ")
                    || trimmed.starts_with("impl ")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// L2 extraction: function/method signatures (no bodies).
    fn extract_signatures(content: &str) -> String {
        let mut sigs = Vec::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("export function ")
                || trimmed.starts_with("export async function ")
                || trimmed.starts_with("export const ")
                || trimmed.starts_with("pub fn ")
                || trimmed.starts_with("pub async fn ")
                || trimmed.starts_with("fn ")
                || trimmed.starts_with("async fn ")
                || trimmed.contains("): ")  // TS method signature
                || trimmed.starts_with("interface ")
                || trimmed.starts_with("export interface ")
                || trimmed.starts_with("pub struct ")
                || trimmed.starts_with("pub enum ")
                || trimmed.starts_with("pub trait ")
            {
                sigs.push(trimmed.to_string());
            }
        }
        sigs.join("\n")
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

    // ── Worker process management ────────────────────────────────────────

    /// Determine the unique agent roles needed for a workplan.
    ///
    /// Loads the swarm composition from the embedded `dev-pipeline.yml` and
    /// evaluates each agent's `when` guard against workplan properties.
    /// See [`crate::pipeline::swarm_config::SwarmConfig`] (ADR-2603241230 step 8).
    fn roles_for_workplan(workplan: &WorkplanData) -> Vec<String> {
        let config = crate::pipeline::SwarmConfig::load_default();

        let has_primary_adapters = workplan.steps.iter().any(|s| {
            s.adapter
                .as_deref()
                .map(|a| a.contains("primary"))
                .unwrap_or(false)
                || s.layer
                    .as_deref()
                    .map(|l| l.contains("primary"))
                    .unwrap_or(false)
        });

        let max_tier = workplan.steps.iter().map(|s| s.tier).max().unwrap_or(0);
        let is_final_tier = max_tier > 0; // called once for the full run, so include final-tier agents

        config.roles_for_context(has_primary_adapters, is_final_tier, false)
    }

    /// Spawn `hex agent worker --role <role>` processes for each role.
    ///
    /// Workers register themselves with nexus and poll for task assignments.
    /// The supervisor assigns tasks by PATCHing task status via the REST API.
    fn spawn_workers(&self, roles: &[String]) -> Result<()> {
        let hex_bin = std::env::current_exe()
            .unwrap_or_else(|_| std::path::PathBuf::from("hex"));

        let mut workers = self.workers.lock().unwrap();
        for role in roles {
            let swarm_arg = self.swarm_id.as_deref().unwrap_or("");
            let mut cmd = std::process::Command::new(&hex_bin);
            cmd.args(["agent", "worker", "--role", role]);
            if !swarm_arg.is_empty() {
                cmd.args(["--swarm-id", swarm_arg]);
            }
            // Pass the supervisor's agent ID so the worker polls for tasks
            // assigned to this agent identity (supervisor assigns to itself).
            if let Some(ref aid) = self.agent_id {
                cmd.args(["--agent-id", aid]);
            }
            // Pipe worker stdout+stderr to a log file for diagnostics
            let log_path = format!("/tmp/hex-worker-{}.log", role);
            let log_file = std::fs::File::create(&log_path)
                .unwrap_or_else(|_| std::fs::File::open("/dev/null").unwrap());
            let log_file2 = log_file.try_clone()
                .unwrap_or_else(|_| std::fs::File::open("/dev/null").unwrap());
            cmd.stdout(log_file)
                .stderr(log_file2);

            let child = cmd.spawn().with_context(|| {
                format!("Failed to spawn worker for role {}", role)
            })?;

            println!("  Spawned {} worker (PID {})", role, child.id());
            workers.push((role.clone(), child));
        }
        Ok(())
    }

    /// Kill all spawned worker processes and wait for them to exit.
    fn kill_workers(&self) {
        let mut workers = self.workers.lock().unwrap();
        for (role, child) in workers.iter_mut() {
            let _ = child.kill();
            let _ = child.wait();
            debug!(role = role.as_str(), "killed worker");
        }
        workers.clear();
    }

    /// Check whether a worker is running for the given role.
    fn has_worker_for_role(&self, role: &str) -> bool {
        let workers = self.workers.lock().unwrap();
        workers.iter().any(|(r, _)| r == role)
    }

    // ── Goal-driven objective loop ──────────────────────────────────────

    /// Run all tiers in order, returning a [`SupervisorResult`] with per-tier outcomes.
    pub async fn run(
        &self,
        workplan: &WorkplanData,
        adr_content: &str,
    ) -> Result<SupervisorResult> {
        let workplan_summary = format!("{} — {}", workplan.id, workplan.title);

        // Spawn worker processes for each role needed by the workplan
        let roles = Self::roles_for_workplan(workplan);
        if let Err(e) = self.spawn_workers(&roles) {
            warn!(error = %e, "failed to spawn workers — falling back to inline execution");
        } else if !self.workers.lock().unwrap().is_empty() {
            println!("  Waiting for workers to register with nexus...");
            std::thread::sleep(std::time::Duration::from_secs(2));
        }

        // Scaffold src/ before the tier loop so CodeGenerated can pass on iteration 1.
        // generate_scaffold is idempotent — safe to call even if src/ already exists.
        if let Err(e) = crate::pipeline::code_phase::generate_scaffold(
            &self.output_dir,
            &self.language,
            &workplan.title,
        ) {
            warn!(error = %e, "scaffold generation failed — CodeGenerated may not pass");
        }

        // Clear stale review files from any previous pipeline run so that
        // evaluate_review_passes counts only reviews from THIS run.
        let review_dir = PathBuf::from(&self.output_dir).join(".hex-review");
        if review_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&review_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.extension().and_then(|e| e.to_str()) == Some("json") {
                        let _ = fs::remove_file(&p);
                    }
                }
            }
        }

        // Group steps by tier
        let max_tier = workplan.steps.iter().map(|s| s.tier as u32).max().unwrap_or(0);
        let mut tier_results: Vec<(u32, TierResult)> = Vec::new();

        for tier in 0..=max_tier {
            let steps: Vec<&WorkplanStep> = workplan
                .steps
                .iter()
                .filter(|s| s.tier as u32 == tier)
                .collect();

            if steps.is_empty() {
                continue;
            }

            let is_final_tier = tier == max_tier;
            let has_ui_adapters = steps.iter().any(|s| {
                s.adapter
                    .as_deref()
                    .map(|a| a.contains("primary"))
                    .unwrap_or(false)
                    || s.layer
                        .as_deref()
                        .map(|l| l.contains("primary"))
                        .unwrap_or(false)
            });

            println!("\n━━━ Tier {} ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━", tier);
            println!("  Steps: {}", steps.len());
            println!("  UI adapters: {}", if has_ui_adapters { "yes" } else { "no" });
            println!("  Final tier: {}", if is_final_tier { "yes" } else { "no" });

            let result = self
                .run_tier(
                    tier,
                    &steps,
                    has_ui_adapters,
                    is_final_tier,
                    adr_content,
                    &workplan_summary,
                )
                .await?;

            let passed = matches!(&result, TierResult::AllPassed { .. });
            tier_results.push((tier, result));

            if !passed {
                info!(tier, "tier did not fully pass — continuing to next tier");
            }
        }

        Ok(SupervisorResult { tier_results })
    }

    /// Run a single tier's objective loop: evaluate all objectives, fix unmet
    /// ones via the appropriate agent, re-evaluate, repeat until all pass or
    /// `MAX_ITERATIONS` is reached.
    pub async fn run_tier(
        &self,
        tier: u32,
        workplan_steps: &[&WorkplanStep],
        has_ui_adapters: bool,
        is_final_tier: bool,
        adr_content: &str,
        workplan_summary: &str,
    ) -> Result<TierResult> {
        const MAX_ITERATIONS: u32 = 5;

        let objectives = objectives_for_tier(tier, has_ui_adapters, is_final_tier);

        // Track which objectives have had a prior agent result (for fixer vs primary agent selection)
        let mut prior_results: HashMap<Objective, bool> = HashMap::new();

        for iteration in 1..=MAX_ITERATIONS {
            // Evaluate ALL objectives from scratch each iteration
            let states = evaluate_all(
                &objectives,
                &[],  // fresh evaluation each time
                tier,
                &self.output_dir,
                &self.language,
                &self.nexus_url,
            )
            .await;

            // Print progress
            print_iteration_progress(tier, iteration, &states);

            // Check if all objectives are met
            if states.iter().all(|s| s.met || s.skip_reason.is_some()) {
                println!(
                    "  [tier {}] iteration {}: all objectives met ✓",
                    tier, iteration
                );
                return Ok(TierResult::AllPassed {
                    iterations: iteration,
                    states,
                });
            }

            // Find first unmet objective (in priority order — objectives list is ordered)
            let unmet_state = states
                .iter()
                .find(|s| !s.met && s.skip_reason.is_none());

            let unmet_state = match unmet_state {
                Some(s) => s,
                None => {
                    // All are met or skipped — should have been caught above
                    return Ok(TierResult::AllPassed {
                        iterations: iteration,
                        states,
                    });
                }
            };

            let obj = unmet_state.objective;
            let has_prior = *prior_results.get(&obj).unwrap_or(&false);
            let agent_role = agent_for_objective(obj, has_prior);

            println!(
                "  → {}: addressing {} ...",
                agent_role, obj
            );

            // Build context and execute the appropriate agent.
            // Reviewer failures (timeouts, inference errors) are non-fatal — skip rather than abort.
            let agent_result = self.execute_agent_tracked(
                agent_role,
                tier,
                unmet_state,
                workplan_steps,
                adr_content,
                workplan_summary,
                iteration,
            )
            .await;

            if let Err(e) = agent_result {
                use crate::pipeline::objectives::Objective::*;
                if matches!(obj, ReviewPasses | UxReviewPasses) {
                    warn!(
                        error = %e,
                        objective = %obj,
                        "reviewer agent failed — skipping objective for this iteration"
                    );
                    prior_results.insert(obj, true);
                    continue;
                } else {
                    return Err(e).with_context(|| {
                        format!(
                            "agent {} failed for objective {} (tier {}, iteration {})",
                            agent_role, obj, tier, iteration
                        )
                    });
                }
            }

            // Mark that this objective now has a prior result
            prior_results.insert(obj, true);
        }

        // Final evaluation after max iterations
        let final_states = evaluate_all(
            &objectives,
            &[],
            tier,
            &self.output_dir,
            &self.language,
            &self.nexus_url,
        )
        .await;

        println!(
            "  [tier {}] max iterations ({}) reached",
            tier, MAX_ITERATIONS
        );
        print_iteration_progress(tier, MAX_ITERATIONS, &final_states);

        Ok(TierResult::MaxIterations {
            iterations: MAX_ITERATIONS,
            states: final_states,
        })
    }

    // ── HexFlo task tracking helpers ────────────────────────────────────

    /// Create a HexFlo task for the given role/objective (best-effort).
    /// Returns the task ID if successful.
    async fn create_tracking_task(
        &self,
        role: &str,
        objective: &Objective,
        iteration: u32,
    ) -> Option<String> {
        let swarm_id = self.swarm_id.as_ref()?;
        let runner = CliRunner::new();
        let title = format!("{}: {} [iteration {}]", role, objective, iteration);
        match runner.task_create(swarm_id, &title) {
            Ok(resp) => {
                let task_id = resp["id"].as_str().map(|s| s.to_string());
                if let Some(ref tid) = task_id {
                    // Assign to current agent if we have an agent_id
                    if let Some(ref aid) = self.agent_id {
                        let _ = runner.run(&["task", "assign", tid, aid]);
                    }
                    debug!(task_id = %tid, role, "created HexFlo tracking task");
                }
                task_id
            }
            Err(e) => {
                debug!(error = %e, role, "failed to create HexFlo tracking task (non-blocking)");
                None
            }
        }
    }

    /// Mark a HexFlo task as completed with a result summary (best-effort).
    async fn complete_tracking_task(&self, task_id: &str, result_summary: &str) {
        let runner = CliRunner::new();
        let truncated = &result_summary[..result_summary.len().min(200)];
        if let Err(e) = runner.task_complete(task_id, Some(truncated)) {
            debug!(error = %e, task_id, "failed to complete HexFlo tracking task (non-blocking)");
        }
    }

    /// Log a per-role ToolCall to the attached dev session (best-effort).
    fn log_agent_performance(
        &self,
        role: &str,
        model: Option<&str>,
        tokens: Option<u64>,
        cost_usd: Option<f64>,
        duration_ms: u64,
        success: bool,
        objective: &Objective,
    ) {
        if let Some(ref session_mutex) = self.session {
            if let Ok(mut session) = session_mutex.lock() {
                let call = ToolCall {
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    phase: format!("agent-{}", role),
                    tool: "POST /api/inference/complete".to_string(),
                    model: model.map(|s| s.to_string()),
                    tokens,
                    cost_usd,
                    duration_ms,
                    status: if success { "ok" } else { "error" }.to_string(),
                    detail: Some(format!("objective: {}", objective)),
                };
                if let Err(e) = session.log_tool_call(call) {
                    debug!(error = %e, role, "failed to log agent performance (non-blocking)");
                }
            }
        }
    }

    // ── Iteration counter for task titles ────────────────────────────────

    /// Get the current iteration count from the tier loop context.
    /// This is injected via `run_tier` into `execute_agent`.
    /// We thread it through via an extra parameter.

    /// Dispatch to the right agent with HexFlo tracking and performance logging.
    async fn execute_agent_tracked(
        &self,
        role: &str,
        tier: u32,
        state: &ObjectiveState,
        workplan_steps: &[&WorkplanStep],
        adr_content: &str,
        workplan_summary: &str,
        iteration: u32,
    ) -> Result<()> {
        // Create HexFlo tracking task (best-effort)
        let tracking_task_id = self
            .create_tracking_task(role, &state.objective, iteration)
            .await;

        let start = Instant::now();

        // Decide: delegate to worker process or execute inline.
        // Worker delegation is currently disabled — task_assign/task_status
        // plumbing is not fully wired end-to-end, so we always execute inline.
        let agent_result = if false && self.has_worker_for_role(role) {
            // ── Worker delegation path ──────────────────────────────────
            // Assign the task to the worker via nexus (worker picks it up
            // because it matches its agent_id + "in_progress" status).
            if let Some(ref tid) = tracking_task_id {
                let runner = CliRunner::new();
                // Mark task as assigned so the worker picks it up
                // (use run_raw — task assign has no --json output)
                let aid = self.agent_id.clone().unwrap_or_default();
                let _ = runner.run_raw(&["task", "assign", tid, &aid]);

                // Poll until the worker completes the task (max 120s)
                let poll_start = Instant::now();
                let timeout = std::time::Duration::from_secs(120);
                let mut retries = 0u32;
                let poll_result: Result<()> = loop {
                    if poll_start.elapsed() > timeout {
                        break Err(anyhow::anyhow!(
                            "Task {} timed out after 120s waiting for worker {}",
                            tid,
                            role
                        ));
                    }

                    // Check if worker is still alive
                    {
                        let mut workers = self.workers.lock().unwrap();
                        let worker_dead = workers
                            .iter_mut()
                            .find(|(r, _)| r == role)
                            .map(|(_, child)| {
                                child.try_wait().ok().flatten().is_some()
                            })
                            .unwrap_or(true);

                        if worker_dead {
                            warn!(
                                role,
                                task_id = ?tid,
                                "worker process died — respawning"
                            );

                            // Remove dead worker
                            workers.retain(|(r, _)| r != role);
                            drop(workers); // release lock before spawning

                            // Respawn
                            self.spawn_workers(&[role.to_string()])?;

                            // Reclaim the task back to assigned so new worker picks it up
                            let _ = runner.run_raw(&["task", "assign", tid, &aid]);

                            retries += 1;
                            if retries > 3 {
                                break Err(anyhow::anyhow!(
                                    "Worker for role {} died {} times — giving up",
                                    role,
                                    retries
                                ));
                            }

                            // Wait for new worker to register
                            std::thread::sleep(std::time::Duration::from_secs(3));
                            continue;
                        }
                    }

                    if let Ok(status) = runner.run(&["task", "status", tid]) {
                        let task_status = status["status"].as_str().unwrap_or("pending");
                        match task_status {
                            "completed" => break Ok(()),
                            "failed" => {
                                let reason =
                                    status["result"].as_str().unwrap_or("unknown error");
                                break Err(anyhow::anyhow!(
                                    "Task {} failed (worker {}): {}",
                                    tid,
                                    role,
                                    reason
                                ));
                            }
                            _ => {} // pending / in_progress — keep polling
                        }
                    }

                    std::thread::sleep(std::time::Duration::from_secs(2));
                };
                poll_result
            } else {
                // No tracking task ID — cannot delegate without a task, fall back to inline
                debug!(role, "no tracking task ID — falling back to inline dispatch");
                self.dispatch_agent(
                    role, tier, state, workplan_steps, adr_content, workplan_summary,
                )
                .await
            }
        } else {
            // ── Inline fallback (no workers running) ────────────────────
            self.dispatch_agent(
                role, tier, state, workplan_steps, adr_content, workplan_summary,
            )
            .await
        };

        let duration_ms = start.elapsed().as_millis() as u64;
        let success = agent_result.is_ok();

        // Extract performance metrics from agent results and log them.
        // Each agent type returns model_used/tokens/cost_usd — we capture
        // what we can from the dispatch. For roles that don't surface metrics
        // directly (hex-coder delegates to CodePhase), we log with None.
        //
        // The per-role ToolCall is logged regardless of success/failure.
        self.log_agent_performance(
            role,
            None, // model extracted below per-role where available
            None,
            None,
            duration_ms,
            success,
            &state.objective,
        );

        // Complete HexFlo tracking task (best-effort)
        if let Some(ref tid) = tracking_task_id {
            let summary = if success {
                format!("{}: completed in {}ms", role, duration_ms)
            } else {
                format!(
                    "{}: failed — {}",
                    role,
                    agent_result
                        .as_ref()
                        .err()
                        .map(|e| format!("{}", e))
                        .unwrap_or_default()
                )
            };
            self.complete_tracking_task(tid, &summary).await;
        }

        agent_result
    }

    /// Inner dispatch — executes the actual agent logic without tracking wrapper.
    async fn dispatch_agent(
        &self,
        role: &str,
        tier: u32,
        state: &ObjectiveState,
        workplan_steps: &[&WorkplanStep],
        _adr_content: &str,
        workplan_summary: &str,
    ) -> Result<()> {
        let model_override = self.model_override.as_deref();
        let provider_pref = self.provider_pref.as_deref();

        match role {
            "hex-coder" => {
                let phase = CodePhase::from_env();

                // Load YAML agent definition for phase-based workflow
                let agent_def = AgentDefinition::load("hex-coder");

                let use_workflow = agent_def
                    .as_ref()
                    .and_then(|d| d.workflow.as_ref())
                    .map(|w| w.is_phase_based())
                    .unwrap_or(false);

                if use_workflow {
                    let workflow = agent_def.as_ref().unwrap().workflow.as_ref().unwrap();

                    // Build engine with adapter vars from the first workplan step.
                    // For steps without an explicit adapter (e.g. domain/ports in Tier 0),
                    // derive a sensible default from the step's layer or description so
                    // that feedback gate templates like `{{adapter}}` resolve correctly.
                    let first_step = workplan_steps.first();
                    let first_adapter = first_step
                        .and_then(|s| s.adapter.as_deref());
                    let fallback_adapter: Option<String> = if first_adapter.is_none() {
                        first_step.map(|s| {
                            s.layer.as_deref().unwrap_or("core").to_string()
                        })
                    } else {
                        None
                    };
                    let adapter_ref = first_adapter
                        .or(fallback_adapter.as_deref());
                    let adapter_name = adapter_ref
                        .and_then(|a| a.rsplit('/').next());
                    let engine = self.workflow_engine_for_step(
                        adapter_ref,
                        adapter_name,
                    );

                    info!(
                        phases = workflow.phases.len(),
                        "running YAML workflow phases for hex-coder"
                    );

                    // Run pre_validate gate before code generation (ADR-2603240130 S01/S07)
                    // Skip for tier 0 (domain/ports have no adapter boundaries to violate)
                    if tier >= 1 {
                        if let Some(pre_validate_phase) = workflow.phases.iter().find(|p| p.id == "pre_validate") {
                            if let Some(ref gate) = pre_validate_phase.gate {
                                if gate.blocking {
                                    let gate_result = engine.run_phase_gate_pub(gate);
                                    if !gate_result.success {
                                        anyhow::bail!(
                                            "pre_validate gate '{}' failed for step — aborting code generation.\n\
                                             On-fail instructions: {}\n\
                                             Gate output: {}",
                                            gate.name,
                                            gate.on_fail.as_deref().unwrap_or("(none)"),
                                            gate_result.output
                                        );
                                    }
                                    info!(gate = %gate.name, "pre_validate gate passed ✓");
                                }
                            }
                        }
                    }

                    // Execute structured TDD phases (pre_validate → red → green → refactor → gate)
                    let phase_results = engine.execute_phases(workflow);
                    for pr in &phase_results {
                        if let Some(ref gf) = pr.gate_failure {
                            warn!(
                                phase = %pr.phase_id,
                                gate = %gf.gate_name,
                                "blocking gate recorded — supervisor will enforce"
                            );
                        }
                    }

                    // Select model from YAML definition (ADR-2603240130)
                    let yaml_selected = self.select_model_for_role("hex-coder", 0);
                    let yaml_model_id = yaml_selected.model_id.clone();
                    let effective_model: Option<&str> = model_override.or(Some(&yaml_model_id));
                    info!(
                        model = %yaml_model_id,
                        source = %yaml_selected.source,
                        "hex-coder model selected from YAML"
                    );

                    // Execute code generation for each workplan step
                    for step in workplan_steps {
                        // Build YAML-driven context for this step (ADR-2603240130 S03)
                        let target_adapter = step.adapter.as_deref();
                        let _yaml_ctx = self.build_context_from_yaml(
                            agent_def.as_ref().unwrap(),
                            &step.description,
                            tier,
                            target_adapter,
                            None,
                        );
                        debug!(
                            context_source = %_yaml_ctx.metadata.get("context_source").map(|s| s.as_str()).unwrap_or("fallback"),
                            source_files = _yaml_ctx.source_files.len(),
                            port_interfaces = _yaml_ctx.port_interfaces.len(),
                            "YAML context assembled for hex-coder step"
                        );

                        let step_workplan = WorkplanData {
                            id: "supervisor-tier".into(),
                            title: workplan_summary.to_string(),
                            specs: None,
                            adr: None,
                            created: None,
                            status: None,
                            status_note: None,
                            topology: None,
                            budget: None,
                            steps: vec![(*step).clone()],
                            merge_order: None,
                            risk_register: None,
                            success_criteria: None,
                            dependencies: None,
                        };

                        // Gate behind HEX_PHASE_MODE env var (default = "single" to avoid 3x cost)
                        let phase_mode = std::env::var("HEX_PHASE_MODE")
                            .unwrap_or_else(|_| "single".to_string());

                        let result = if phase_mode == "tdd" {
                            // Run red → green → refactor in sequence, passing accumulated output forward
                            let mut accumulated: Option<String> = None;
                            let mut last_result: Option<crate::pipeline::code_phase::CodeStepResult> = None;
                            for wf_phase in workflow.phases.iter().filter(|p| p.id != "pre_validate") {
                                match phase
                                    .execute_step_for_phase(
                                        step,
                                        wf_phase,
                                        &step_workplan,
                                        effective_model,
                                        provider_pref,
                                        accumulated.as_deref(),
                                    )
                                    .await
                                {
                                    Ok(r) => {
                                        accumulated = Some(r.content.clone());
                                        last_result = Some(r);
                                    }
                                    Err(e) => {
                                        warn!(error = %e, phase = %wf_phase.id, "phase inference failed — stopping phase loop");
                                        break;
                                    }
                                }
                            }
                            last_result.ok_or_else(|| anyhow::anyhow!("all TDD phases failed for step {}", step.id))?
                        } else {
                            // Default: single inference call (current behaviour, no 3x cost)
                            phase
                                .execute_step(step, &step_workplan, effective_model, provider_pref)
                                .await
                                .with_context(|| format!("code phase step {} failed", step.id))?
                        };

                        // Write generated code to disk
                        if let Some(ref rel_path) = result.file_path {
                            let full_path = PathBuf::from(&self.output_dir).join(rel_path);
                            if let Some(parent) = full_path.parent() {
                                fs::create_dir_all(parent)
                                    .with_context(|| format!("creating directory for {}", rel_path))?;
                            }
                            fs::write(&full_path, &result.content)
                                .with_context(|| format!("writing generated code to {}", rel_path))?;
                            info!(path = %rel_path, bytes = result.content.len(), "wrote generated code to disk");
                        }

                        // Evaluate quality thresholds from hex-coder YAML (ADR-2603240130 S06)
                        {
                            let file_lines = result.content.lines().count() as u32;
                            // Longest contiguous non-empty block as a proxy for max function lines
                            let max_fn_lines = {
                                let mut max_block = 0u32;
                                let mut cur_block = 0u32;
                                for line in result.content.lines() {
                                    if !line.trim().is_empty() {
                                        cur_block += 1;
                                        if cur_block > max_block {
                                            max_block = cur_block;
                                        }
                                    } else {
                                        cur_block = 0;
                                    }
                                }
                                max_block
                            };
                            let checks = self.evaluate_quality_thresholds(
                                "hex-coder",
                                0,
                                file_lines,
                                max_fn_lines,
                                0,
                            );
                            let mut blocking_violations: Vec<String> = Vec::new();
                            for check in &checks {
                                if !check.passed {
                                    // Blocking if max_file_lines or max_function_lines exceeded by >50%
                                    let is_blocking = (check.name == "max_file_lines"
                                        || check.name == "max_function_lines")
                                        && check.actual > check.threshold * 3 / 2;
                                    if is_blocking {
                                        warn!(
                                            threshold = %check.name,
                                            value = check.actual,
                                            limit = check.threshold,
                                            "blocking quality violation"
                                        );
                                        blocking_violations.push(format!(
                                            "Quality threshold '{}' exceeded: {} > {} (limit {}). \
                                             Exceeds 50% over limit — file must be split.",
                                            check.name, check.actual, check.threshold, check.threshold
                                        ));
                                    } else {
                                        info!(
                                            threshold = %check.name,
                                            value = check.actual,
                                            limit = check.threshold,
                                            "quality threshold warning"
                                        );
                                    }
                                }
                            }
                            if !blocking_violations.is_empty() {
                                let (target_file, fix_type) = self.infer_fix_target(
                                    tier,
                                    &state.objective,
                                    &blocking_violations,
                                );
                                let fix_input = FixTaskInput {
                                    fix_type,
                                    target_file,
                                    error_context: blocking_violations.join("\n\n"),
                                    language: self.language.clone(),
                                    output_dir: self.output_dir.clone(),
                                };
                                let fix_agent = FixAgent::from_env();
                                match fix_agent.execute(fix_input, effective_model, provider_pref).await {
                                    Ok(fix_result) => {
                                        info!(
                                            status = %fix_result.status,
                                            file = %fix_result.file_path,
                                            "quality threshold fixer complete"
                                        );
                                    }
                                    Err(e) => {
                                        warn!(
                                            error = %e,
                                            "quality threshold fixer failed — step marked as needing fix"
                                        );
                                    }
                                }
                            }
                        }
                    }

                    // Run feedback loop if defined (compile → lint → test gates)
                    if let Some(ref fl) = workflow.feedback_loop {
                        info!(
                            max_iterations = fl.max_iterations,
                            gates = fl.gates.len(),
                            "running feedback loop"
                        );
                        let (iterations, escalated, escalation_msg) =
                            engine.run_feedback_loop(fl);
                        let total_iterations = iterations.len();
                        let all_passed = iterations
                            .last()
                            .map(|last| last.iter().all(|g| g.success))
                            .unwrap_or(false);
                        info!(
                            iterations = total_iterations,
                            all_passed,
                            "feedback loop complete"
                        );
                        if !all_passed {
                            // Gates failed — invoke FixAgent on gate errors, then retry once
                            let gate_errors: Vec<String> = iterations
                                .last()
                                .map(|last| {
                                    last.iter()
                                        .filter(|g| !g.success)
                                        .map(|g| {
                                            format!("Gate '{}' failed:\n{}", g.gate_name, g.output)
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();

                            if !gate_errors.is_empty() {
                                let (target_file, fix_type) = self.infer_fix_target(
                                    tier,
                                    &state.objective,
                                    &gate_errors,
                                );
                                let fix_input = FixTaskInput {
                                    fix_type,
                                    target_file,
                                    error_context: gate_errors.join("\n\n"),
                                    language: self.language.clone(),
                                    output_dir: self.output_dir.clone(),
                                };
                                let fix_agent = FixAgent::from_env();
                                match fix_agent.execute(fix_input, effective_model, provider_pref).await {
                                    Ok(fix_result) => {
                                        info!(
                                            status = %fix_result.status,
                                            file = %fix_result.file_path,
                                            "gate fixer complete — retrying gates"
                                        );
                                        // One retry pass after the fix
                                        let (retry_iters, _, _) = engine.run_feedback_loop(fl);
                                        let retry_passed = retry_iters
                                            .last()
                                            .map(|last| last.iter().all(|g| g.success))
                                            .unwrap_or(false);
                                        info!(retry_passed, "gate retry after fix complete");
                                    }
                                    Err(e) => {
                                        warn!(error = %e, "gate fixer failed — continuing");
                                    }
                                }
                            }
                        }
                        if escalated {
                            warn!(
                                "feedback loop escalated after {} iterations: {:?}",
                                total_iterations,
                                escalation_msg
                            );
                        }
                    }
                } else {
                    // Fallback: direct CodePhase execution (no YAML workflow)
                    // Still use YAML model selection (ADR-2603240130)
                    let yaml_selected = self.select_model_for_role("hex-coder", 0);
                    let yaml_model_id = yaml_selected.model_id.clone();
                    let effective_model: Option<&str> = model_override.or(Some(&yaml_model_id));
                    info!(
                        model = %yaml_model_id,
                        source = %yaml_selected.source,
                        "hex-coder model selected from YAML (fallback path)"
                    );
                    for step in workplan_steps {
                        // Build YAML-driven context for this step (ADR-2603240130 S03)
                        let target_adapter = step.adapter.as_deref();
                        let _yaml_ctx = self.build_context_from_yaml(
                            agent_def.as_ref().unwrap(),
                            &step.description,
                            tier,
                            target_adapter,
                            None,
                        );
                        debug!(
                            context_source = %_yaml_ctx.metadata.get("context_source").map(|s| s.as_str()).unwrap_or("fallback"),
                            source_files = _yaml_ctx.source_files.len(),
                            port_interfaces = _yaml_ctx.port_interfaces.len(),
                            "YAML context assembled for hex-coder step"
                        );

                        let step_workplan = WorkplanData {
                            id: "supervisor-tier".into(),
                            title: workplan_summary.to_string(),
                            specs: None,
                            adr: None,
                            created: None,
                            status: None,
                            status_note: None,
                            topology: None,
                            budget: None,
                            steps: vec![(*step).clone()],
                            merge_order: None,
                            risk_register: None,
                            success_criteria: None,
                            dependencies: None,
                        };
                        let result = phase
                            .execute_step(step, &step_workplan, effective_model, provider_pref)
                            .await
                            .with_context(|| format!("code phase step {} failed", step.id))?;

                        // Write generated code to disk
                        if let Some(ref rel_path) = result.file_path {
                            let full_path = PathBuf::from(&self.output_dir).join(rel_path);
                            if let Some(parent) = full_path.parent() {
                                fs::create_dir_all(parent)
                                    .with_context(|| format!("creating directory for {}", rel_path))?;
                            }
                            fs::write(&full_path, &result.content)
                                .with_context(|| format!("writing generated code to {}", rel_path))?;
                            info!(path = %rel_path, bytes = result.content.len(), "wrote generated code to disk");
                        }

                        // Evaluate quality thresholds from hex-coder YAML (ADR-2603240130 S06)
                        {
                            let file_lines = result.content.lines().count() as u32;
                            let max_fn_lines = {
                                let mut max_block = 0u32;
                                let mut cur_block = 0u32;
                                for line in result.content.lines() {
                                    if !line.trim().is_empty() {
                                        cur_block += 1;
                                        if cur_block > max_block {
                                            max_block = cur_block;
                                        }
                                    } else {
                                        cur_block = 0;
                                    }
                                }
                                max_block
                            };
                            let checks = self.evaluate_quality_thresholds(
                                "hex-coder",
                                0,
                                file_lines,
                                max_fn_lines,
                                0,
                            );
                            let mut blocking_violations: Vec<String> = Vec::new();
                            for check in &checks {
                                if !check.passed {
                                    let is_blocking = (check.name == "max_file_lines"
                                        || check.name == "max_function_lines")
                                        && check.actual > check.threshold * 3 / 2;
                                    if is_blocking {
                                        warn!(
                                            threshold = %check.name,
                                            value = check.actual,
                                            limit = check.threshold,
                                            "blocking quality violation"
                                        );
                                        blocking_violations.push(format!(
                                            "Quality threshold '{}' exceeded: {} > {} (limit {}). \
                                             Exceeds 50% over limit — file must be split.",
                                            check.name, check.actual, check.threshold, check.threshold
                                        ));
                                    } else {
                                        info!(
                                            threshold = %check.name,
                                            value = check.actual,
                                            limit = check.threshold,
                                            "quality threshold warning"
                                        );
                                    }
                                }
                            }
                            if !blocking_violations.is_empty() {
                                let (target_file, fix_type) = self.infer_fix_target(
                                    tier,
                                    &state.objective,
                                    &blocking_violations,
                                );
                                let fix_input = FixTaskInput {
                                    fix_type,
                                    target_file,
                                    error_context: blocking_violations.join("\n\n"),
                                    language: self.language.clone(),
                                    output_dir: self.output_dir.clone(),
                                };
                                let fix_agent = FixAgent::from_env();
                                match fix_agent.execute(fix_input, effective_model, provider_pref).await {
                                    Ok(fix_result) => {
                                        info!(
                                            status = %fix_result.status,
                                            file = %fix_result.file_path,
                                            "quality threshold fixer complete"
                                        );
                                    }
                                    Err(e) => {
                                        warn!(
                                            error = %e,
                                            "quality threshold fixer failed — step marked as needing fix"
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "hex-reviewer" => {
                let agent = ReviewerAgent::from_env();
                let target_file = self.first_source_file_for_tier(tier);
                let context = self.build_reviewer_context(&target_file);
                let reviewer_selected = self.select_model_for_role("hex-reviewer", 0);
                let reviewer_model_id = reviewer_selected.model_id.clone();
                let reviewer_model: Option<&str> = model_override.or(Some(&reviewer_model_id));
                let result = agent
                    .execute(&context, reviewer_model, provider_pref)
                    .await
                    .context("reviewer agent failed")?;
                // Log per-role performance with actual metrics
                self.log_agent_performance(
                    "hex-reviewer",
                    Some(&result.model_used),
                    Some(result.tokens),
                    Some(result.cost_usd),
                    result.duration_ms,
                    true,
                    &state.objective,
                );
                // Write review output so evaluate_review_passes can pick it up
                let review_dir = PathBuf::from(&self.output_dir).join(".hex-review");
                let _ = fs::create_dir_all(&review_dir);
                let review_json = serde_json::json!({
                    "verdict": result.verdict,
                    "issues": result.issues.iter().map(|i| serde_json::json!({
                        "severity": i.severity,
                        "message": i.description,
                        "location": i.location,
                        "recommendation": i.recommendation,
                    })).collect::<Vec<_>>(),
                });
                // Always write to the same file so evaluate_review_passes sees only the
                // most-recent review verdict (not an accumulation across all tiers).
                let review_path = review_dir.join("review-latest.json");
                fs::write(&review_path, serde_json::to_string_pretty(&review_json)?)
                    .with_context(|| format!("writing review to {}", review_path.display()))?;
                info!(verdict = %result.verdict, issues = result.issues.len(), "review complete");
            }
            "hex-tester" => {
                let agent = TesterAgent::from_env();
                let target_file = self.first_source_file_for_tier(tier);
                let context = self.build_tester_context(&target_file);
                let tester_selected = self.select_model_for_role("hex-tester", 0);
                let tester_model_id = tester_selected.model_id.clone();
                let tester_model: Option<&str> = model_override.or(Some(&tester_model_id));
                let result = agent
                    .execute(&context, tester_model, provider_pref)
                    .await
                    .context("tester agent failed")?;
                // Log per-role performance with actual metrics
                self.log_agent_performance(
                    "hex-tester",
                    Some(&result.model_used),
                    Some(result.tokens),
                    Some(result.cost_usd),
                    result.duration_ms,
                    true,
                    &state.objective,
                );
                // Write test file to the suggested path
                let test_path = PathBuf::from(&self.output_dir).join(&result.suggested_path);
                if let Some(parent) = test_path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                fs::write(&test_path, &result.test_content)
                    .with_context(|| format!("writing test to {}", test_path.display()))?;
                info!(path = %result.suggested_path, "test file written");
            }
            "hex-documenter" => {
                let agent = DocumenterAgent::from_env();
                let context = self.build_documenter_context(_adr_content, workplan_summary);
                let result = agent
                    .execute(&context, &self.output_dir, model_override, provider_pref)
                    .await
                    .context("documenter agent failed")?;
                // Log per-role performance with actual metrics
                self.log_agent_performance(
                    "hex-documenter",
                    Some(&result.model_used),
                    Some(result.tokens),
                    Some(result.cost_usd),
                    result.duration_ms,
                    true,
                    &state.objective,
                );
                info!("documentation generated");
            }
            "hex-ux" => {
                let agent = UxReviewerAgent::from_env();
                let target_file = self.first_source_file_for_tier(tier);
                let context = self.build_ux_context(&target_file, workplan_summary);
                let result = agent
                    .execute(&context, &self.output_dir, model_override, provider_pref)
                    .await
                    .context("ux reviewer agent failed")?;
                // Log per-role performance with actual metrics
                self.log_agent_performance(
                    "hex-ux",
                    Some(&result.model_used),
                    Some(result.tokens),
                    Some(result.cost_usd),
                    result.duration_ms,
                    true,
                    &state.objective,
                );
                // Write UX review output
                let ux_dir = PathBuf::from(&self.output_dir).join(".hex-ux-review");
                let _ = fs::create_dir_all(&ux_dir);
                let ux_json = serde_json::json!({
                    "verdict": result.verdict,
                    "issues": result.issues.iter().map(|i| serde_json::json!({
                        "severity": i.severity,
                        "message": i.description,
                        "recommendation": i.recommendation,
                        "user_impact": i.user_impact,
                    })).collect::<Vec<_>>(),
                });
                let ux_path = ux_dir.join(format!("ux-review-tier{}.json", tier));
                fs::write(&ux_path, serde_json::to_string_pretty(&ux_json)?)
                    .with_context(|| format!("writing ux review to {}", ux_path.display()))?;
                info!(verdict = %result.verdict, issues = result.issues.len(), "ux review complete");
            }
            "hex-fixer" => {
                // For Rust compile errors: auto-add missing crates to Cargo.toml
                // before calling the expensive LLM fixer. This handles the common
                // "use of undeclared crate" error caused by generated code importing
                // crates not yet listed in Cargo.toml.
                if self.language == "rust" && state.objective == Objective::CodeCompiles {
                    self.auto_patch_cargo_toml(&state.blocking_issues);
                }

                let agent = FixAgent::from_env();
                let (target_file, fix_type) =
                    self.infer_fix_target(tier, &state.objective, &state.blocking_issues);
                let input = FixTaskInput {
                    fix_type,
                    target_file,
                    error_context: state.blocking_issues.join("\n"),
                    language: self.language.clone(),
                    output_dir: self.output_dir.clone(),
                };
                let result = agent
                    .execute(input, model_override, provider_pref)
                    .await
                    .context("fix agent failed")?;
                // Log per-role performance with actual metrics (FixTaskOutput has no duration_ms)
                self.log_agent_performance(
                    "hex-fixer",
                    Some(&result.model_used),
                    Some(result.tokens),
                    Some(result.cost_usd),
                    0, // duration tracked by outer wrapper
                    result.status == "fixed",
                    &state.objective,
                );
                info!(
                    status = %result.status,
                    file = %result.file_path,
                    "fix agent complete"
                );
            }
            other => {
                warn!(role = other, "unknown agent role — skipping");
            }
        }
        Ok(())
    }

    /// Find the first source file path for a given tier (for reviewer/tester targeting).
    fn first_source_file_for_tier(&self, tier: u32) -> String {
        let files = self.files_for_tier(tier);
        files
            .first()
            .map(|(path, _)| path.clone())
            .unwrap_or_else(|| {
                let ext = match self.language.as_str() {
                    "rust" => "rs",
                    "go" => "go",
                    _ => "ts",
                };
                format!("src/unknown-tier{}.{}", tier, ext)
            })
    }

    /// Infer fix target file and fix_type from the objective and blocking issues.
    fn infer_fix_target(
        &self,
        tier: u32,
        objective: &Objective,
        blocking_issues: &[String],
    ) -> (String, String) {
        // Try to extract a file path from the first blocking issue
        let target_file = blocking_issues
            .first()
            .and_then(|issue| {
                // Look for patterns like "path/to/file.ts:42:" or "path/to/file.ts: message"
                let parts: Vec<&str> = issue.splitn(2, ':').collect();
                let candidate = parts.first().unwrap_or(&"").trim();
                if candidate.contains('.') && (candidate.contains('/') || candidate.contains('\\'))
                {
                    // Looks like a file path — resolve to an absolute path so that
                    // the second path join below (full_path) doesn't double-prepend
                    // output_dir when output_dir is itself a relative path.
                    let abs_output = std::env::current_dir()
                        .unwrap_or_else(|_| PathBuf::from("."))
                        .join(&self.output_dir);
                    let full_path = abs_output.join(candidate);
                    if full_path.exists() {
                        Some(full_path.display().to_string())
                    } else {
                        // File doesn't exist yet — return relative so fixer can create it
                        Some(candidate.to_string())
                    }
                } else {
                    None
                }
            })
            .unwrap_or_else(|| self.first_source_file_for_tier(tier));

        let fix_type = match objective {
            Objective::CodeCompiles => "compile".to_string(),
            Objective::TestsPass => "test".to_string(),
            Objective::ArchitectureGradeA => "violation".to_string(),
            Objective::ReviewPasses | Objective::UxReviewPasses => "violation".to_string(),
            _ => "compile".to_string(),
        };

        let full_path = if Path::new(&target_file).is_absolute() {
            target_file
        } else {
            PathBuf::from(&self.output_dir)
                .join(&target_file)
                .display()
                .to_string()
        };

        (full_path, fix_type)
    }

    /// Auto-patch `Cargo.toml` by adding missing crate dependencies inferred from
    /// compile error messages (e.g. "use of undeclared crate or module `clap`").
    /// This avoids expensive LLM calls for the common "missing dependency" error.
    fn auto_patch_cargo_toml(&self, blocking_issues: &[String]) {
        // Known crates and their Cargo.toml entries
        let known_crates: &[(&str, &str)] = &[
            ("clap", r#"clap = { version = "4", features = ["derive"] }"#),
            ("serde", r#"serde = { version = "1", features = ["derive"] }"#),
            ("serde_json", r#"serde_json = "1""#),
            ("anyhow", r#"anyhow = "1""#),
            ("tokio", r#"tokio = { version = "1", features = ["full"] }"#),
            ("thiserror", r#"thiserror = "1""#),
            ("tracing", r#"tracing = "0.1""#),
            ("regex", r#"regex = "1""#),
            ("chrono", r#"chrono = "0.4""#),
            ("uuid", r#"uuid = { version = "1", features = ["v4"] }"#),
        ];

        let cargo_path = PathBuf::from(&self.output_dir).join("Cargo.toml");
        if !cargo_path.exists() {
            return;
        }

        let cargo_content = match fs::read_to_string(&cargo_path) {
            Ok(c) => c,
            Err(_) => return,
        };

        let mut additions: Vec<&str> = Vec::new();
        let combined_errors = blocking_issues.join("\n");

        for (crate_name, dep_line) in known_crates {
            // Check if the error mentions this crate and it's not yet in Cargo.toml
            let mentioned = combined_errors.contains(&format!("crate or module `{}`", crate_name))
                || combined_errors.contains(&format!("use of undeclared crate `{}`", crate_name))
                || combined_errors.contains(&format!("`{}` is not", crate_name));
            if mentioned && !cargo_content.contains(crate_name) {
                additions.push(dep_line);
            }
        }

        if additions.is_empty() {
            return;
        }

        // Insert additions before the end of [dependencies] section
        let patched = if cargo_content.contains("[dependencies]") {
            format!("{}\n{}\n", cargo_content.trim_end(), additions.join("\n"))
        } else {
            format!("{}\n[dependencies]\n{}\n", cargo_content.trim_end(), additions.join("\n"))
        };

        if let Err(e) = fs::write(&cargo_path, patched) {
            warn!(error = %e, "failed to auto-patch Cargo.toml");
        } else {
            info!(added = additions.len(), "auto-patched Cargo.toml with missing dependencies");
        }
    }

    // ── YAML-driven dispatch (ADR-2603240130 steps 8-9) ─────────────────

    /// Get the YAML agent definition for a role, if available.
    pub fn agent_def(&self, role: &str) -> Option<&AgentDefinition> {
        self.agent_defs.get(role)
    }

    /// Select a model for an agent role using YAML configuration.
    ///
    /// Falls back to the hardcoded path if the agent has no YAML definition.
    pub fn select_model_for_role(
        &self,
        role: &str,
        iteration: u32,
    ) -> crate::pipeline::model_selection::SelectedModel {
        let selector = ModelSelector::from_env();

        // User override always wins
        if let Some(ref model) = self.model_override {
            return crate::pipeline::model_selection::SelectedModel {
                model_id: model.clone(),
                state_key: None,
                action: None,
                source: crate::pipeline::model_selection::SelectionSource::UserOverride,
            };
        }

        // Try YAML-driven selection
        if let Some(def) = self.agent_defs.get(role) {
            let swarm_default = self.swarm_model_default_for_role(role);
            return selector.select_from_yaml(
                &def.model,
                None,
                iteration,
                3, // default upgrade threshold
                swarm_default.as_deref(),
            );
        }

        // Fallback: default model
        crate::pipeline::model_selection::SelectedModel {
            model_id: "openrouter/free".to_string(),
            state_key: None,
            action: None,
            source: crate::pipeline::model_selection::SelectionSource::Default,
        }
    }

    /// Look up model default from swarm composition for a role's task type.
    fn swarm_model_default_for_role(&self, role: &str) -> Option<String> {
        let comp = self.swarm_comp.as_ref()?;
        let entry = comp.agents.iter().find(|a| a.role == role)?;
        let task_type = entry.inference.as_ref()?.task_type.as_ref()?;
        comp.model_defaults.as_ref()?.get(task_type.as_str()).cloned()
    }

    /// Create a workflow engine configured for the current output dir and language.
    pub fn workflow_engine(&self) -> WorkflowEngine {
        WorkflowEngine::new(&self.output_dir, &self.language)
    }

    /// Create a workflow engine with adapter-specific variables for a workplan step.
    pub fn workflow_engine_for_step(
        &self,
        adapter: Option<&str>,
        adapter_name: Option<&str>,
    ) -> WorkflowEngine {
        let mut engine = WorkflowEngine::new(&self.output_dir, &self.language);
        if let Some(a) = adapter {
            engine = engine.with_var("adapter", a);
        }
        if let Some(n) = adapter_name {
            engine = engine.with_var("adapter_name", n);
        }
        engine
    }

    /// Evaluate quality thresholds from a YAML agent definition.
    pub fn evaluate_quality_thresholds(
        &self,
        role: &str,
        lint_warnings: u32,
        file_lines: u32,
        function_lines: u32,
        test_coverage: u32,
    ) -> Vec<QualityCheck> {
        let thresholds = match self.agent_defs.get(role) {
            Some(def) => match &def.quality_thresholds {
                Some(qt) => qt,
                None => return Vec::new(),
            },
            None => return Vec::new(),
        };

        let mut checks = Vec::new();

        if let Some(max) = thresholds.max_lint_warnings {
            checks.push(QualityCheck {
                name: "max_lint_warnings".into(),
                passed: lint_warnings <= max,
                actual: lint_warnings,
                threshold: max,
            });
        }
        if let Some(max) = thresholds.max_file_lines {
            checks.push(QualityCheck {
                name: "max_file_lines".into(),
                passed: file_lines <= max,
                actual: file_lines,
                threshold: max,
            });
        }
        if let Some(max) = thresholds.max_function_lines {
            checks.push(QualityCheck {
                name: "max_function_lines".into(),
                passed: function_lines <= max,
                actual: function_lines,
                threshold: max,
            });
        }
        if let Some(min) = thresholds.test_coverage {
            checks.push(QualityCheck {
                name: "test_coverage".into(),
                passed: test_coverage >= min,
                actual: test_coverage,
                threshold: min,
            });
        }
        if let Some(max) = thresholds.max_cyclomatic_complexity {
            checks.push(QualityCheck {
                name: "max_cyclomatic_complexity".into(),
                passed: true, // not yet measured by hex analyze
                actual: 0,
                threshold: max,
            });
        }

        checks
    }

    /// Check if all quality thresholds pass for a given role.
    pub fn quality_thresholds_pass(
        &self,
        role: &str,
        lint_warnings: u32,
        file_lines: u32,
        function_lines: u32,
        test_coverage: u32,
    ) -> bool {
        self.evaluate_quality_thresholds(role, lint_warnings, file_lines, function_lines, test_coverage)
            .iter()
            .all(|c| c.passed)
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.kill_workers();
    }
}

// ── Result types ────────────────────────────────────────────────────────

/// Outcome of running a single tier's objective loop.
#[derive(Debug, Clone)]
pub enum TierResult {
    /// All objectives were met within the iteration budget.
    AllPassed {
        iterations: u32,
        states: Vec<ObjectiveState>,
    },
    /// Max iterations reached with some objectives still unmet.
    MaxIterations {
        iterations: u32,
        states: Vec<ObjectiveState>,
    },
}

impl TierResult {
    /// Whether all objectives passed.
    pub fn passed(&self) -> bool {
        matches!(self, TierResult::AllPassed { .. })
    }

    /// The final objective states.
    pub fn states(&self) -> &[ObjectiveState] {
        match self {
            TierResult::AllPassed { states, .. } => states,
            TierResult::MaxIterations { states, .. } => states,
        }
    }

    /// Number of iterations used.
    pub fn iterations(&self) -> u32 {
        match self {
            TierResult::AllPassed { iterations, .. } => *iterations,
            TierResult::MaxIterations { iterations, .. } => *iterations,
        }
    }
}

/// Outcome of running all tiers.
#[derive(Debug)]
pub struct SupervisorResult {
    pub tier_results: Vec<(u32, TierResult)>,
}

impl SupervisorResult {
    /// True if every tier passed all objectives.
    pub fn all_passed(&self) -> bool {
        self.tier_results.iter().all(|(_, r)| r.passed())
    }

    /// Total iterations across all tiers.
    pub fn total_iterations(&self) -> u32 {
        self.tier_results.iter().map(|(_, r)| r.iterations()).sum()
    }

    /// Build a QualityReport from the supervisor's objective states.
    pub fn to_quality_report(&self, language: &str) -> crate::session::QualityReport {
        use crate::pipeline::objectives::Objective;

        // Collect all final objective states across tiers
        let all_states: Vec<&crate::pipeline::objectives::ObjectiveState> =
            self.tier_results.iter().flat_map(|(_, r)| r.states()).collect();

        let compile_state = all_states.iter().find(|s| matches!(s.objective, Objective::CodeCompiles));
        let test_state = all_states.iter().find(|s| matches!(s.objective, Objective::TestsPass));
        let arch_state = all_states.iter().find(|s| matches!(s.objective, Objective::ArchitectureGradeA));

        let compile_pass = compile_state.map(|s| s.met).unwrap_or(false);
        let test_pass = test_state.map(|s| s.met).unwrap_or(false);

        // Parse test counts from detail string (e.g. "3/5 tests passing")
        let (tests_passed, tests_failed) = test_state
            .and_then(|s| {
                let parts: Vec<&str> = s.detail.split('/').collect();
                if parts.len() >= 2 {
                    let passed = parts[0].trim().parse::<u32>().ok()?;
                    let total_str = parts[1].split_whitespace().next()?;
                    let total = total_str.parse::<u32>().ok()?;
                    Some((passed, total.saturating_sub(passed)))
                } else {
                    None
                }
            })
            .unwrap_or((0, 0));

        // Parse violation count from architecture detail (e.g. "Score 87/100" or "2 violations")
        let violations_found = arch_state
            .and_then(|s| {
                if s.met { return Some(0); }
                // Try to extract number from blocking_issues count
                Some(s.blocking_issues.len() as u32)
            })
            .unwrap_or(0);

        // Compute overall score: each objective met = 12.5 points (8 objectives)
        let met_count = all_states.iter().filter(|s| s.met).count() as u32;
        let total_count = all_states.len().max(1) as u32;
        let score = (met_count * 100) / total_count;

        let grade = match score {
            90..=100 => "A",
            80..=89 => "B",
            70..=79 => "C",
            60..=69 => "D",
            _ => "F",
        }
        .to_string();

        // Load quality thresholds from agent YAML to include in report
        let thresholds = crate::pipeline::objectives::load_quality_thresholds("hex-fixer");
        let mut thresholds_checked = Vec::new();
        if let Some(v) = thresholds.max_lint_warnings {
            thresholds_checked.push(format!("max_lint_warnings={}", v));
        }
        if let Some(v) = thresholds.max_file_lines {
            thresholds_checked.push(format!("max_file_lines={}", v));
        }
        if let Some(v) = thresholds.max_function_lines {
            thresholds_checked.push(format!("max_function_lines={}", v));
        }
        if let Some(v) = thresholds.max_cyclomatic_complexity {
            thresholds_checked.push(format!("max_cyclomatic_complexity={}", v));
        }
        if let Some(v) = thresholds.test_coverage {
            thresholds_checked.push(format!("test_coverage={}%", v));
        }

        crate::session::QualityReport {
            grade,
            score,
            iterations: self.total_iterations(),
            compile_pass,
            compile_language: language.to_string(),
            test_pass,
            tests_passed,
            tests_failed,
            violations_found,
            violations_fixed: 0,
            fix_cost_usd: 0.0,
            fix_tokens: 0,
            quality_thresholds_checked: thresholds_checked,
        }
    }

    /// Summary string suitable for printing.
    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        for (tier, result) in &self.tier_results {
            let status = if result.passed() { "PASS" } else { "INCOMPLETE" };
            let met = result
                .states()
                .iter()
                .filter(|s| s.met || s.skip_reason.is_some())
                .count();
            let total = result.states().len();
            lines.push(format!(
                "  Tier {}: {} ({}/{} objectives, {} iterations)",
                tier,
                status,
                met,
                total,
                result.iterations()
            ));
        }
        lines.join("\n")
    }
}

/// Result of a single quality threshold check.
#[derive(Debug, Clone)]
pub struct QualityCheck {
    pub name: String,
    pub passed: bool,
    pub actual: u32,
    pub threshold: u32,
}

// ── Progress printing ───────────────────────────────────────────────────

/// Print a single iteration's objective status line.
fn print_iteration_progress(tier: u32, iteration: u32, states: &[ObjectiveState]) {
    let parts: Vec<String> = states
        .iter()
        .map(|s| {
            if s.skip_reason.is_some() {
                format!("{} ⊘", s.objective)
            } else if s.met {
                format!("{} ✓", s.objective)
            } else {
                let detail = if s.detail.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", s.detail)
                };
                format!("{} ✗{}", s.objective, detail)
            }
        })
        .collect();
    println!("  [tier {}] iteration {}: {}", tier, iteration, parts.join(", "));
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

    /// S04: agent with token_budget.max=2000 truncates files at 2000 bytes.
    #[test]
    fn test_read_file_truncated_uses_token_budget() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        // Create a temp file with 3000 bytes of content
        let mut tmp = NamedTempFile::new().expect("tempfile");
        let content = "x".repeat(3000);
        tmp.write_all(content.as_bytes()).expect("write");
        tmp.flush().expect("flush");

        // With budget=2000, output should be truncated
        let result = Supervisor::read_file_truncated(tmp.path(), Some(2000));
        assert!(result.is_some(), "should return Some");
        let s = result.unwrap();
        // The truncated string plus the trailing comment must be <= 2000 chars of original content
        assert!(s.contains("truncated"), "should contain truncation marker");
        // Original content prefix taken must be <= 2000 bytes
        assert!(s.len() < 3000, "output must be shorter than original");

        // With no budget (None), falls back to MAX_FILE_BYTES=4096, file fits entirely
        let result_default = Supervisor::read_file_truncated(tmp.path(), None);
        assert!(result_default.is_some());
        // 3000 bytes < 4096 fallback, so no truncation
        assert!(!result_default.unwrap().contains("truncated"), "3000-byte file should not be truncated at default 4096 limit");
    }

    /// S04: budget=2000 truncates but budget=None falls back to 4096 (no truncation for 3000-byte file).
    #[test]
    fn test_read_file_truncated_fallback_to_max_file_bytes() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        // 5000-byte file exceeds default 4096 fallback
        let mut tmp = NamedTempFile::new().expect("tempfile");
        let content = "y".repeat(5000);
        tmp.write_all(content.as_bytes()).expect("write");
        tmp.flush().expect("flush");

        let result = Supervisor::read_file_truncated(tmp.path(), None);
        assert!(result.is_some());
        assert!(result.unwrap().contains("truncated"), "5000-byte file should be truncated at default 4096 limit");
    }
}
