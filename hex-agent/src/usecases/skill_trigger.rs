//! Skill Trigger Engine — matches user input against the cached SkillManifest
//! and returns the matching skill's prompt body for injection.
//!
//! This use case operates entirely on in-memory data populated by the
//! SpacetimeDB subscription (or filesystem fallback). Zero network latency.

use crate::domain::skills::{SkillManifest, SkillManifestExt};
use crate::ports::skills::{SkillLoaderPort, SkillLoadError};
use std::sync::Arc;

/// Result of matching user input against available skills.
#[derive(Debug, Clone)]
pub struct SkillMatch {
    pub name: String,
    pub description: String,
    pub body: String,
}

/// Skill trigger engine — queries the SkillLoaderPort for the manifest,
/// matches user input, and returns the prompt body to inject.
pub struct SkillTriggerEngine {
    loader: Arc<dyn SkillLoaderPort>,
}

impl SkillTriggerEngine {
    pub fn new(loader: Arc<dyn SkillLoaderPort>) -> Self {
        Self { loader }
    }

    /// Match user input against all loaded skills.
    /// Returns all matching skills (there may be multiple).
    pub async fn match_input(&self, input: &str) -> Result<Vec<SkillMatch>, SkillLoadError> {
        let manifest = self.loader.load(&[]).await?;
        Ok(Self::match_against_manifest(&manifest, input))
    }

    /// Pure function — matches input against a manifest without I/O.
    /// Extracted for testability.
    pub fn match_against_manifest(manifest: &SkillManifest, input: &str) -> Vec<SkillMatch> {
        manifest
            .match_input(input)
            .into_iter()
            .map(|skill| SkillMatch {
                name: skill.name.clone(),
                description: skill.description.clone(),
                body: skill.body.clone(),
            })
            .collect()
    }

    /// Get the system prompt section listing all available skills.
    pub async fn system_prompt_section(&self) -> Result<String, SkillLoadError> {
        let manifest = self.loader.load(&[]).await?;
        Ok(manifest.system_prompt_section())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::skills::{Skill, SkillTrigger};

    #[test]
    fn test_slash_command_match() {
        let manifest = SkillManifest {
            skills: vec![Skill {
                name: "hex-scaffold".into(),
                description: "Scaffold a project".into(),
                triggers: vec![SkillTrigger::SlashCommand("/hex-scaffold".into())],
                body: "Create a new hex project...".into(),
                source_path: "test".into(),
            }],
        };

        let matches = SkillTriggerEngine::match_against_manifest(&manifest, "/hex-scaffold myproject");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "hex-scaffold");
    }

    #[test]
    fn test_keyword_match() {
        let manifest = SkillManifest {
            skills: vec![Skill {
                name: "hex-analyze".into(),
                description: "Analyze arch".into(),
                triggers: vec![SkillTrigger::Keyword("architecture health".into())],
                body: "Run hex analyze...".into(),
                source_path: "test".into(),
            }],
        };

        let matches = SkillTriggerEngine::match_against_manifest(&manifest, "check architecture health please");
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn test_no_match() {
        let manifest = SkillManifest {
            skills: vec![Skill {
                name: "hex-scaffold".into(),
                description: "Scaffold".into(),
                triggers: vec![SkillTrigger::SlashCommand("/hex-scaffold".into())],
                body: "body".into(),
                source_path: "test".into(),
            }],
        };

        let matches = SkillTriggerEngine::match_against_manifest(&manifest, "hello world");
        assert!(matches.is_empty());
    }
}
