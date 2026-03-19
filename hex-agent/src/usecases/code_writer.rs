//! ValidatedCodeWriter — wraps file writes with boundary enforcement.
//!
//! Flow: acquire lock → extract imports → validate boundaries → write → release lock.

use hex_core::ports::coordination::{CoordinationError, ICoordinationPort, LockType, Verdict};
use hex_core::ports::file_system::{FileSystemError, IFileSystemPort};
use hex_core::rules::boundary;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("Boundary violation: {violations:?}")]
    BoundaryViolation { violations: Vec<String> },
    #[error("File lock conflict: {0}")]
    LockConflict(String),
    #[error("File system error: {0}")]
    FileSystem(#[from] FileSystemError),
    #[error("Coordination error: {0}")]
    Coordination(#[from] CoordinationError),
}

/// Result of a validated write operation.
#[derive(Debug)]
pub enum WriteResult {
    Written { path: String },
}

pub struct ValidatedCodeWriter {
    fs: Arc<dyn IFileSystemPort>,
    coordination: Arc<dyn ICoordinationPort>,
    agent_id: String,
}

impl ValidatedCodeWriter {
    pub fn new(
        fs: Arc<dyn IFileSystemPort>,
        coordination: Arc<dyn ICoordinationPort>,
        agent_id: String,
    ) -> Self {
        Self {
            fs,
            coordination,
            agent_id,
        }
    }

    /// Write a file with pre-write boundary validation.
    pub async fn write_file(&self, path: &str, content: &str) -> Result<WriteResult, WriteError> {
        // 1. Acquire exclusive file lock
        let _lock = self
            .coordination
            .acquire_file_lock(path, &self.agent_id, LockType::Exclusive)
            .await
            .map_err(|e| match e {
                CoordinationError::LockConflict {
                    file_path,
                    held_by,
                } => WriteError::LockConflict(format!("{} held by {}", file_path, held_by)),
                other => WriteError::Coordination(other),
            })?;

        // 2. Extract imports from content
        let imports = extract_imports(content);

        // 3. Client-side boundary check (fast, no network)
        let local_violations = boundary::validate_imports(path, &imports);
        if !local_violations.is_empty() {
            self.coordination
                .release_file_lock(path, &self.agent_id)
                .await
                .ok();
            let violation_msgs: Vec<String> = local_violations
                .iter()
                .map(|v| {
                    format!(
                        "{}: {} -> {} ({})",
                        v.source_file, v.source_layer, v.imported_layer, v.rule
                    )
                })
                .collect();
            return Err(WriteError::BoundaryViolation {
                violations: violation_msgs,
            });
        }

        // 4. Server-side validation (SpacetimeDB enforcer — defense in depth)
        let validation = self
            .coordination
            .validate_write(&self.agent_id, path, &imports)
            .await?;

        if validation.verdict == Verdict::Rejected {
            self.coordination
                .release_file_lock(path, &self.agent_id)
                .await
                .ok();
            return Err(WriteError::BoundaryViolation {
                violations: validation.violations,
            });
        }

        // 5. Write the file
        self.fs.write_file(path, content).await?;

        // 6. Release lock
        self.coordination
            .release_file_lock(path, &self.agent_id)
            .await
            .ok();

        Ok(WriteResult::Written {
            path: path.to_string(),
        })
    }
}

/// Extract import paths from source code (supports Rust, TypeScript).
fn extract_imports(content: &str) -> Vec<String> {
    let mut imports = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        // Rust: use crate::domain::tools;
        if trimmed.starts_with("use ") {
            if let Some(path) = trimmed.strip_prefix("use ").and_then(|s| s.strip_suffix(';')) {
                imports.push(path.trim().to_string());
            }
        }
        // TypeScript: import { X } from './path';
        else if trimmed.starts_with("import ") {
            if let Some(from_idx) = trimmed.find("from ") {
                let path = trimmed[from_idx + 5..]
                    .trim()
                    .trim_matches(|c| c == '\'' || c == '"' || c == ';');
                imports.push(path.to_string());
            }
        }
    }
    imports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_rust_imports() {
        let code = "use crate::domain::tools;\nuse crate::adapters::primary::cli;";
        let imports = extract_imports(code);
        assert_eq!(
            imports,
            vec!["crate::domain::tools", "crate::adapters::primary::cli"]
        );
    }

    #[test]
    fn extract_typescript_imports() {
        let code = "import { Foo } from './domain/entities.js';";
        let imports = extract_imports(code);
        assert_eq!(imports, vec!["./domain/entities.js"]);
    }

    #[test]
    fn extract_no_imports() {
        let imports = extract_imports("fn main() {}");
        assert!(imports.is_empty());
    }
}
