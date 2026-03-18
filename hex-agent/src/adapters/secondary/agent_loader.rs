use crate::domain::AgentDefinition;
use crate::ports::agents::{AgentLoadError, AgentLoaderPort};
use async_trait::async_trait;
use std::path::Path;

/// Loads agent definitions from YAML files.
pub struct AgentLoaderAdapter;

impl AgentLoaderAdapter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl AgentLoaderPort for AgentLoaderAdapter {
    async fn load(&self, dirs: &[&str]) -> Result<Vec<AgentDefinition>, AgentLoadError> {
        let mut agents = Vec::new();

        for dir in dirs {
            let dir_path = Path::new(dir);
            if !dir_path.exists() {
                continue;
            }

            let pattern = format!("{}/**/*.yml", dir);
            let paths = glob::glob(&pattern).map_err(|e| AgentLoadError::ReadError {
                path: dir.to_string(),
                reason: e.to_string(),
            })?;

            for entry in paths.flatten() {
                let content =
                    tokio::fs::read_to_string(&entry)
                        .await
                        .map_err(|e| AgentLoadError::ReadError {
                            path: entry.display().to_string(),
                            reason: e.to_string(),
                        })?;

                let agent: AgentDefinition =
                    serde_yaml::from_str(&content).map_err(|e| AgentLoadError::ParseError {
                        path: entry.display().to_string(),
                        reason: e.to_string(),
                    })?;

                agents.push(agent);
            }

            // Also check .yaml extension
            let pattern_yaml = format!("{}/**/*.yaml", dir);
            if let Ok(paths) = glob::glob(&pattern_yaml) {
                for entry in paths.flatten() {
                    let content =
                        tokio::fs::read_to_string(&entry)
                            .await
                            .map_err(|e| AgentLoadError::ReadError {
                                path: entry.display().to_string(),
                                reason: e.to_string(),
                            })?;

                    let agent: AgentDefinition = serde_yaml::from_str(&content)
                        .map_err(|e| AgentLoadError::ParseError {
                            path: entry.display().to_string(),
                            reason: e.to_string(),
                        })?;

                    agents.push(agent);
                }
            }
        }

        Ok(agents)
    }

    async fn load_by_name(
        &self,
        dirs: &[&str],
        name: &str,
    ) -> Result<AgentDefinition, AgentLoadError> {
        let all = self.load(dirs).await?;
        all.into_iter()
            .find(|a| a.name == name)
            .ok_or_else(|| AgentLoadError::NotFound {
                name: name.to_string(),
                dirs: dirs.iter().map(|d| d.to_string()).collect(),
            })
    }
}
