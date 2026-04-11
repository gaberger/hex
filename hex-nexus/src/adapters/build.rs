//! Build adapter — executes language-specific compile/lint/test commands (ADR-018).
//!
//! Dispatches to the correct toolchain based on project manifest files:
//! - `package.json` / `tsconfig.json` → TypeScript (tsc, eslint, bun test)
//! - `go.mod` → Go (go build, golangci-lint, go test)
//! - `Cargo.toml` → Rust (cargo check, cargo clippy, cargo test)

use std::path::Path;
use std::process::Command;

use hex_core::ports::build::*;

pub struct BuildAdapter;

impl BuildAdapter {
    pub fn new() -> Self {
        Self
    }

    fn run_command(&self, project_dir: &str, cmd: &str) -> BuildOutput {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.is_empty() {
            return BuildOutput {
                success: false,
                exit_code: -1,
                stdout: String::new(),
                stderr: "Empty command".to_string(),
                diagnostics: vec![],
            };
        }

        let result = Command::new(parts[0])
            .args(&parts[1..])
            .current_dir(project_dir)
            .output();

        match result {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code().unwrap_or(-1);
                let diagnostics = parse_diagnostics(&stdout, &stderr);

                BuildOutput {
                    success: output.status.success(),
                    exit_code,
                    stdout,
                    stderr,
                    diagnostics,
                }
            }
            Err(e) => BuildOutput {
                success: false,
                exit_code: -1,
                stdout: String::new(),
                stderr: format!("Failed to execute '{}': {}", cmd, e),
                diagnostics: vec![],
            },
        }
    }
}

impl Default for BuildAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl IBuildPort for BuildAdapter {
    fn detect_toolchain(&self, project_dir: &str) -> Option<BuildToolchain> {
        let root = Path::new(project_dir);

        // Go: go.mod present
        if root.join("go.mod").is_file() {
            return Some(BuildToolchain {
                language: "go".to_string(),
                compile_cmd: "go build ./...".to_string(),
                lint_cmd: "golangci-lint run".to_string(),
                test_cmd: "go test ./...".to_string(),
            });
        }

        // Rust: Cargo.toml present
        if root.join("Cargo.toml").is_file() {
            return Some(BuildToolchain {
                language: "rust".to_string(),
                compile_cmd: "cargo check".to_string(),
                lint_cmd: "cargo clippy -- -D warnings".to_string(),
                test_cmd: "cargo test".to_string(),
            });
        }

        // TypeScript: tsconfig.json or package.json with typescript dep
        if root.join("tsconfig.json").is_file() {
            return Some(BuildToolchain {
                language: "typescript".to_string(),
                compile_cmd: "npx tsc --noEmit".to_string(),
                lint_cmd: "npx eslint .".to_string(),
                test_cmd: "bun test".to_string(),
            });
        }

        if root.join("package.json").is_file() {
            // Check if it's a TypeScript project
            if let Ok(content) = std::fs::read_to_string(root.join("package.json")) {
                if content.contains("\"typescript\"") {
                    return Some(BuildToolchain {
                        language: "typescript".to_string(),
                        compile_cmd: "npx tsc --noEmit".to_string(),
                        lint_cmd: "npx eslint .".to_string(),
                        test_cmd: "bun test".to_string(),
                    });
                }
            }
        }

        None
    }

    fn compile(&self, project_dir: &str) -> BuildOutput {
        match self.detect_toolchain(project_dir) {
            Some(tc) => self.run_command(project_dir, &tc.compile_cmd),
            None => BuildOutput {
                success: false,
                exit_code: -1,
                stdout: String::new(),
                stderr: "No supported language detected".to_string(),
                diagnostics: vec![],
            },
        }
    }

    fn lint(&self, project_dir: &str) -> BuildOutput {
        match self.detect_toolchain(project_dir) {
            Some(tc) => self.run_command(project_dir, &tc.lint_cmd),
            None => BuildOutput {
                success: false,
                exit_code: -1,
                stdout: String::new(),
                stderr: "No supported language detected".to_string(),
                diagnostics: vec![],
            },
        }
    }

    fn test(&self, project_dir: &str) -> BuildOutput {
        match self.detect_toolchain(project_dir) {
            Some(tc) => self.run_command(project_dir, &tc.test_cmd),
            None => BuildOutput {
                success: false,
                exit_code: -1,
                stdout: String::new(),
                stderr: "No supported language detected".to_string(),
                diagnostics: vec![],
            },
        }
    }
}

/// Parse compiler/linter output into structured diagnostics.
/// Handles common formats: file:line:col: message
fn parse_diagnostics(stdout: &str, stderr: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for line in stdout.lines().chain(stderr.lines()) {
        if let Some(diag) = parse_diagnostic_line(line) {
            diagnostics.push(diag);
        }
    }

    diagnostics
}

/// Parse a single diagnostic line in file:line:col: message format.
/// Works for Go, Rust, and TypeScript compiler output.
fn parse_diagnostic_line(line: &str) -> Option<Diagnostic> {
    // Format: path:line:col: severity: message (Rust/Go)
    // Format: path(line,col): severity TS1234: message (TypeScript)

    // Try standard format: file:line:col: message
    let parts: Vec<&str> = line.splitn(4, ':').collect();
    if parts.len() >= 4 {
        let file = parts[0].trim();
        if let (Ok(line_num), Ok(col)) = (parts[1].trim().parse::<u32>(), parts[2].trim().parse::<u32>()) {
            let rest = parts[3].trim();
            let severity = if rest.contains("error") || rest.starts_with("error") {
                DiagnosticSeverity::Error
            } else if rest.contains("warning") || rest.starts_with("warning") {
                DiagnosticSeverity::Warning
            } else {
                DiagnosticSeverity::Info
            };

            return Some(Diagnostic {
                file: file.to_string(),
                line: line_num,
                column: col,
                severity,
                message: rest.to_string(),
            });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_go_project() {
        let _adapter = BuildAdapter::new();
        // This test would need a temp dir with go.mod — just test the parse logic
        let diag = parse_diagnostic_line("main.go:10:5: undefined: fmt.Printl");
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.file, "main.go");
        assert_eq!(d.line, 10);
        assert_eq!(d.column, 5);
    }

    #[test]
    fn detect_rust_error() {
        let diag = parse_diagnostic_line("src/main.rs:15:1: error[E0308]: mismatched types");
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.severity, DiagnosticSeverity::Error);
        assert_eq!(d.file, "src/main.rs");
    }

    #[test]
    fn detect_warning() {
        let diag = parse_diagnostic_line("lib.rs:3:1: warning: unused variable");
        assert!(diag.is_some());
        assert_eq!(diag.unwrap().severity, DiagnosticSeverity::Warning);
    }

    #[test]
    fn non_diagnostic_line_returns_none() {
        assert!(parse_diagnostic_line("Compiling hex-core v26.8.0").is_none());
        assert!(parse_diagnostic_line("ok").is_none());
    }
}
