use serde_json::json;
use std::fs;
use std::path::Path;

pub struct ConfigSetup {
    profile: String,
    dry_run: bool,
}

impl ConfigSetup {
    pub fn new(config: super::BootstrapConfig) -> Self {
        Self {
            profile: config.profile,
            dry_run: config.dry_run,
        }
    }

    pub async fn setup(&self) -> anyhow::Result<()> {
        if self.dry_run {
            println!("Would create/update .hex/project.json with profile: {}", self.profile);
            return Ok(());
        }

        let config_dir = Path::new(".hex");
        if !config_dir.exists() {
            fs::create_dir_all(config_dir)?;
        }

        let config_path = config_dir.join("project.json");
        let config_json = json!({
            "inference": {
                "tier_models": {
                    "t1": "qwen3:4b",
                    "t2": "qwen2.5-coder:32b",
                    "t2_5": "devstral-small-2:24b"
                }
            },
            "bootstrap": {
                "profile": self.profile,
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "services_started": ["spacetimedb", "hex-nexus", "ollama"]
            }
        });

        let json_string = serde_json::to_string_pretty(&config_json)?;
        fs::write(&config_path, json_string)?;

        Ok(())
    }
}
