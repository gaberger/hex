use crate::ports::{AgentDefinition, agents::{AgentLoadError, AgentLoaderPort}};
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

            // Collect both .yml and .yaml files
            let pattern_yaml = format!("{}/**/*.yaml", dir);
            let all_paths = paths.flatten().chain(
                glob::glob(&pattern_yaml).into_iter().flatten().flatten()
            );

            for entry in all_paths {
                let content = match tokio::fs::read_to_string(&entry).await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::debug!("Skipping {}: {}", entry.display(), e);
                        continue;
                    }
                };

                match serde_yaml::from_str::<AgentDefinition>(&content) {
                    Ok(agent) => agents.push(agent),
                    Err(e) => {
                        tracing::debug!("Skipping {}: {}", entry.display(), e);
                        continue;
                    }
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
