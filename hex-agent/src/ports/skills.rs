use crate::domain::SkillManifest;
use async_trait::async_trait;

/// Port for loading skill definitions from the filesystem.
#[async_trait]
pub trait SkillLoaderPort: Send + Sync {
    /// Load all skills from the given directories.
    async fn load(&self, dirs: &[&str]) -> Result<SkillManifest, SkillLoadError>;
}

#[derive(Debug, thiserror::Error)]
pub enum SkillLoadError {
    #[error("Failed to read skill file {path}: {reason}")]
    ReadError { path: String, reason: String },
    #[error("Failed to parse skill frontmatter in {path}: {reason}")]
    ParseError { path: String, reason: String },
}
