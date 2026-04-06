//! Build port — multi-language compile/lint/test dispatch (ADR-018, ADR-2604061000).
//!
//! Defines the contract for running language-specific build toolchains
//! against target projects. Implementations dispatch to tsc, go build,
//! cargo check, etc. based on detected project language.

/// Language-specific build toolchain configuration.
#[derive(Debug, Clone)]
pub struct BuildToolchain {
    /// Language identifier: "typescript", "go", "rust"
    pub language: String,
    /// Compile command (e.g., "npx tsc --noEmit", "go build ./...", "cargo check")
    pub compile_cmd: String,
    /// Lint command (e.g., "npx eslint .", "golangci-lint run", "cargo clippy")
    pub lint_cmd: String,
    /// Test command (e.g., "bun test", "go test ./...", "cargo test")
    pub test_cmd: String,
}

/// Result of a build operation (compile, lint, or test).
#[derive(Debug, Clone)]
pub struct BuildOutput {
    pub success: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub diagnostics: Vec<Diagnostic>,
}

/// A single diagnostic message from a build tool.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub severity: DiagnosticSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

/// Port for executing language-specific build operations on target projects.
pub trait IBuildPort: Send + Sync {
    /// Detect the project language and return its toolchain configuration.
    fn detect_toolchain(&self, project_dir: &str) -> Option<BuildToolchain>;

    /// Run the compile step for the detected language.
    fn compile(&self, project_dir: &str) -> BuildOutput;

    /// Run the lint step for the detected language.
    fn lint(&self, project_dir: &str) -> BuildOutput;

    /// Run the test step for the detected language.
    fn test(&self, project_dir: &str) -> BuildOutput;
}
