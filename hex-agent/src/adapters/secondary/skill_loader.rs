use crate::domain::{Skill, SkillManifest, SkillTrigger};
use crate::ports::skills::{SkillLoadError, SkillLoaderPort};
use async_trait::async_trait;
use std::path::Path;

/// Loads skill definitions from markdown files with YAML frontmatter.
///
/// Skill files follow the format:
/// ```markdown
/// ---
/// name: hex-scaffold
/// description: Scaffold a new hex project
/// triggers:
///   - slash: /hex-scaffold
///   - keyword: scaffold hexagonal
/// ---
/// <prompt body>
/// ```
pub struct SkillLoaderAdapter;

impl SkillLoaderAdapter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SkillLoaderPort for SkillLoaderAdapter {
    async fn load(&self, dirs: &[&str]) -> Result<SkillManifest, SkillLoadError> {
        let mut skills = Vec::new();

        for dir in dirs {
            let dir_path = Path::new(dir);
            if !dir_path.exists() {
                continue;
            }

            let pattern = format!("{}/**/*.md", dir);
            let paths = glob::glob(&pattern).map_err(|e| SkillLoadError::ReadError {
                path: dir.to_string(),
                reason: e.to_string(),
            })?;

            for entry in paths.flatten() {
                match parse_skill_file(&entry).await {
                    Ok(skill) => skills.push(skill),
                    Err(e) => {
                        tracing::warn!("Skipping skill file {}: {}", entry.display(), e);
                    }
                }
            }
        }

        Ok(SkillManifest { skills })
    }
}

async fn parse_skill_file(path: &Path) -> Result<Skill, SkillLoadError> {
    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| SkillLoadError::ReadError {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;

    // Split frontmatter from body
    let (frontmatter, body) = split_frontmatter(&content).ok_or_else(|| {
        SkillLoadError::ParseError {
            path: path.display().to_string(),
            reason: "No YAML frontmatter found (expected --- delimiters)".into(),
        }
    })?;

    let yaml: serde_yaml::Value =
        serde_yaml::from_str(&frontmatter).map_err(|e| SkillLoadError::ParseError {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;

    let name = yaml
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
        })
        .to_string();

    let description = yaml
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let triggers = parse_triggers(&yaml, &name);

    Ok(Skill {
        name,
        description,
        triggers,
        body: body.to_string(),
        source_path: path.display().to_string(),
    })
}

fn split_frontmatter(content: &str) -> Option<(String, &str)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    let after_first = &trimmed[3..];
    let end = after_first.find("\n---")?;
    let frontmatter = after_first[..end].trim().to_string();
    let body = after_first[end + 4..].trim_start();
    Some((frontmatter, body))
}

fn parse_triggers(yaml: &serde_yaml::Value, name: &str) -> Vec<SkillTrigger> {
    let mut triggers = Vec::new();

    // Always add a slash command based on the name
    triggers.push(SkillTrigger::SlashCommand(format!("/{}", name)));

    if let Some(trigger_list) = yaml.get("triggers").and_then(|v| v.as_sequence()) {
        for trigger in trigger_list {
            if let Some(slash) = trigger.get("slash").and_then(|v| v.as_str()) {
                triggers.push(SkillTrigger::SlashCommand(slash.to_string()));
            }
            if let Some(pattern) = trigger.get("pattern").and_then(|v| v.as_str()) {
                triggers.push(SkillTrigger::Pattern(pattern.to_string()));
            }
            if let Some(keyword) = trigger.get("keyword").and_then(|v| v.as_str()) {
                triggers.push(SkillTrigger::Keyword(keyword.to_string()));
            }
            // Handle string-only triggers (just keywords)
            if let Some(s) = trigger.as_str() {
                triggers.push(SkillTrigger::Keyword(s.to_string()));
            }
        }
    }

    triggers
}
