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
use tokio::sync::oneshot;

use crate::pipeline::agent_def::{AgentDefinition, SwarmComposition};
use crate::pipeline::agents::{DocumenterAgent, ReviewerAgent, TesterAgent, UxReviewerAgent};
use crate::pipeline::cli_runner::CliRunner;
use crate::pipeline::code_phase::CodePhase;
use crate::pipeline::fix_agent::{FixAgent, FixTaskInput};
use crate::pipeline::model_selection::{ModelSelector, SelectedModel, TaskType, is_compatible_with_provider};
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

// ── AgentMetrics ─────────────────────────────────────────────────────────

/// Performance metrics captured by `dispatch_agent` for a single agent invocation.
/// Written by `dispatch_agent`, consumed once by `execute_agent_tracked` for session logging.
#[derive(Debug, Clone, Default)]
pub struct AgentMetrics {
    pub model: Option<String>,
    pub tokens: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
}

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
    /// hex project ID for architecture fingerprint injection (ADR-2603301200).
    pub project_id: Option<String>,
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
    /// Pre-existing HexFlo task IDs created by SwarmPhase (key = step ID like "P0.1").
    /// When present, `execute_agent_tracked` reuses these instead of creating new shadow tasks.
    task_id_map: HashMap<String, String>,
    /// RL model selector — used to report code generation outcomes after objective evaluation.
    selector: ModelSelector,
    /// Last code step selection + duration, stored so `run_tier` can call `report_outcome`
    /// once `CodeCompiles` state is known (after `evaluate_all`).
    last_code_selection: Mutex<Option<(SelectedModel, u64)>>,
    /// Metrics from the most recent `dispatch_agent` call.
    /// Written by `dispatch_agent`, consumed once by `execute_agent_tracked`.
    last_dispatch_metrics: Mutex<Option<AgentMetrics>>,
    /// hex project ID for architecture fingerprint injection (ADR-2603301200).
    pub project_id: Option<String>,
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

        let selector = ModelSelector::new(&nexus_url);
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
            task_id_map: HashMap::new(),
            selector,
            last_code_selection: Mutex::new(None),
            last_dispatch_metrics: Mutex::new(None),
            project_id: None,
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

    /// Supply pre-existing HexFlo task IDs from the SwarmPhase.
    ///
    /// Key = workplan step ID (e.g. "P0.1"), value = HexFlo task UUID.
    /// When set, `execute_agent_tracked` will reuse these task IDs instead of
    /// creating new shadow tasks, ensuring the dashboard shows the correct P*
    /// tasks progressing rather than duplicate "hex-coder: [iteration N]" tasks.
    pub fn with_task_ids(mut self, task_id_map: HashMap<String, String>) -> Self {
        self.task_id_map = task_id_map;
        self
    }

    /// Set the hex project ID for architecture fingerprint injection (ADR-2603301200).
    ///
    /// When set, all agent contexts will include this ID so agents can fetch and
    /// inject the architecture fingerprint into their inference system prompts.
    pub fn with_project_id(mut self, project_id: impl Into<Option<String>>) -> Self {
        self.project_id = project_id.into();
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

        // For Rust/Go single-binary projects, files live under src/ root (Rust)
        // or the project root directly (Go: main.go, *_test.go).
        let dirs: Vec<PathBuf> = match lang {
            "rust" => vec![base.clone()],
            "go" => vec![PathBuf::from(&self.output_dir)],
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

    /// Strip the output_dir prefix from a coder-returned file path if the model
    /// accidentally included it (e.g. `examples/proj/src/foo.ts` → `src/foo.ts`).
    /// This prevents doubled paths when the supervisor joins output_dir + rel_path.
    ///
    /// When `output_dir` is absolute (anchored to git root), LLMs typically
    /// return repo-relative paths like `"examples/proj/src/foo.ts"` rather than
    /// the full absolute path.  In that case we also try stripping any trailing
    /// suffix of `output_dir` that matches the start of `path` (longest first).
    fn strip_output_dir_prefix<'a>(&self, path: &'a str) -> &'a str {
        let prefix_slash = format!("{}/", self.output_dir);
        if let Some(stripped) = path.strip_prefix(&prefix_slash) {
            return stripped;
        }
        if path == self.output_dir {
            return "";
        }
        // When output_dir is absolute, also try repo-relative sub-paths.
        // E.g. for output_dir="/repo/examples/proj", try stripping:
        //   "repo/examples/proj/", "examples/proj/", "proj/" (longest first).
        let out_path = std::path::Path::new(&self.output_dir);
        if out_path.is_absolute() {
            let components: Vec<_> = out_path
                .components()
                .filter(|c| !matches!(
                    c,
                    std::path::Component::RootDir | std::path::Component::Prefix(_)
                ))
                .collect();
            for start in 0..components.len() {
                let candidate: std::path::PathBuf = components[start..].iter().collect();
                let cand_str = format!("{}/", candidate.display());
                if let Some(stripped) = path.strip_prefix(&cand_str) {
                    return stripped;
                }
            }
        }
        path
    }

    /// Extract the package name from a Cargo.toml string (the `[package] name` field).
    /// Used to inject the correct binary name into tester/fixer prompts so the model
    /// never has to guess the `CARGO_BIN_EXE_<name>` macro argument.
    fn cargo_package_name(cargo_toml: &str) -> Option<String> {
        let mut in_package = false;
        for line in cargo_toml.lines() {
            let t = line.trim();
            if t == "[package]" { in_package = true; continue; }
            if t.starts_with('[') { in_package = false; continue; }
            if in_package && t.starts_with("name") {
                if let Some(eq) = t.find('=') {
                    let v = t[eq + 1..].trim().trim_matches(|c: char| c == '"' || c == '\'');
                    if !v.is_empty() { return Some(v.to_string()); }
                }
            }
        }
        None
    }

    /// Read binary name from `Cargo.toml` in the output directory (Rust projects only).
    fn rust_binary_name(&self) -> Option<String> {
        let cargo_path = PathBuf::from(&self.output_dir).join("Cargo.toml");
        let content = fs::read_to_string(&cargo_path).ok()?;
        Self::cargo_package_name(&content)
    }

    /// Read port interface files from `src/core/ports/`.
    /// Only files matching the project language are counted — a TypeScript port
    /// file in a Rust project is an artefact of the coder generating hex scaffolding
    /// for a standalone project, not a real port interface.
    fn port_files(&self) -> Vec<(String, String)> {
        let ports_dir = PathBuf::from(&self.output_dir)
            .join("src")
            .join("core")
            .join("ports");
        let mut files = Vec::new();
        Self::collect_files(&ports_dir, &self.output_dir, &mut files);
        let expected_ext = if self.language == "rust" { ".rs" } else { ".ts" };
        files.retain(|(path, _)| path.ends_with(expected_ext));
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
            project_id: self.project_id.clone(),
        }
    }

    /// Build context for a **reviewer** agent examining a specific file.
    pub fn build_reviewer_context(&self, file_path: &str, workplan_summary: &str) -> AgentContext {
        let tier = Self::infer_tier_from_path(file_path);
        let port_interfaces = self.port_files();
        let mut metadata = HashMap::new();
        metadata.insert("language".into(), self.language.clone());
        metadata.insert("review_target".into(), file_path.to_string());
        metadata.insert("workplan_summary".into(), workplan_summary.to_string());
        // Signal whether this is a hexagonal project (has port interfaces) or a
        // standalone project (examples, CLIs, etc.).  The reviewer uses this to
        // avoid flagging missing port interfaces as a violation in standalone code.
        let project_type = if port_interfaces.is_empty() {
            "standalone"
        } else {
            "hexagonal"
        };
        metadata.insert("project_type".into(), project_type.to_string());

        AgentContext {
            prompt_template: "agent-reviewer".into(),
            source_files: self.read_single_file(file_path),
            port_interfaces,
            boundary_rules: self.rules_for_tier(tier),
            workplan_step: None,
            upstream_output: None,
            metadata,
            project_id: self.project_id.clone(),
        }
    }

    /// Build context for a **tester** agent writing tests for a specific file.
    pub fn build_tester_context(&self, file_path: &str) -> AgentContext {
        let tier = Self::infer_tier_from_path(file_path);
        let mut metadata = HashMap::new();
        metadata.insert("language".into(), self.language.clone());
        metadata.insert("test_target".into(), file_path.to_string());
        // Inject exact binary name so the tester never hallucinates CARGO_BIN_EXE_<name>
        if self.language == "rust" {
            if let Some(name) = self.rust_binary_name() {
                metadata.insert("binary_name".into(), name);
            }
        }

        AgentContext {
            prompt_template: "agent-tester".into(),
            source_files: self.read_single_file(file_path),
            port_interfaces: self.port_files(),
            boundary_rules: self.rules_for_tier(tier),
            workplan_step: None,
            upstream_output: None,
            metadata,
            project_id: self.project_id.clone(),
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
            project_id: self.project_id.clone(),
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
            project_id: self.project_id.clone(),
        }
    }

    /// Build context for a **fixer** agent resolving an issue in a file.
    pub fn build_fixer_context(&self, file_path: &str, issue_desc: &str) -> AgentContext {
        let tier = Self::infer_tier_from_path(file_path);
        let mut metadata = HashMap::new();
        metadata.insert("language".into(), self.language.clone());
        metadata.insert("fix_target".into(), file_path.to_string());
        // Inject exact binary name for Rust so the fixer uses the correct CARGO_BIN_EXE_<name>
        if self.language == "rust" {
            if let Some(name) = self.rust_binary_name() {
                metadata.insert("binary_name".into(), name);
            }
        }

        AgentContext {
            prompt_template: "agent-fixer".into(),
            source_files: self.read_single_file(file_path),
            port_interfaces: self.port_files(),
            boundary_rules: self.rules_for_tier(tier),
            workplan_step: None,
            upstream_output: Some(issue_desc.to_string()),
            metadata,
            project_id: self.project_id.clone(),
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
                    _ => {
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

        // Apply token budget and pressure thresholds if specified
        if let Some(ref budget) = ctx_config.token_budget {
            metadata.insert("token_budget_max".into(), budget.max.to_string());
            if let Some(ref p) = budget.pressure {
                metadata.insert("pressure_warn_pct".into(), p.warn_at_pct.to_string());
                metadata.insert("pressure_compress_pct".into(), p.compress_at_pct.to_string());
                metadata.insert("pressure_block_pct".into(), p.block_at_pct.to_string());
                metadata.insert("pressure_relief".into(), p.relief.clone());
            }
        }

        AgentContext {
            prompt_template: format!("agent-{}", agent_def.agent_type),
            source_files,
            port_interfaces,
            boundary_rules: self.rules_for_tier(tier),
            workplan_step: Some(step_desc.to_string()),
            upstream_output: None,
            metadata,
            project_id: self.project_id.clone(),
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
        let use_sandbox = Self::sandbox_available();

        let mut workers = self.workers.lock().unwrap();
        for role in roles {
            let swarm_arg = self.swarm_id.as_deref().unwrap_or("");

            // Pipe worker stdout+stderr to a log file for diagnostics
            let log_dir = dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                .join(".hex/logs");
            let _ = std::fs::create_dir_all(&log_dir);
            let log_path = log_dir.join(format!(
                "worker-{}-{}.log",
                role,
                chrono::Utc::now().format("%Y%m%d-%H%M%S")
            ));
            let log_file = std::fs::File::create(&log_path)
                .unwrap_or_else(|_| std::fs::File::open("/dev/null").unwrap());
            let log_file2 = log_file.try_clone()
                .unwrap_or_else(|_| std::fs::File::open("/dev/null").unwrap());

            let abs_output = std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join(&self.output_dir);
            let abs_output_str = abs_output.to_string_lossy().to_string();

            let child = if use_sandbox {
                // Spawn hex-agent daemon inside an isolated Docker container (ADR-2603282000).
                //
                // Uses plain `docker run` — Docker AI Sandbox (docker sandbox) is designed for
                // interactive agents (Claude, Copilot); hex-agent is a background daemon that
                // needs to stay running while polling, which conflicts with sandbox init hooks.
                //
                // `docker run --rm` spins up the container, runs the daemon, and cleans up on exit.
                // child.try_wait() monitors liveness via the docker process.

                // Rewrite localhost/127.0.0.1 → host.docker.internal so hex-agent daemon
                // inside the container can reach the nexus daemon on the host.
                let nexus_host = self.nexus_url
                    .trim_start_matches("http://")
                    .trim_start_matches("https://")
                    .split(':')
                    .next()
                    .unwrap_or("localhost")
                    .replace("localhost", "host.docker.internal")
                    .replace("127.0.0.1", "host.docker.internal");
                let nexus_port = self.nexus_url
                    .split(':')
                    .next_back()
                    .unwrap_or("5555")
                    .trim_end_matches('/')
                    .to_string();

                // Deterministic container name per role (removed in kill_workers).
                let sandbox_name = format!("hex-sandbox-{}", role);

                // Remove any pre-existing container with this name (best-effort).
                let _ = std::process::Command::new("docker")
                    .args(["rm", "-f", &sandbox_name])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .output();

                // docker run --rm: mount output dir, inject env, run hex-agent daemon.
                let mut cmd = std::process::Command::new("docker");
                cmd.args([
                    "run", "--rm",
                    "--name", &sandbox_name,
                    "-v", &format!("{}:{}", abs_output_str, abs_output_str),
                    "--add-host", "host.docker.internal:host-gateway",
                    "-e", &format!("NEXUS_HOST={nexus_host}"),
                    "-e", &format!("NEXUS_PORT={nexus_port}"),
                    "-e", &format!("HEX_NEXUS_URL=http://{}:{}", nexus_host, nexus_port),
                    "-e", "RUST_LOG=info",
                    "-e", &format!("HEX_OUTPUT_DIR={abs_output_str}"),
                ]);
                // Docker workers use the pull model: they register fresh UUIDs and
                // self-claim tasks by role via /claim. Do NOT pass --agent-id so each
                // container gets its own unique identity for step 2 task execution.
                // Run `hex agent worker --role <role>` — same command as the non-docker path.
                // Override ENTRYPOINT so we invoke the `hex` CLI binary, not hex-agent.
                // Propagate inference config so workers use the same model/provider
                if let Some(ref m) = self.model_override {
                    cmd.args(["-e", &format!("HEX_MODEL={}", m)]);
                }
                if let Some(ref p) = self.provider_pref {
                    cmd.args(["-e", &format!("HEX_PROVIDER={}", p)]);
                }
                cmd.args(["--entrypoint", "hex", "hex-agent:latest",
                    "agent", "worker", "--role", role]);
                if !swarm_arg.is_empty() {
                    cmd.args(["--swarm-id", swarm_arg]);
                }
                cmd.stdout(log_file).stderr(log_file2);

                let child = cmd.spawn().with_context(|| {
                    format!("docker run failed for role {}", role)
                })?;
                println!("  Spawned {} daemon in Docker container '{}' (PID {})", role, sandbox_name, child.id());
                child
            } else {
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
                // Scope the worker to the example project directory so it reads/writes
                // the right source files rather than the entire hex-intf workspace.
                cmd.env("HEX_OUTPUT_DIR", &abs_output_str);
                // Propagate inference config so workers use the same model/provider
                if let Some(ref m) = self.model_override {
                    cmd.env("HEX_MODEL", m);
                }
                if let Some(ref p) = self.provider_pref {
                    cmd.env("HEX_PROVIDER", p);
                }
                cmd.env("HEX_NEXUS_URL", &self.nexus_url);
                cmd.current_dir(&abs_output_str);
                cmd.stdout(log_file).stderr(log_file2);

                let child = cmd.spawn().with_context(|| {
                    format!("Failed to spawn worker for role {}", role)
                })?;
                println!("  Spawned {} worker (PID {})", role, child.id());
                child
            };

            workers.push((role.clone(), child));
        }
        Ok(())
    }

    /// Returns true when Docker is available for isolated worker containers.
    /// ADR-2603282000: workers run inside plain `docker run` containers.
    fn sandbox_available() -> bool {
        // HEX_NO_SANDBOX=1 forces local worker mode (no Docker isolation).
        // Useful when the project lives on a volume Docker can't bind-mount
        // (e.g. /Volumes/... on macOS external drives) or for faster iteration.
        if std::env::var("HEX_NO_SANDBOX").is_ok() {
            return false;
        }
        std::process::Command::new("docker")
            .args(["info", "--format", "{{.ServerVersion}}"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Kill all spawned worker processes and wait for them to exit.
    /// For Docker workers, also force-removes the container.
    fn kill_workers(&self) {
        let use_sandbox = Self::sandbox_available();
        let mut workers = self.workers.lock().unwrap();
        for (role, child) in workers.iter_mut() {
            let _ = child.kill();
            let _ = child.wait();
            if use_sandbox {
                let container_name = format!("hex-sandbox-{}", role);
                let _ = std::process::Command::new("docker")
                    .args(["rm", "-f", &container_name])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .output();
                debug!(role = role.as_str(), container = %container_name, "removed docker container");
            }
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

        // Spawn worker processes for each role needed by the workplan.
        // hex-fixer is excluded: it runs inline inside dispatch_agent so it has
        // direct access to the current error context and can complete synchronously
        // before the next goal-loop iteration. A long-running worker process would
        // receive a stub result (the worker handler in agent.rs does not call FixAgent).
        let roles: Vec<String> = Self::roles_for_workplan(workplan)
            .into_iter()
            .filter(|r| r != "hex-fixer")
            .collect();
        if let Err(e) = self.spawn_workers(&roles) {
            warn!(error = %e, "failed to spawn workers — falling back to inline execution");
        } else if !self.workers.lock().unwrap().is_empty() {
            println!("  Waiting for workers to register with nexus...");
            std::thread::sleep(std::time::Duration::from_secs(2));
        }

        // Clean stale source/test directories from any previous pipeline run.
        // Without this, leftover files contaminate the coder's context and cause
        // it to generate code that references types from a completely different project.
        // Also clean `examples/` subdirectory which can contain doubled-path artifacts
        // from prior runs where the coder included output_dir in the returned file path.
        for stale_dir in &["src", "tests", "examples"] {
            let dir = PathBuf::from(&self.output_dir).join(stale_dir);
            if dir.is_dir() {
                if let Err(e) = fs::remove_dir_all(&dir) {
                    warn!(error = %e, dir = *stale_dir, "failed to remove stale directory");
                } else {
                    info!(dir = *stale_dir, "removed stale directory before pipeline start");
                }
            }
        }
        // Also remove any nested dir that matches the project directory name itself
        // (from doubled-path files: output_dir/output_dir_name/src/...).
        if let Some(project_name) = PathBuf::from(&self.output_dir)
            .file_name()
            .and_then(|n| n.to_str())
        {
            let doubled = PathBuf::from(&self.output_dir).join(project_name);
            if doubled.is_dir() {
                let _ = fs::remove_dir_all(&doubled);
                info!(dir = project_name, "removed doubled-path stale directory");
            }
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

        // Spawn background heartbeat task to keep the agent alive during long pipeline runs.
        // Without this, the cleanup daemon reclaims task assignments after 45s of silence.
        let _heartbeat_guard = if let Some(ref aid) = self.agent_id {
            let agent_id = aid.clone();
            let nexus_url = self.nexus_url.clone();
            let (tx, mut rx) = oneshot::channel::<()>();
            tokio::spawn(async move {
                let client = reqwest::Client::new();
                let url = format!("{}/api/agents/{}/heartbeat", nexus_url, agent_id);
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(tokio::time::Duration::from_secs(30)) => {
                            let _ = client
                                .post(&url)
                                .json(&serde_json::json!({ "timestamp": chrono::Utc::now().to_rfc3339() }))
                                .send()
                                .await;
                            debug!(agent_id = %agent_id, "supervisor heartbeat sent");
                        }
                        _ = &mut rx => {
                            debug!("supervisor heartbeat task cancelled");
                            break;
                        }
                    }
                }
            });
            Some(tx)
        } else {
            None
        };

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
            let halted = matches!(&result, TierResult::Halted { .. });
            tier_results.push((tier, result));

            if halted {
                let reason = tier_results
                    .last()
                    .and_then(|(_, r)| if let TierResult::Halted { reason, .. } = r { Some(reason.clone()) } else { None })
                    .unwrap_or_else(|| format!("tier {} exhausted max iterations", tier));
                eprintln!("\n[hex] Pipeline halted: {}\n[hex] Fix the issues above and re-run `hex dev`.", reason);
                return Err(anyhow::anyhow!("pipeline halted: {}", reason));
            }

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
        let max_iterations = crate::pipeline::SwarmConfig::load_default().max_iterations_per_tier();

        let objectives = objectives_for_tier(tier, has_ui_adapters, is_final_tier);

        // Track which objectives have had a prior agent result (for fixer vs primary agent selection)
        let mut prior_results: HashMap<Objective, bool> = HashMap::new();
        // Accumulate error outputs per objective across fix iterations (last 2 kept).
        let mut prior_errors_map: HashMap<Objective, Vec<String>> = HashMap::new();
        // P2 (ADR-2604070400): Track fixer output hashes to detect loops.
        // Maps objective → list of SHA-256 hashes of blocking_issues after each fixer run.
        let mut fixer_hashes: HashMap<Objective, Vec<String>> = HashMap::new();
        let mut fixer_stuck_count: HashMap<Objective, u32> = HashMap::new();

        for iteration in 1..=max_iterations {
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

            // Report RL outcome for code generation once CodeCompiles state is known.
            // `report_outcome` is a no-op when the model was not RL-selected (YAML/default),
            // so this is safe to call unconditionally — it will never panic.
            let rl_report = if let Ok(mut guard) = self.last_code_selection.lock() {
                let result = if let Some((ref selected, duration_ms)) = *guard {
                    Some((selected.clone(), duration_ms))
                } else {
                    None
                };
                if result.is_some() {
                    *guard = None;
                }
                result
            } else {
                None
            };
            if let Some((selected, duration_ms)) = rl_report {
                let compile_met = states
                    .iter()
                    .find(|s| matches!(s.objective, Objective::CodeCompiles))
                    .map(|s| s.met)
                    .unwrap_or(false);
                if let Err(e) = self
                    .selector
                    .report_outcome(&selected, TaskType::CodeGeneration, compile_met, 0.0, duration_ms)
                    .await
                {
                    debug!(error = %e, "RL reward report failed — continuing (non-fatal)");
                }
            }

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
            let prior_errors = prior_errors_map.get(&obj).cloned().unwrap_or_default();
            let agent_result = self.execute_agent_tracked(
                agent_role,
                tier,
                unmet_state,
                workplan_steps,
                adr_content,
                workplan_summary,
                iteration,
                &prior_errors,
            )
            .await;

            // If this was a fixer role and the objective still has blocking issues,
            // carry the current error forward for the next iteration (keep last 2).
            if agent_role == "hex-fixer" {
                let errors = prior_errors_map.entry(obj).or_default();
                let current_error = unmet_state.blocking_issues.join("\n");
                if !current_error.is_empty() {
                    errors.push(current_error.clone());
                    if errors.len() > 2 {
                        errors.remove(0);
                    }
                }

                // P2 (ADR-2604070400): Detect fixer loop via content hash.
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut hasher = DefaultHasher::new();
                current_error.hash(&mut hasher);
                let hash = format!("{:x}", hasher.finish());

                let hashes = fixer_hashes.entry(obj).or_default();
                let is_repeat = hashes.last().map_or(false, |prev| prev == &hash);
                hashes.push(hash);

                if is_repeat {
                    let stuck = fixer_stuck_count.entry(obj).or_insert(0);
                    *stuck += 1;
                    if *stuck >= 4 {
                        // After 4 repeated failures: regenerate from scratch
                        warn!(
                            objective = %obj,
                            stuck_count = *stuck,
                            "fixer loop detected (4x) — resetting to coder for full regeneration"
                        );
                        // Reset so next iteration dispatches coder, not fixer
                        prior_results.insert(obj, false);
                        fixer_stuck_count.insert(obj, 0);
                        fixer_hashes.remove(&obj);
                    } else if *stuck >= 2 {
                        // After 2 repeated failures: signal model upgrade via env
                        // (ModelSelector checks HEX_MODEL_UPGRADE on next select_model call)
                        warn!(
                            objective = %obj,
                            stuck_count = *stuck,
                            "fixer loop detected (2x) — signalling model upgrade"
                        );
                        std::env::set_var("HEX_MODEL_UPGRADE", "1");
                    }
                } else {
                    // Different output — reset stuck counter
                    fixer_stuck_count.insert(obj, 0);
                }
            }

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

            // Mark that this objective now has a prior result.
            // Exception: for ReviewPasses, alternate reviewer → fixer → reviewer by resetting
            // to false after the fixer runs, so the next iteration re-reviews the fixed code.
            let next_prior = if obj == Objective::ReviewPasses && has_prior {
                false // fixer just ran — reset so reviewer runs next
            } else {
                true
            };
            prior_results.insert(obj, next_prior);
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
            tier, max_iterations
        );
        print_iteration_progress(tier, max_iterations, &final_states);

        // Identify the first unmet objective for the notification message.
        let first_unmet = final_states
            .iter()
            .find(|s| !s.met && s.skip_reason.is_none())
            .map(|s| format!("{}", s.objective))
            .unwrap_or_else(|| "unknown objective".to_string());
        let reason = format!(
            "Pipeline stalled: tier {} exhausted {} iterations on {}",
            tier, max_iterations, first_unmet
        );
        let last_error = final_states
            .iter()
            .find(|s| !s.met && s.skip_reason.is_none())
            .map(|s| s.blocking_issues.join("\n"))
            .unwrap_or_default();

        // Send a critical inbox notification (best-effort — don't abort on failure).
        {
            let nexus = crate::nexus_client::NexusClient::new(self.nexus_url.clone());
            let body = serde_json::json!({
                "priority": 2,
                "kind": "pipeline_stalled",
                "payload": serde_json::json!({
                    "title": reason,
                    "body": last_error,
                }).to_string(),
            });
            if let Err(e) = nexus.post("/api/hexflo/inbox/notify", &body).await {
                warn!(error = %e, "failed to send pipeline-stalled inbox notification");
            }
        }

        Ok(TierResult::Halted {
            reason,
            states: final_states,
        })
    }

    // ── HexFlo task tracking helpers ────────────────────────────────────

    /// Create (or reuse) a HexFlo task for the given role/objective (best-effort).
    ///
    /// If `step_id` matches a key in `self.task_id_map` (populated from SwarmPhase),
    /// the existing task ID is returned and the task is marked in_progress via PATCH.
    /// Otherwise a new shadow task is created as before.
    ///
    /// Returns the task ID if successful.
    async fn create_tracking_task(
        &self,
        role: &str,
        objective: &Objective,
        iteration: u32,
        step_id: Option<&str>,
        step: Option<&WorkplanStep>,
    ) -> Option<String> {
        // Reuse a SwarmPhase-created task only for the coder role on iteration 1.
        // Other roles (reviewer, tester, fixer) always create a fresh task so
        // they don't collide with completed coder tasks sharing the same step_id.
        if iteration == 1 && role == "hex-coder" {
            if let Some(sid) = step_id {
                if let Some(existing_id) = self.task_id_map.get(sid) {
                    debug!(task_id = %existing_id, step_id = %sid, role, "reusing SwarmPhase HexFlo task (iteration 1)");
                    return Some(existing_id.clone());
                }
            }
        }

        let swarm_id = self.swarm_id.as_ref()?;
        let runner = CliRunner::new();

        // Encode the WorkplanStep as TaskPayload JSON so the worker can deserialize
        // `step_id`, `description`, `model_hint`, and `output_dir` without needing
        // a separate hexflo memory lookup (ADR-2603300100 P4.1).
        let title = if let Some(s) = step {
            let mut payload = serde_json::json!({
                "role": role,
                "step_id": s.id,
                "description": s.description,
                "output_dir": self.output_dir,
            });
            if iteration > 1 {
                payload["iteration"] = serde_json::json!(iteration);
            }
            payload.to_string()
        } else {
            format!("{}: {} [iteration {}]", role, objective, iteration)
        };

        // Create unassigned (None agent_id) so docker workers can self-claim via /claim.
        match runner.task_create(swarm_id, &title, None) {
            Ok(resp) => {
                let task_id = resp["id"].as_str().map(|s| s.to_string());
                if let Some(ref tid) = task_id {
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

    /// Store WorkplanStep metadata + output_dir in hexflo memory so Docker
    /// workers can retrieve full execution context when picking up a task.
    ///
    /// Key: `{task_id}:step_metadata`
    /// Value: JSON `{ steps: [...], output_dir: "..." }`
    async fn store_step_metadata(&self, task_id: &str, steps: &[&WorkplanStep]) {
        let nexus = crate::nexus_client::NexusClient::new(self.nexus_url.clone());
        let steps_json: Vec<serde_json::Value> = steps
            .iter()
            .filter_map(|s| serde_json::to_value(s).ok())
            .collect();
        let metadata = serde_json::json!({
            "steps": steps_json,
            "output_dir": self.output_dir,
            "model": self.model_override.as_deref().unwrap_or(""),
            "provider": self.provider_pref.as_deref().unwrap_or(""),
        });
        let key = format!("{}:step_metadata", task_id);
        let scope = self.swarm_id.clone().unwrap_or_default();
        let body = serde_json::json!({
            "key": key,
            "value": metadata.to_string(),
            "scope": scope,
        });
        if let Err(e) = nexus.post("/api/hexflo/memory", &body).await {
            debug!(error = %e, task_id, "failed to store step metadata (non-blocking)");
        } else {
            debug!(task_id, "stored step metadata for worker");
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

    /// Read structured result stored by a Docker worker under `{task_id}:result` in hexflo memory.
    async fn read_worker_result(&self, task_id: &str) -> Option<WorkerResult> {
        let key = format!("{}:result", task_id);
        let nexus = crate::nexus_client::NexusClient::new(self.nexus_url.clone());
        let resp = nexus
            .get(&format!("/api/hexflo/memory/{}", key))
            .await
            .ok()?;
        let value_str = resp["value"].as_str()?;
        serde_json::from_str::<WorkerResult>(value_str).ok()
    }

    /// Log a per-role ToolCall to the attached dev session (best-effort).
    #[allow(clippy::too_many_arguments)]
    fn log_agent_performance(
        &self,
        role: &str,
        model: Option<&str>,
        tokens: Option<u64>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
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
                    input_tokens,
                    output_tokens,
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

    /// Store agent metrics from `dispatch_agent` for consumption by `execute_agent_tracked`.
    fn store_dispatch_metrics(&self, metrics: AgentMetrics) {
        if let Ok(mut guard) = self.last_dispatch_metrics.lock() {
            *guard = Some(metrics);
        }
    }

    // ── Iteration counter for task titles ────────────────────────────────

    /// Get the current iteration count from the tier loop context.
    /// This is injected via `run_tier` into `execute_agent`.
    /// We thread it through via an extra parameter.
    ///
    /// Dispatch to the right agent with HexFlo tracking and performance logging.
    #[allow(clippy::too_many_arguments)]
    async fn execute_agent_tracked(
        &self,
        role: &str,
        tier: u32,
        state: &ObjectiveState,
        workplan_steps: &[&WorkplanStep],
        adr_content: &str,
        workplan_summary: &str,
        iteration: u32,
        prior_errors: &[String],
    ) -> Result<()> {
        // Create (or reuse) HexFlo tracking task (best-effort).
        // Use the first workplan step's ID as the map key so SwarmPhase tasks
        // are reused instead of creating duplicate shadow tasks.
        let first_step = workplan_steps.first().copied();
        let step_id = first_step.map(|s| s.id.as_str());
        let tracking_task_id = self
            .create_tracking_task(role, &state.objective, iteration, step_id, first_step)
            .await;

        // Store step metadata in hexflo memory so Docker workers can read the
        // full WorkplanStep context + output_dir when they pick up the task.
        if let Some(ref tid) = tracking_task_id {
            self.store_step_metadata(tid, workplan_steps).await;
        }

        // Read cardinality for this role from the swarm YAML (ADR-2603240130 S06).
        let cardinality = crate::pipeline::SwarmConfig::load_default().cardinality_for_role(role);
        info!(role = %role, cardinality = ?cardinality, "agent cardinality from swarm YAML");

        let start = Instant::now();

        // Decide: delegate to worker process or execute inline.
        // Delegates to a Docker worker when one is registered for this role,
        // falls back to inline execution when Docker is unavailable.
        let agent_result = if self.has_worker_for_role(role) {
            // ── Worker delegation path ──────────────────────────────────
            // Tasks are left in "pending" state so workers self-claim via
            // the role-guarded /claim endpoint (pull model, ADR-2603282000).
            // Do NOT call `task assign` here — that would set the task
            // in_progress with the supervisor's agent_id, which workers
            // never match when polling for their own tasks.
            if let Some(ref tid) = tracking_task_id {
                // Poll until the worker completes the task (configurable via HEX_WORKER_TIMEOUT)
                let poll_start = Instant::now();
                let timeout_secs: u64 = std::env::var("HEX_WORKER_TIMEOUT")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(600); // default 10 minutes (was 5)
                let timeout = std::time::Duration::from_secs(timeout_secs);
                let mut retries = 0u32;
                let poll_result: Result<()> = loop {
                    if poll_start.elapsed() > timeout {
                        break Err(anyhow::anyhow!(
                            "Task {} timed out after {}s waiting for worker {}",
                            tid,
                            timeout_secs,
                            role
                        ));
                    }

                    // Check if worker is still alive
                    let worker_dead = {
                        let mut workers = self.workers.lock().unwrap();
                        let dead = workers
                            .iter_mut()
                            .find(|(r, _)| r == role)
                            .map(|(_, child)| {
                                child.try_wait().ok().flatten().is_some()
                            })
                            .unwrap_or(true);

                        if dead {
                            // Remove dead worker while we hold the lock
                            workers.retain(|(r, _)| r != role);
                        }
                        dead
                    }; // MutexGuard dropped here, before any .await

                    if worker_dead {
                        warn!(
                            role,
                            task_id = ?tid,
                            "worker process died — respawning"
                        );

                        // Respawn
                        self.spawn_workers(&[role.to_string()])?;

                        // Reset task to pending so the new worker can self-claim it.
                        let nexus_reset = crate::nexus_client::NexusClient::new(self.nexus_url.clone());
                        let _ = nexus_reset.patch(
                            &format!("/api/hexflo/tasks/{}", tid),
                            &serde_json::json!({"status": "pending", "agentId": ""}),
                        ).await;

                        retries += 1;
                        if retries > 3 {
                            break Err(anyhow::anyhow!(
                                "Worker for role {} died {} times — giving up",
                                role,
                                retries
                            ));
                        }

                        // Wait for new worker to register
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                        continue;
                    }

                    // Poll task status directly via HTTP (no CLI subprocess needed).
                    // `hex task status` does not exist as a subcommand; use the
                    // /api/hexflo/tasks/:id endpoint instead.
                    let nexus_http = crate::nexus_client::NexusClient::new(self.nexus_url.clone());
                    if let Ok(status) = nexus_http.get(&format!("/api/hexflo/tasks/{}", tid)).await {
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

                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                };
                // P2.1: After worker completes, read result from hexflo memory for observability.
                // The actual ObjectiveState update happens on the next iteration via evaluate_all().
                if poll_result.is_ok() {
                    if let Some(worker_result) = self.read_worker_result(tid).await {
                        info!(
                            task_id = %tid,
                            file_path = %worker_result.file_path,
                            compile_pass = worker_result.compile_pass,
                            tests_pass = worker_result.tests_pass,
                            "worker result retrieved from hexflo memory"
                        );
                        println!(
                            "  [worker] {} — compile:{} tests:{} file:{}",
                            role,
                            if worker_result.compile_pass { "✓" } else { "✗" },
                            if worker_result.tests_pass { "✓" } else { "✗" },
                            worker_result.file_path,
                        );
                        // Store audit metrics so execute_agent_tracked logs them once.
                        if worker_result.model.is_some() || worker_result.tokens.is_some() {
                            self.store_dispatch_metrics(AgentMetrics {
                                model: worker_result.model.clone(),
                                tokens: worker_result.tokens,
                                input_tokens: worker_result.input_tokens,
                                output_tokens: worker_result.output_tokens,
                                cost_usd: worker_result.cost_usd,
                            });
                        }
                    }
                }
                poll_result
            } else {
                // No tracking task ID — cannot delegate without a task, fall back to inline
                debug!(role, "no tracking task ID — falling back to inline dispatch");
                self.dispatch_agent(
                    role, tier, state, workplan_steps, adr_content, workplan_summary, prior_errors,
                )
                .await
            }
        } else {
            // ── Inline fallback (no workers running) ────────────────────
            self.dispatch_agent(
                role, tier, state, workplan_steps, adr_content, workplan_summary, prior_errors,
            )
            .await
        };

        let duration_ms = start.elapsed().as_millis() as u64;
        let success = agent_result.is_ok();

        // Read metrics stored by dispatch_agent (or worker path) and log once.
        // dispatch_agent writes to last_dispatch_metrics before returning; we
        // consume it here so there is exactly one ToolCall entry per agent run.
        let metrics = self.last_dispatch_metrics.lock()
            .ok()
            .and_then(|mut g| g.take())
            .unwrap_or_default();
        self.log_agent_performance(
            role,
            metrics.model.as_deref(),
            metrics.tokens,
            metrics.input_tokens,
            metrics.output_tokens,
            metrics.cost_usd,
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
    #[allow(clippy::too_many_arguments)]
    async fn dispatch_agent(
        &self,
        role: &str,
        tier: u32,
        state: &ObjectiveState,
        workplan_steps: &[&WorkplanStep],
        _adr_content: &str,
        workplan_summary: &str,
        prior_errors: &[String],
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
                    let effective_model: Option<&str> = model_override
                        .or_else(|| is_compatible_with_provider(&yaml_model_id, provider_pref)
                            .then_some(yaml_model_id.as_str()));
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

                        // Default mode is "tdd" (5-phase YAML workflow). Set HEX_PHASE_MODE=single
                        // to opt out of TDD overhead (e.g. for quick one-shot generation).
                        let phase_mode = std::env::var("HEX_PHASE_MODE")
                            .unwrap_or_else(|_| "tdd".to_string());

                        let result = if phase_mode != "single" {
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
                                        Some(self.output_dir.as_str()),
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
                                .execute_step(step, &step_workplan, effective_model, provider_pref, Some(self.output_dir.as_str()))
                                .await
                                .with_context(|| format!("code phase step {} failed", step.id))?
                        };

                        // Store metrics for session audit trail (ADR-2604071300)
                        self.store_dispatch_metrics(AgentMetrics {
                            model: Some(result.model_used.clone()),
                            tokens: Some(result.tokens),
                            input_tokens: None,
                            output_tokens: None,
                            cost_usd: Some(result.cost_usd),
                        });

                        // Store selection metadata for RL reward reporting after evaluate_all.
                        // Success/failure is not known until CodeCompiles is evaluated, so we
                        // store here and report in run_tier once the objective state is available.
                        if let Ok(mut guard) = self.last_code_selection.lock() {
                            *guard = Some((result.selected_model.clone(), result.duration_ms));
                        }

                        // Write generated code to disk
                        if let Some(ref raw_path) = result.file_path {
                            let rel_path = self.strip_output_dir_prefix(raw_path);
                            let full_path = PathBuf::from(&self.output_dir).join(rel_path);
                            if let Some(parent) = full_path.parent() {
                                fs::create_dir_all(parent)
                                    .with_context(|| format!("creating directory for {}", rel_path))?;
                            }
                            let clean_content = strip_chat_tokens(&result.content);
                            let clean_content = if self.language == "go" {
                                sanitize_go_source(&clean_content)
                            } else {
                                clean_content
                            };
                            fs::write(&full_path, &clean_content)
                                .with_context(|| format!("writing generated code to {}", rel_path))?;
                            info!(path = %rel_path, bytes = clean_content.len(), "wrote generated code to disk");
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
                                let (target_files, fix_type) = self.infer_fix_target(
                                    tier,
                                    &state.objective,
                                    &blocking_violations,
                                );
                                let fix_input = FixTaskInput {
                                    fix_type,
                                    target_file: target_files.join("\n"),
                                    error_context: blocking_violations.join("\n\n"),
                                    language: self.language.clone(),
                                    output_dir: self.output_dir.clone(),
                                    prior_errors: prior_errors.to_vec(),
                                    project_id: self.project_id.clone(),
                                };
                                let fix_agent = FixAgent::from_env();
                                let fix_start = std::time::Instant::now();
                                match fix_agent.execute(fix_input, effective_model, provider_pref).await {
                                    Ok(fix_result) => {
                                        let fix_dur = fix_start.elapsed().as_millis() as u64;
                                        self.log_agent_performance(
                                            "hex-fixer",
                                            Some(&fix_result.model_used),
                                            Some(fix_result.tokens),
                                            Some(fix_result.input_tokens),
                                            Some(fix_result.output_tokens),
                                            Some(fix_result.cost_usd),
                                            fix_dur,
                                            fix_result.status != "failed",
                                            &Objective::CodeGenerated,
                                        );
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
                            engine.run_feedback_loop(fl).await;
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
                            // Gates failed — invoke FixAgent on gate errors, then retry once.
                            // Re-select model with actual iteration count so the upgrade
                            // threshold (default: 3) fires when the feedback loop exhausted
                            // its iterations — escalating to sonnet/opus automatically.
                            let escalated_selected =
                                self.select_model_for_role("hex-coder", total_iterations as u32);
                            let escalated_model_id = escalated_selected.model_id.clone();
                            let fix_model: Option<&str> = model_override
                                .or_else(|| is_compatible_with_provider(&escalated_model_id, provider_pref)
                                    .then_some(escalated_model_id.as_str()));
                            if escalated_model_id != yaml_model_id {
                                info!(
                                    original = %yaml_model_id,
                                    escalated = %escalated_model_id,
                                    iterations = total_iterations,
                                    "model escalated for fix agent — iteration threshold reached"
                                );
                            }

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
                                let (target_files, fix_type) = self.infer_fix_target(
                                    tier,
                                    &state.objective,
                                    &gate_errors,
                                );
                                let fix_input = FixTaskInput {
                                    fix_type,
                                    target_file: target_files.join("\n"),
                                    error_context: gate_errors.join("\n\n"),
                                    language: self.language.clone(),
                                    output_dir: self.output_dir.clone(),
                                    prior_errors: prior_errors.to_vec(),
                                    project_id: self.project_id.clone(),
                                };
                                let fix_agent = FixAgent::from_env();
                                let fix_start = std::time::Instant::now();
                                match fix_agent.execute(fix_input, fix_model, provider_pref).await {
                                    Ok(fix_result) => {
                                        let fix_dur = fix_start.elapsed().as_millis() as u64;
                                        self.log_agent_performance(
                                            "hex-fixer",
                                            Some(&fix_result.model_used),
                                            Some(fix_result.tokens),
                                            Some(fix_result.input_tokens),
                                            Some(fix_result.output_tokens),
                                            Some(fix_result.cost_usd),
                                            fix_dur,
                                            fix_result.status != "failed",
                                            &Objective::CodeCompiles,
                                        );
                                        info!(
                                            status = %fix_result.status,
                                            file = %fix_result.file_path,
                                            "gate fixer complete — retrying gates"
                                        );
                                        // One retry pass after the fix
                                        let (retry_iters, _, _) = engine.run_feedback_loop(fl).await;
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
                    let effective_model: Option<&str> = model_override
                        .or_else(|| is_compatible_with_provider(&yaml_model_id, provider_pref)
                            .then_some(yaml_model_id.as_str()));
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
                            .execute_step(step, &step_workplan, effective_model, provider_pref, Some(self.output_dir.as_str()))
                            .await
                            .with_context(|| format!("code phase step {} failed", step.id))?;

                        // Store metrics for session audit trail (ADR-2604071300)
                        self.store_dispatch_metrics(AgentMetrics {
                            model: Some(result.model_used.clone()),
                            tokens: Some(result.tokens),
                            input_tokens: None,
                            output_tokens: None,
                            cost_usd: Some(result.cost_usd),
                        });

                        // Store selection metadata for RL reward reporting after evaluate_all.
                        if let Ok(mut guard) = self.last_code_selection.lock() {
                            *guard = Some((result.selected_model.clone(), result.duration_ms));
                        }

                        // Write generated code to disk
                        if let Some(ref raw_path) = result.file_path {
                            let rel_path = self.strip_output_dir_prefix(raw_path);
                            let full_path = PathBuf::from(&self.output_dir).join(rel_path);
                            if let Some(parent) = full_path.parent() {
                                fs::create_dir_all(parent)
                                    .with_context(|| format!("creating directory for {}", rel_path))?;
                            }
                            let clean_content = strip_chat_tokens(&result.content);
                            let clean_content = if self.language == "go" {
                                sanitize_go_source(&clean_content)
                            } else {
                                clean_content
                            };
                            fs::write(&full_path, &clean_content)
                                .with_context(|| format!("writing generated code to {}", rel_path))?;
                            info!(path = %rel_path, bytes = clean_content.len(), "wrote generated code to disk");
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
                                let (target_files, fix_type) = self.infer_fix_target(
                                    tier,
                                    &state.objective,
                                    &blocking_violations,
                                );
                                let fix_input = FixTaskInput {
                                    fix_type,
                                    target_file: target_files.join("\n"),
                                    error_context: blocking_violations.join("\n\n"),
                                    language: self.language.clone(),
                                    output_dir: self.output_dir.clone(),
                                    prior_errors: prior_errors.to_vec(),
                                    project_id: self.project_id.clone(),
                                };
                                let fix_agent = FixAgent::from_env();
                                let fix_start = std::time::Instant::now();
                                match fix_agent.execute(fix_input, effective_model, provider_pref).await {
                                    Ok(fix_result) => {
                                        let fix_dur = fix_start.elapsed().as_millis() as u64;
                                        self.log_agent_performance(
                                            "hex-fixer",
                                            Some(&fix_result.model_used),
                                            Some(fix_result.tokens),
                                            Some(fix_result.input_tokens),
                                            Some(fix_result.output_tokens),
                                            Some(fix_result.cost_usd),
                                            fix_dur,
                                            fix_result.status != "failed",
                                            &Objective::CodeGenerated,
                                        );
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

                // Carry forward issues from the previous review iteration so the
                // reviewer doesn't re-find already-known problems or lose progress.
                let prior_issues: Option<String> = {
                    let review_path = PathBuf::from(&self.output_dir)
                        .join(".hex-review")
                        .join("review-latest.json");
                    fs::read_to_string(&review_path).ok().and_then(|raw| {
                        let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
                        let issues = v["issues"].as_array()?;
                        if issues.is_empty() { return None; }
                        let lines: Vec<String> = issues.iter().filter_map(|i| {
                            let msg = i["message"].as_str().unwrap_or_default();
                            let sev = i["severity"].as_str().unwrap_or("minor");
                            if msg.is_empty() { None } else { Some(format!("[{}] {}", sev, msg)) }
                        }).collect();
                        if lines.is_empty() { None } else {
                            Some(format!("Previously flagged (verify if resolved):\n{}", lines.join("\n")))
                        }
                    })
                };

                let mut context = self.build_reviewer_context(&target_file, workplan_summary);
                context.upstream_output = prior_issues;

                let reviewer_selected = self.select_model_for_role("hex-reviewer", 0);
                let reviewer_model_id = reviewer_selected.model_id.clone();
                // Pass YAML model as a soft preference, not a hard override.
                // This lets the reviewer's internal RL loop select the model
                // (and report outcomes back to the Q-table). Only a CLI --model
                // flag (model_override) is treated as a hard override.
                let result = agent
                    .execute_with_preference(
                        &context,
                        model_override,
                        Some(&reviewer_model_id),
                        provider_pref,
                    )
                    .await
                    .context("reviewer agent failed")?;
                // Store metrics for execute_agent_tracked to log once
                self.store_dispatch_metrics(AgentMetrics {
                    model: Some(result.model_used.clone()),
                    tokens: Some(result.tokens),
                    input_tokens: Some(result.input_tokens),
                    output_tokens: Some(result.output_tokens),
                    cost_usd: Some(result.cost_usd),
                });
                if result.reviewer_skipped {
                    warn!(
                        tier,
                        "reviewer produced no valid JSON after 3 attempts — \
                         synthetic PASS written; review was skipped for this tier"
                    );
                }
                // Write review output so evaluate_review_passes can pick it up
                let review_dir = PathBuf::from(&self.output_dir).join(".hex-review");
                let _ = fs::create_dir_all(&review_dir);

                // For standalone projects, downgrade hex-architecture false positives.
                // Free models routinely flag "no composition root", "port interface contract",
                // etc. on standalone CLIs despite the standalone_note instruction.  These
                // are never real issues for a CLI/script project.
                let is_standalone = self.port_files().is_empty();
                let hex_false_positive_patterns = [
                    "composition root",
                    "port interface",
                    "domain layer",
                    "hexagonal",
                    "adapter implementation",
                    "hex boundary",
                    "composition-root",
                ];
                let filtered_issues: Vec<_> = result.issues.iter().map(|i| {
                    let mut severity = i.severity.clone();
                    if is_standalone && (severity == "critical" || severity == "major") {
                        let desc_lower = i.description.to_lowercase();
                        if hex_false_positive_patterns.iter().any(|p| desc_lower.contains(p)) {
                            severity = "minor".to_string();
                        }
                    }
                    (severity, i)
                }).collect();

                // Recompute verdict: PASS if no critical/high issues remain after filtering.
                let filtered_verdict = if filtered_issues.iter().any(|(sev, _)| sev == "critical" || sev == "high") {
                    result.verdict.clone()
                } else {
                    "PASS".to_string()
                };

                let review_json = serde_json::json!({
                    "verdict": filtered_verdict,
                    "reviewer_skipped": result.reviewer_skipped,
                    "issues": filtered_issues.iter().map(|(sev, i)| serde_json::json!({
                        "severity": sev,
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
                self.store_dispatch_metrics(AgentMetrics {
                    model: Some(result.model_used.clone()),
                    tokens: Some(result.tokens),
                    input_tokens: Some(result.input_tokens),
                    output_tokens: Some(result.output_tokens),
                    cost_usd: Some(result.cost_usd),
                });
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
                self.store_dispatch_metrics(AgentMetrics {
                    model: Some(result.model_used.clone()),
                    tokens: Some(result.tokens),
                    input_tokens: None,
                    output_tokens: None,
                    cost_usd: Some(result.cost_usd),
                });
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
                self.store_dispatch_metrics(AgentMetrics {
                    model: Some(result.model_used.clone()),
                    tokens: Some(result.tokens),
                    input_tokens: None,
                    output_tokens: None,
                    cost_usd: Some(result.cost_usd),
                });
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
                let (target_files, fix_type) =
                    self.infer_fix_target(tier, &state.objective, &state.blocking_issues);
                // Include workplan summary so the fixer knows WHAT the code should do,
                // not just what the issues are. This prevents the fixer from "fixing"
                // review issues without understanding the intended behaviour.
                // For Rust test failures, prepend the exact binary name so the fixer
                // never guesses a wrong CARGO_BIN_EXE_<name> macro argument.
                let binary_name_prefix = if self.language == "rust"
                    && state.objective == Objective::TestsPass
                {
                    self.rust_binary_name()
                        .map(|n| format!("The binary name from Cargo.toml is EXACTLY `{}`.\n\n", n))
                        .unwrap_or_default()
                } else {
                    String::new()
                };

                let error_context = if workplan_summary.is_empty() {
                    format!("{}{}", binary_name_prefix, state.blocking_issues.join("\n"))
                } else {
                    format!(
                        "{}FEATURE OBJECTIVE (what this code must do):\n{}\n\nISSUES TO FIX:\n{}",
                        binary_name_prefix,
                        workplan_summary,
                        state.blocking_issues.join("\n")
                    )
                };
                let input = FixTaskInput {
                    fix_type,
                    target_file: target_files.join("\n"),
                    error_context,
                    language: self.language.clone(),
                    output_dir: self.output_dir.clone(),
                    prior_errors: prior_errors.to_vec(),
                    project_id: self.project_id.clone(),
                };
                let yaml_selected = self.select_model_for_role("hex-fixer", 0);
                let yaml_model_id = yaml_selected.model_id.clone();
                info!(model = %yaml_model_id, source = %yaml_selected.source, "selected model for fix");
                let fixer_model: Option<&str> = model_override
                    .or_else(|| is_compatible_with_provider(&yaml_model_id, provider_pref)
                        .then_some(yaml_model_id.as_str()));
                let result = agent
                    .execute(input, fixer_model, provider_pref)
                    .await
                    .context("fix agent failed")?;
                self.store_dispatch_metrics(AgentMetrics {
                    model: Some(result.model_used.clone()),
                    tokens: Some(result.tokens),
                    input_tokens: Some(result.input_tokens),
                    output_tokens: Some(result.output_tokens),
                    cost_usd: Some(result.cost_usd),
                });
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
        if let Some((path, _)) = files.first() {
            return path.clone();
        }
        // For Go single-binary projects, main.go is always the target
        if self.language == "go" {
            return "main.go".to_string();
        }
        let ext = match self.language.as_str() {
            "rust" => "rs",
            _ => "ts",
        };
        format!("src/unknown-tier{}.{}", tier, ext)
    }

    /// Infer fix target files and fix_type from the objective and blocking issues.
    /// Returns all unique file paths found across all blocking issues (not just the first).
    fn infer_fix_target(
        &self,
        tier: u32,
        objective: &Objective,
        blocking_issues: &[String],
    ) -> (Vec<String>, String) {
        let abs_output = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(&self.output_dir);

        // Collect unique file paths from ALL blocking issues.
        // Matches patterns like:
        //   "path/to/file.rs:42:5: error ..."   (tsc / rustc colon-separated)
        //   " --> src/main.rs:10:3"              (rustc arrow notation)
        let mut seen = std::collections::HashSet::new();
        let mut target_files: Vec<String> = Vec::new();

        let resolve_candidate = |candidate: &str| -> Option<String> {
            // Skip cargo progress lines like "Checking crate v0.1.0 (/path)" —
            // they match the dot+slash heuristic but are not file paths.
            if candidate.contains(' ') {
                return None;
            }
            // Strip TypeScript (line,col) suffix: "src/foo.ts(1,21)" → "src/foo.ts"
            let candidate = if let Some(paren_pos) = candidate.rfind('(') {
                let suffix = &candidate[paren_pos..];
                // Only strip if it looks like (digits,digits)
                let inner = suffix.trim_start_matches('(').trim_end_matches(')');
                if inner.chars().all(|c| c.is_ascii_digit() || c == ',') {
                    &candidate[..paren_pos]
                } else {
                    candidate
                }
            } else {
                candidate
            };
            // Also accept bare filenames like "main.go" or "main.rs" with no path separator
            let is_bare_source_file = candidate.contains('.')
                && !candidate.contains('/')
                && !candidate.contains('\\')
                && (candidate.ends_with(".go") || candidate.ends_with(".rs") || candidate.ends_with(".ts"));
            if is_bare_source_file {
                // Return the bare filename — the caller prepends output_dir
                return Some(candidate.to_string());
            }
            if candidate.contains('.') && (candidate.contains('/') || candidate.contains('\\')) {
                // Normalize to a path relative to output_dir so that
                // PathBuf::join never silently discards the base (which happens
                // when the input is already absolute). Two cases:
                //   • Absolute path (from tsc/eslint with rootDir mismatch):
                //     strip the abs_output prefix → get the relative portion.
                //   • Relative path that echoes output_dir prefix:
                //     strip_output_dir_prefix handles this.
                let relative: &str = if candidate.starts_with('/') || candidate.starts_with('\\') {
                    let abs_str = abs_output.to_str().unwrap_or("");
                    let prefix = format!("{}/", abs_str);
                    // Path is outside output_dir — not a project file; skip.
                    candidate.strip_prefix(prefix.as_str())?
                } else {
                    self.strip_output_dir_prefix(candidate)
                };

                let full_path = abs_output.join(relative);
                if full_path.exists() {
                    Some(full_path.display().to_string())
                } else {
                    // File doesn't exist yet — return relative so fixer can create it
                    let abs_relative = PathBuf::from(&self.output_dir)
                        .join(relative)
                        .display()
                        .to_string();
                    Some(abs_relative)
                }
            } else {
                None
            }
        };

        for issue in blocking_issues {
            // Pattern 1: " --> src/foo.rs:10:3" (rustc arrow notation)
            for line in issue.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("--> ") {
                    let file_part = rest.split(':').next().unwrap_or("").trim();
                    if let Some(resolved) = resolve_candidate(file_part) {
                        if seen.insert(resolved.clone()) {
                            target_files.push(resolved);
                        }
                        continue;
                    }
                }
                // Pattern 2: "path/to/file.rs:42:5: error ..." (colon-separated first token)
                let parts: Vec<&str> = trimmed.splitn(2, ':').collect();
                let candidate = parts.first().unwrap_or(&"").trim();
                if let Some(resolved) = resolve_candidate(candidate) {
                    if seen.insert(resolved.clone()) {
                        target_files.push(resolved);
                    }
                }
            }
        }

        // Fall back to the first source file for the tier if nothing was found.
        if target_files.is_empty() {
            let fallback = self.first_source_file_for_tier(tier);
            let full_path = if Path::new(&fallback).is_absolute() {
                fallback
            } else {
                PathBuf::from(&self.output_dir)
                    .join(&fallback)
                    .display()
                    .to_string()
            };
            target_files.push(full_path);
        }

        let fix_type = match objective {
            Objective::CodeCompiles => "compile".to_string(),
            Objective::TestsPass => "test".to_string(),
            Objective::ArchitectureGradeA => "violation".to_string(),
            Objective::ReviewPasses | Objective::UxReviewPasses => "violation".to_string(),
            _ => "compile".to_string(),
        };

        (target_files, fix_type)
    }

    /// Auto-patch `Cargo.toml` by adding missing crate dependencies inferred from
    /// compile error messages (e.g. "use of undeclared crate or module `clap`").
    /// This avoids expensive LLM calls for the common "missing dependency" error.
    fn auto_patch_cargo_toml(&self, blocking_issues: &[String]) {
        // Known crates and their Cargo.toml entries
        let known_crates: &[(&str, &str)] = &[
            ("axum", r#"axum = "0.8""#),
            ("tokio", r#"tokio = { version = "1", features = ["full"] }"#),
            ("clap", r#"clap = { version = "4", features = ["derive"] }"#),
            ("serde", r#"serde = { version = "1", features = ["derive"] }"#),
            ("serde_json", r#"serde_json = "1""#),
            ("anyhow", r#"anyhow = "1""#),
            ("thiserror", r#"thiserror = "1""#),
            ("tracing", r#"tracing = "0.1""#),
            ("tracing_subscriber", r#"tracing-subscriber = "0.3""#),
            ("reqwest", r#"reqwest = { version = "0.12", features = ["json"] }"#),
            ("tower", r#"tower = "0.5""#),
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
                || combined_errors.contains(&format!("unlinked crate `{}`", crate_name))
                || combined_errors.contains(&format!("use of undeclared crate `{}`", crate_name))
                || combined_errors.contains(&format!("unresolved import `{}`", crate_name))
                || combined_errors.contains(&format!("`{}` is not", crate_name));
            // Check if it's already a dependency (not just part of package name or path).
            // A dep line starts at the beginning of a line: `crate_name =` or `crate_name.`
            let already_dep = cargo_content.lines().any(|l| {
                let t = l.trim_start();
                t.starts_with(&format!("{} =", crate_name))
                    || t.starts_with(&format!("{}.workspace", crate_name))
            });
            if mentioned && !already_dep {
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

        // Fallback: default model (use specific free model, not dynamic openrouter/free routing)
        crate::pipeline::model_selection::SelectedModel {
            model_id: crate::pipeline::model_selection::default_model_for_general().to_string(),
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

/// Structured result stored by a Docker worker in hexflo memory under `{task_id}:result`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct WorkerResult {
    pub file_path: String,
    #[serde(default)]
    pub content_len: usize,
    #[serde(default)]
    pub compile_pass: bool,
    #[serde(default)]
    pub tests_pass: bool,
    #[serde(default)]
    pub test_output: String,
    /// Inference model used (populated by reviewer/tester workers).
    #[serde(default)]
    pub model: Option<String>,
    /// Total tokens (input + output).
    #[serde(default)]
    pub tokens: Option<u64>,
    /// Prompt tokens (context window usage).
    #[serde(default)]
    pub input_tokens: Option<u64>,
    /// Completion tokens.
    #[serde(default)]
    pub output_tokens: Option<u64>,
    /// Cost in USD.
    #[serde(default)]
    pub cost_usd: Option<f64>,
}

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
    /// Pipeline halted: max iterations exhausted; inbox notification was sent.
    Halted {
        reason: String,
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
            TierResult::Halted { states, .. } => states,
        }
    }

    /// Number of iterations used.
    pub fn iterations(&self) -> u32 {
        match self {
            TierResult::AllPassed { iterations, .. } => *iterations,
            TierResult::MaxIterations { iterations, .. } => *iterations,
            TierResult::Halted { .. } => 0,
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

        // Parse test counts from detail string.
        // Formats: "3/5 tests passed" (failed run), "3 tests passed" (all pass), "1 tests passed"
        let (tests_passed, tests_failed) = test_state
            .and_then(|s| {
                let detail = &s.detail;
                if let Some(slash) = detail.find('/') {
                    // "N/total tests ..." — extract passed and total
                    let passed = detail[..slash].trim().parse::<u32>().ok()?;
                    let total_str = detail[slash + 1..].split_whitespace().next()?;
                    let total = total_str.parse::<u32>().ok()?;
                    Some((passed, total.saturating_sub(passed)))
                } else {
                    // "N tests passed" — all tests passed, failed = 0
                    let first = detail.split_whitespace().next()?;
                    let passed = first.parse::<u32>().ok()?;
                    Some((passed, 0))
                }
            })
            .unwrap_or((0, 0));

        // Parse violation count from architecture detail (e.g. "Score 87/100" or "2 violations")
        let violations_found = arch_state
            .map(|s| {
                if s.met { return 0; }
                // Try to extract number from blocking_issues count
                s.blocking_issues.len() as u32
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

/// Strip qwen/llama chat special tokens from LLM output before writing to disk.
/// Models like qwen3.5 sometimes emit `<|endoftext|>`, `<|im_start|>`, etc.
/// after their answer. Truncate at the first such marker.
fn strip_chat_tokens(s: &str) -> String {
    if let Some(pos) = s.find("<|") {
        s[..pos].trim_end().to_string()
    } else {
        s.trim_end().to_string()
    }
}

/// Fix common LLM Go formatting bugs.
///
/// LLMs sometimes output `package mainimport (` when they mean:
/// ```text
/// package main
///
/// import (
/// ```
fn sanitize_go_source(s: &str) -> String {
    // Fix "package mainimport" → "package main\n\nimport"
    let s = s.replace("package mainimport (", "package main\n\nimport (");
    
    // Fix "package main\nimport" → "package main\n\nimport" (missing blank line — valid but noisy)
    s.replace("package mainimport(", "package main\n\nimport(")
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

    /// S05: load_strategy entries with `load: on_demand` are excluded from the
    /// initial context build; entries without `load` (or with `load: startup`) are included.
    #[test]
    fn test_build_context_from_yaml_skips_on_demand_entries() {
        use crate::pipeline::agent_def::{
            AgentDefinition, ContextConfig, LoadStrategyEntry, TokenBudget,
        };
        use std::collections::HashMap;
        use tempfile::TempDir;

        // Create a temp project dir with two files
        let tmp = TempDir::new().expect("tempdir");
        let ports_dir = tmp.path().join("ports");
        std::fs::create_dir_all(&ports_dir).expect("mkdir ports");

        let port_file = ports_dir.join("iport.ts");
        std::fs::write(&port_file, "export interface IPort { doThing(): void; }").expect("write port");

        let edit_file = tmp.path().join("active_edit.ts");
        std::fs::write(&edit_file, "SECRET_ON_DEMAND_CONTENT").expect("write edit file");

        // Build an AgentDefinition with one normal entry and one on_demand entry
        let agent_def = AgentDefinition {
            name: "test-coder".into(),
            agent_type: "coder".into(),
            version: "1.0.0".into(),
            description: String::new(),
            model: Default::default(),
            context: Some(ContextConfig {
                load_strategy: vec![
                    LoadStrategyEntry {
                        level: "L1".into(),
                        scope: "ports/**".into(),
                        purpose: None,
                        load: None, // startup (default) — should be loaded
                    },
                    LoadStrategyEntry {
                        level: "L3".into(),
                        scope: "active_edit.ts".into(),
                        purpose: None,
                        load: Some("on_demand".into()), // must be skipped
                    },
                ],
                load_on_start: vec![],
                load_on_demand: vec![],
                token_budget: Some(TokenBudget {
                    max: 100000,
                    reserved_response: 20000,
                    allocation: HashMap::new(),
                    pressure: None,
                }),
            }),
            constraints: vec![],
            tools: None,
            inputs: HashMap::new(),
            outputs: HashMap::new(),
            workflow: None,
            escalation: None,
            quality_thresholds: None,
        };

        let sup = Supervisor::new(tmp.path().to_str().unwrap(), "typescript");
        let ctx = sup.build_context_from_yaml(&agent_def, "test step", 1, None, None);

        // The port file (normal entry) should appear in context
        let all_content: String = ctx
            .source_files
            .iter()
            .chain(ctx.port_interfaces.iter())
            .map(|(_, c)| c.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            all_content.contains("IPort"),
            "normal L1 entry should be loaded into context"
        );
        assert!(
            !all_content.contains("SECRET_ON_DEMAND_CONTENT"),
            "on_demand entry must not appear in initial context build"
        );
    }

    /// P6 integration: build_context_from_yaml with a pressure block emits
    /// all four pressure metadata keys into the returned AgentContext.
    #[test]
    fn test_pressure_metadata_emitted_from_yaml() {
        use crate::pipeline::agent_def::{
            AgentDefinition, ContextConfig, LoadStrategyEntry, PressureConfig, TokenBudget,
        };
        use std::collections::HashMap;

        let agent_def = AgentDefinition {
            name: "test-coder".into(),
            agent_type: "coder".into(),
            version: "1.0.0".into(),
            description: String::new(),
            model: Default::default(),
            context: Some(ContextConfig {
                load_strategy: vec![
                    LoadStrategyEntry {
                        level: "L1".into(),
                        scope: "ports/**".into(),
                        purpose: None,
                        load: None,
                    },
                ],
                load_on_start: vec![],
                load_on_demand: vec![],
                token_budget: Some(TokenBudget {
                    max: 100_000,
                    reserved_response: 20_000,
                    allocation: HashMap::new(),
                    pressure: Some(PressureConfig {
                        warn_at_pct: 70,
                        compress_at_pct: 80,
                        block_at_pct: 90,
                        relief: "summarize_history".into(),
                    }),
                }),
            }),
            constraints: vec![],
            tools: None,
            inputs: HashMap::new(),
            outputs: HashMap::new(),
            workflow: None,
            escalation: None,
            quality_thresholds: None,
        };

        let sup = Supervisor::new("/tmp/nonexistent", "typescript");
        let ctx = sup.build_context_from_yaml(&agent_def, "step", 1, None, None);

        assert_eq!(ctx.metadata.get("pressure_warn_pct").map(String::as_str), Some("70"));
        assert_eq!(ctx.metadata.get("pressure_compress_pct").map(String::as_str), Some("80"));
        assert_eq!(ctx.metadata.get("pressure_block_pct").map(String::as_str), Some("90"));
        assert_eq!(ctx.metadata.get("pressure_relief").map(String::as_str), Some("summarize_history"));
    }

    /// P6 integration: hex-coder.yml round-trip — pressure metadata present in
    /// context built from the embedded YAML.
    #[test]
    fn test_hex_coder_yaml_pressure_metadata_round_trip() {
        use crate::pipeline::agent_def::AgentDefinition;

        let def = AgentDefinition::load("hex-coder").expect("hex-coder.yml must parse");
        let sup = Supervisor::new("/tmp/nonexistent", "typescript");
        let ctx = sup.build_context_from_yaml(&def, "implement IPort", 1, None, None);

        // Pressure metadata from hex-coder.yml (warn=70, compress=80, block=90)
        assert_eq!(ctx.metadata.get("pressure_warn_pct").map(String::as_str), Some("70"),
            "hex-coder pressure warn threshold should be in context metadata");
        assert_eq!(ctx.metadata.get("pressure_relief").map(String::as_str), Some("summarize_history"),
            "hex-coder pressure relief strategy should be in context metadata");
        // Budget max is also present
        assert_eq!(ctx.metadata.get("token_budget_max").map(String::as_str), Some("100000"));
    }
}
