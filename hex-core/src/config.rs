use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub autonomous: AutonomousConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct AutonomousConfig {
    pub enabled: bool,
    pub max_concurrent_agents: u8,
    pub rollback_on_failure: bool,
}

impl Default for AutonomousConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_concurrent_agents: 3,
            rollback_on_failure: true,
        }
    }
}
