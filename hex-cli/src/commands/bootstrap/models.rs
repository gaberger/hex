use std::process::Command;
use std::time::Duration;
use tokio::time::sleep;

#[derive(Debug, Clone)]
pub struct ModelStatus {
    pub name: String,
    pub loaded: bool,
    pub size_mb: u32,
}

pub struct ModelLoader {
    dry_run: bool,
}

impl ModelLoader {
    pub fn new(dry_run: bool) -> Self {
        Self { dry_run }
    }

    pub async fn load_default_models(&self) -> anyhow::Result<Vec<ModelStatus>> {
        let models = vec![
            ("qwen3:4b", 4700),
            ("qwen2.5-coder:32b", 19000),
            ("devstral-small-2:24b", 14000),
        ];

        let mut statuses = vec![];

        for (model_name, size_mb) in models {
            if self.dry_run {
                statuses.push(ModelStatus {
                    name: model_name.to_string(),
                    loaded: false,
                    size_mb,
                });
            } else {
                let loaded = self.pull_model(model_name).await;
                statuses.push(ModelStatus {
                    name: model_name.to_string(),
                    loaded,
                    size_mb,
                });
            }
        }

        Ok(statuses)
    }

    async fn pull_model(&self, model_name: &str) -> bool {
        // Check if model already exists
        if self.model_exists(model_name) {
            return true;
        }

        // Pull the model with retries
        for attempt in 0..3 {
            match Command::new("ollama")
                .arg("pull")
                .arg(model_name)
                .output()
            {
                Ok(output) if output.status.success() => {
                    return true;
                }
                Ok(_) => {
                    if attempt < 2 {
                        sleep(Duration::from_secs(5 * (attempt as u64 + 1))).await;
                    }
                }
                Err(_) => {
                    if attempt < 2 {
                        sleep(Duration::from_secs(5 * (attempt as u64 + 1))).await;
                    }
                }
            }
        }

        false
    }

    fn model_exists(&self, model_name: &str) -> bool {
        if let Ok(output) = Command::new("ollama")
            .arg("show")
            .arg(model_name)
            .output()
        {
            output.status.success()
        } else {
            false
        }
    }
}
