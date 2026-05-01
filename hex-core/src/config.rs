use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    pub autonomous: AutonomousConfig,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AutonomousConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_max_concurrent_agents")]
    pub max_concurrent_agents: u32,
    #[serde(default = "default_rollback_on_failure")]
    pub rollback_on_failure: bool,
}

fn default_enabled() -> bool {
    true
}

fn default_max_concurrent_agents() -> u32 {
    3
}

fn default_rollback_on_failure() -> bool {
    true
}

impl Config {
    pub fn load() -> Result<Self, serde_json::Error> {
        let config_str = std::fs::read_to_string(".hex/project.json")?;
        serde_json::from_str(&config_str)
    }
}