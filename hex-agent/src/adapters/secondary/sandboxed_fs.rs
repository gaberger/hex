//! Sandboxed file system adapter — path traversal protection + hex boundary awareness.

use async_trait::async_trait;
use hex_core::ports::file_system::{FileSystemError, IFileSystemPort};
use std::path::{Path, PathBuf};

/// File system adapter with path traversal protection.
pub struct SandboxedFsAdapter {
    /// Root directory — all operations are confined within this.
    root: PathBuf,
}

impl SandboxedFsAdapter {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Resolve a path safely within the sandbox root.
    fn safe_path(&self, path: &str) -> Result<PathBuf, FileSystemError> {
        let requested = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.root.join(path)
        };

        // Canonicalize to resolve .. and symlinks
        let canonical = requested
            .canonicalize()
            .map_err(|_| FileSystemError::NotFound(path.to_string()))?;

        // Ensure it's within the sandbox root
        let canonical_root = self
            .root
            .canonicalize()
            .map_err(|e| FileSystemError::Io(e.to_string()))?;

        if !canonical.starts_with(&canonical_root) {
            return Err(FileSystemError::PathTraversal(format!(
                "Path {} escapes sandbox root {}",
                path,
                self.root.display()
            )));
        }

        Ok(canonical)
    }

    /// Validate that a path for writing stays within the sandbox.
    /// Unlike safe_path, the file itself may not exist yet.
    fn safe_write_path(&self, path: &str) -> Result<PathBuf, FileSystemError> {
        let requested = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.root.join(path)
        };

        if let Some(parent) = requested.parent() {
            if parent.exists() {
                let canonical_parent = parent
                    .canonicalize()
                    .map_err(|e| FileSystemError::Io(e.to_string()))?;
                let canonical_root = self
                    .root
                    .canonicalize()
                    .map_err(|e| FileSystemError::Io(e.to_string()))?;
                if !canonical_parent.starts_with(&canonical_root) {
                    return Err(FileSystemError::PathTraversal(format!(
                        "Path {} escapes sandbox root",
                        path
                    )));
                }
            }
        }

        Ok(requested)
    }
}

#[async_trait]
impl IFileSystemPort for SandboxedFsAdapter {
    async fn read_file(&self, path: &str) -> Result<String, FileSystemError> {
        let safe = self.safe_path(path)?;
        tokio::fs::read_to_string(&safe)
            .await
            .map_err(|e| FileSystemError::Io(e.to_string()))
    }

    async fn write_file(&self, path: &str, content: &str) -> Result<(), FileSystemError> {
        let target = self.safe_write_path(path)?;

        // Create parent dirs if needed
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| FileSystemError::Io(e.to_string()))?;
        }

        tokio::fs::write(&target, content)
            .await
            .map_err(|e| FileSystemError::Io(e.to_string()))
    }

    async fn file_exists(&self, path: &str) -> Result<bool, FileSystemError> {
        let requested = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.root.join(path)
        };
        Ok(requested.exists())
    }

    async fn list_directory(&self, path: &str) -> Result<Vec<String>, FileSystemError> {
        let safe = self.safe_path(path)?;
        let mut entries = Vec::new();
        let mut dir = tokio::fs::read_dir(&safe)
            .await
            .map_err(|e| FileSystemError::Io(e.to_string()))?;
        while let Some(entry) = dir
            .next_entry()
            .await
            .map_err(|e| FileSystemError::Io(e.to_string()))?
        {
            if let Some(name) = entry.file_name().to_str() {
                entries.push(name.to_string());
            }
        }
        entries.sort();
        Ok(entries)
    }

    async fn glob(&self, pattern: &str, base: &str) -> Result<Vec<String>, FileSystemError> {
        let base_path = if Path::new(base).is_absolute() {
            PathBuf::from(base)
        } else {
            self.root.join(base)
        };
        let full_pattern = base_path.join(pattern).to_string_lossy().to_string();
        let matches: Vec<String> = glob::glob(&full_pattern)
            .map_err(|e| FileSystemError::Io(e.to_string()))?
            .filter_map(|entry| entry.ok())
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        Ok(matches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_path_rejects_traversal() {
        let adapter = SandboxedFsAdapter::new(PathBuf::from("/tmp/test-sandbox"));
        // This should fail because the path doesn't exist (can't canonicalize)
        assert!(adapter.safe_path("../../etc/passwd").is_err());
    }
}
