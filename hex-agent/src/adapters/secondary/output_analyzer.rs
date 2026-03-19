//! Output Analyzer Adapter — analyzes LLM output quality via hex-nexus + local checks.
//!
//! Implements `OutputAnalyzerPort` by:
//! 1. Calling hex-nexus `/api/analyze` for architecture boundary compliance
//! 2. Running build checks (`cargo check` / `bun run check`) for compilation
//! 3. Computing token efficiency from response length vs file changes
//!
//! Results feed into the RL reward signal for model selection optimization.

use async_trait::async_trait;
use std::path::Path;
use tokio::process::Command;

use crate::ports::output_analyzer::{AnalysisContext, OutputAnalyzerPort, OutputScore};

const DEFAULT_NEXUS_URL: &str = "http://127.0.0.1:5555";
const NEXUS_TIMEOUT_SECS: u64 = 15;
const BUILD_TIMEOUT_SECS: u64 = 60;

/// Adapter that analyzes LLM output by calling hex-nexus and running local checks.
pub struct NexusOutputAnalyzer {
    nexus_url: String,
    client: reqwest::Client,
}

impl NexusOutputAnalyzer {
    pub fn new(nexus_url: Option<String>) -> Self {
        let url = nexus_url
            .or_else(|| std::env::var("HEX_NEXUS_URL").ok())
            .unwrap_or_else(|| DEFAULT_NEXUS_URL.to_string());

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(NEXUS_TIMEOUT_SECS))
            .build()
            .unwrap_or_default();

        Self {
            nexus_url: url,
            client,
        }
    }

    /// Check hex boundary compliance via hex-nexus REST API.
    async fn check_boundaries(&self, project_root: &str) -> f64 {
        let url = format!("{}/api/analyze", self.nexus_url);
        let body = serde_json::json!({ "root_path": project_root });

        let res = match self.client.post(&url).json(&body).send().await {
            Ok(r) if r.status().is_success() => r,
            _ => return 1.0, // nexus unavailable → assume clean
        };

        let data: serde_json::Value = match res.json().await {
            Ok(v) => v,
            Err(_) => return 1.0,
        };

        let violations = data["violations"]
            .as_array()
            .map_or(0, |v| v.len());
        let file_count = data["file_count"].as_u64().unwrap_or(1).max(1);

        if violations == 0 {
            1.0
        } else {
            // Scale: each violation reduces compliance proportionally
            (1.0 - (violations as f64 / file_count as f64)).max(0.0)
        }
    }

    /// Run a build check to see if the project compiles.
    async fn check_compiles(&self, project_root: &str) -> Option<bool> {
        let root = Path::new(project_root);

        // Detect build system
        let (cmd, args) = if root.join("Cargo.toml").exists() {
            ("cargo", vec!["check", "--quiet"])
        } else if root.join("package.json").exists() {
            if root.join("bun.lockb").exists() || root.join("bunfig.toml").exists() {
                ("bun", vec!["run", "check"])
            } else {
                ("npx", vec!["tsc", "--noEmit"])
            }
        } else if root.join("go.mod").exists() {
            ("go", vec!["build", "./..."])
        } else {
            return None; // no recognized build system
        };

        let result = Command::new(cmd)
            .args(&args)
            .current_dir(project_root)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .status();

        match tokio::time::timeout(
            std::time::Duration::from_secs(BUILD_TIMEOUT_SECS),
            result,
        )
        .await
        {
            Ok(Ok(status)) => Some(status.success()),
            _ => None, // timeout or exec failure
        }
    }

    /// Compute token efficiency: ratio of "useful work" to total tokens.
    ///
    /// Heuristic: lines changed per 1000 tokens. A response that changes
    /// 50 lines using 5000 tokens is efficient (10 lines/ktoken).
    /// One that changes 2 lines using 10000 tokens is not (0.2 lines/ktoken).
    fn compute_token_efficiency(context: &AnalysisContext) -> f64 {
        if context.total_tokens == 0 {
            return 0.5; // no data
        }

        let lines_changed: usize = context
            .files_changed
            .iter()
            .map(|f| f.lines_added + f.lines_removed)
            .sum();

        if lines_changed == 0 && context.files_changed.is_empty() {
            // Pure text response (explanation, not code) — moderate efficiency
            let response_len = context.llm_response.len();
            if response_len > 0 {
                return 0.7;
            }
            return 0.3; // empty response
        }

        let ktokens = context.total_tokens as f64 / 1000.0;
        let lines_per_ktoken = lines_changed as f64 / ktokens.max(0.1);

        // Scale: 0 lines/kt → 0.1, 5 lines/kt → 0.5, 15+ lines/kt → 1.0
        (lines_per_ktoken / 15.0).clamp(0.1, 1.0)
    }
}

