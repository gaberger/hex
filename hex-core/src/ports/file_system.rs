//! File system port — sandboxed file operations with boundary checks.

use async_trait::async_trait;

/// The file system port — adapters implement sandboxed file I/O.
#[async_trait]
pub trait IFileSystemPort: Send + Sync {
    async fn read_file(&self, path: &str) -> Result<String, FileSystemError>;
    async fn write_file(&self, path: &str, content: &str) -> Result<(), FileSystemError>;
    async fn file_exists(&self, path: &str) -> Result<bool, FileSystemError>;
    async fn list_directory(&self, path: &str) -> Result<Vec<String>, FileSystemError>;
    async fn glob(&self, pattern: &str, base: &str) -> Result<Vec<String>, FileSystemError>;
}

#[derive(Debug, thiserror::Error)]
pub enum FileSystemError {
    #[error("Path traversal blocked: {0}")]
    PathTraversal(String),
    #[error("File not found: {0}")]
    NotFound(String),
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("IO error: {0}")]
    Io(String),
}
