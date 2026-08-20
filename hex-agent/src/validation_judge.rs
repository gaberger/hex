use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub enum ValidationResult {
    Pass,
    Fail { reason: String },
    NeedsHumanReview { reason: String },
}

pub struct ValidationJudge;

impl ValidationJudge {
    pub fn validate_task(
        task_id: &str,
        files: &[String],
        evidence: &[String],
        project_dir: &Path,
        background: bool,
    ) -> Result<ValidationResult> {
        if !background {
            eprintln!("    validating task {}...", task_id);
        }

        // Step 1: Verify all declared files exist
        for file in files {
            let file_path = project_dir.join(file);
            if !file_path.exists() {
                return Ok(ValidationResult::Fail {
                    reason: format!("Declared file not found: {}", file),
                });
            }

            // Check if file is just a TODO stub
            if let Ok(content) = std::fs::read_to_string(&file_path) {
                let is_stub = content.contains("TODO: implement")
                    || content.contains("// TODO")
                    || content.trim().is_empty();

                if is_stub {
                    return Ok(ValidationResult::Fail {
                        reason: format!("File {} is a TODO stub, not real code", file),
                    });
                }
            }
        }

        // Step 2: Run all evidence commands and verify they pass
        let mut failed_evidence = Vec::new();

        for (idx, cmd) in evidence.iter().enumerate() {
            if !background {
                eprintln!("      evidence[{}]: {}", idx, cmd);
            }

            let output = Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .current_dir(project_dir)
                .output()
                .with_context(|| format!("Failed to execute evidence command: {}", cmd))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                failed_evidence.push(format!(
                    "Command failed: {}\nstdout: {}\nstderr: {}",
                    cmd,
                    stdout.trim(),
                    stderr.trim()
                ));

                if !background {
                    eprintln!("      ✗ failed");
                }
            } else if !background {
                eprintln!("      ✓ passed");
            }
        }

        if !failed_evidence.is_empty() {
            return Ok(ValidationResult::Fail {
                reason: format!(
                    "{} evidence command(s) failed:\n{}",
                    failed_evidence.len(),
                    failed_evidence.join("\n")
                ),
            });
        }

        // Step 3: Heuristic quality checks
        for file in files {
            let file_path = project_dir.join(file);
            if let Ok(content) = std::fs::read_to_string(&file_path) {
                // Check for common code smells
                let has_fixme = content.contains("FIXME");
                let has_xxx = content.contains("XXX");
                let has_hack = content.contains("HACK");
                let has_placeholder = content.contains("placeholder")
                    || content.contains("Placeholder");

                if has_fixme || has_xxx || has_hack || has_placeholder {
                    return Ok(ValidationResult::NeedsHumanReview {
                        reason: format!(
                            "File {} contains quality markers (FIXME/XXX/HACK/placeholder)",
                            file
                        ),
                    });
                }

                // Check for minimal implementation (very short files might be incomplete)
                let non_empty_lines: Vec<_> = content
                    .lines()
                    .filter(|l| !l.trim().is_empty() && !l.trim().starts_with("//"))
                    .collect();

                if non_empty_lines.len() < 3 && files.len() == 1 {
                    return Ok(ValidationResult::NeedsHumanReview {
                        reason: format!(
                            "File {} has only {} non-comment lines - might be incomplete",
                            file,
                            non_empty_lines.len()
                        ),
                    });
                }
            }
        }

        if !background {
            eprintln!("    ✓ validation passed");
        }

        Ok(ValidationResult::Pass)
    }

    pub fn should_retry(result: &ValidationResult) -> bool {
        matches!(result, ValidationResult::Fail { .. })
    }

    pub fn should_escalate(result: &ValidationResult) -> bool {
        matches!(result, ValidationResult::NeedsHumanReview { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_detects_todo_stub() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("stub.rs");
        fs::write(&file_path, "// TODO: implement this\n").unwrap();

        let result = ValidationJudge::validate_task(
            "test",
            &["stub.rs".to_string()],
            &[],
            temp.path(),
            true,
        )
        .unwrap();

        assert!(matches!(result, ValidationResult::Fail { .. }));
    }

    #[test]
    fn test_passes_real_code() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("real.rs");
        fs::write(&file_path, "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n").unwrap();

        let result = ValidationJudge::validate_task(
            "test",
            &["real.rs".to_string()],
            &[],
            temp.path(),
            true,
        )
        .unwrap();

        assert_eq!(result, ValidationResult::Pass);
    }

    #[test]
    fn test_needs_review_for_fixme() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("fixme.rs");
        fs::write(&file_path, "fn test() {\n    // FIXME: handle errors\n    println!(\"ok\");\n}\n").unwrap();

        let result = ValidationJudge::validate_task(
            "test",
            &["fixme.rs".to_string()],
            &[],
            temp.path(),
            true,
        )
        .unwrap();

        assert!(matches!(result, ValidationResult::NeedsHumanReview { .. }));
    }
}