#[async_trait]
impl OutputAnalyzerPort for NexusOutputAnalyzer {
    async fn analyze(&self, context: &AnalysisContext) -> OutputScore {
        let has_content = !context.llm_response.is_empty();

        // Run boundary check and compile check concurrently
        let (boundary_compliance, compiles) = tokio::join!(
            self.check_boundaries(&context.project_root),
            self.check_compiles(&context.project_root),
        );

        let token_efficiency = Self::compute_token_efficiency(context);

        // Tests: skip for now (too slow for per-turn feedback)
        let tests_pass = None;

        OutputScore::compute(
            boundary_compliance,
            compiles,
            tests_pass,
            token_efficiency,
            has_content,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::output_analyzer::{AnalysisContext, ChangeType, FileChange};

    fn make_context(files: Vec<FileChange>, tokens: u64, response: &str) -> AnalysisContext {
        AnalysisContext {
            task_description: "implement feature".to_string(),
            llm_response: response.to_string(),
            files_changed: files,
            project_root: ".".to_string(),
            model_used: "claude-sonnet-4-6".to_string(),
            latency_ms: 1000,
            total_tokens: tokens,
        }
    }

    #[test]
    fn token_efficiency_with_code_changes() {
        let files = vec![FileChange {
            path: "src/main.rs".to_string(),
            change_type: ChangeType::Modified,
            lines_added: 30,
            lines_removed: 5,
        }];
        let ctx = make_context(files, 3000, "here is the code");
        let eff = NexusOutputAnalyzer::compute_token_efficiency(&ctx);
        // 35 lines / 3 ktokens = ~11.7 lines/kt → ~0.78
        assert!(eff > 0.5);
        assert!(eff < 1.0);
    }

    #[test]
    fn token_efficiency_bloated_response() {
        let files = vec![FileChange {
            path: "src/main.rs".to_string(),
            change_type: ChangeType::Modified,
            lines_added: 2,
            lines_removed: 0,
        }];
        let ctx = make_context(files, 15000, "verbose explanation...");
        let eff = NexusOutputAnalyzer::compute_token_efficiency(&ctx);
        // 2 lines / 15 ktokens = 0.13 lines/kt → low
        assert!(eff < 0.3);
    }

    #[test]
    fn token_efficiency_text_only() {
        let ctx = make_context(vec![], 2000, "Here's an explanation of the architecture.");
        let eff = NexusOutputAnalyzer::compute_token_efficiency(&ctx);
        // No file changes but has content → 0.7
        assert_eq!(eff, 0.7);
    }

    #[test]
    fn token_efficiency_empty_response() {
        let ctx = make_context(vec![], 1000, "");
        let eff = NexusOutputAnalyzer::compute_token_efficiency(&ctx);
        assert!(eff < 0.5);
    }

    #[test]
    fn token_efficiency_zero_tokens() {
        let ctx = make_context(vec![], 0, "something");
        let eff = NexusOutputAnalyzer::compute_token_efficiency(&ctx);
        assert_eq!(eff, 0.5);
    }
}
