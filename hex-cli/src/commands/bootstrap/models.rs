use std::process::Command;
use std::time::Duration;
use tokio::time::sleep;

#[derive(Debug, Clone)]
pub struct ModelStatus {
    /// The tier this model serves, e.g. "T2".
    pub tier: String,
    pub name: String,
    pub loaded: bool,
}

pub struct ModelLoader {
    dry_run: bool,
}

impl ModelLoader {
    pub fn new(dry_run: bool) -> Self {
        Self { dry_run }
    }

    /// Pull the models this project actually configures.
    ///
    /// The list used to be three hardcoded ids and a hardcoded download size
    /// each. Two problems, and the second is worse than the first.
    ///
    /// The first is founding goal G1: a model named outside `hex-infer`
    /// cannot be re-pointed by editing configuration.
    ///
    /// The second is that the list had **drifted out of agreement with the
    /// config**. It pulled `gemma4:latest` and `qwen2.5-coder:32b` while
    /// `.hex/project.json` declared `gemma4-12b` and `devstral-small-2:24b`.
    /// So bootstrap downloaded up to 33 GB of models the project does not
    /// use, reported success, and left the three it does use absent. A check
    /// against the wrong subject passes for the wrong reason, and that is
    /// indistinguishable from passing.
    ///
    /// Reading `inference.tier_models` makes drift impossible: the list and
    /// the dispatcher now read the same line.
    ///
    /// The size estimate is gone rather than moved. It was a guess per model
    /// id, it was never shown to be right, and the registry it would have to
    /// track is not ours.
    pub async fn load_configured_models(&self) -> anyhow::Result<Vec<ModelStatus>> {
        let mut statuses = vec![];
        for (label, _key, model) in hex_infer::configured_tiers() {
            let loaded = if self.dry_run { false } else { self.pull_model(&model).await };
            statuses.push(ModelStatus { tier: label.to_string(), name: model, loaded });
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
            match Command::new(hex_infer::local_provider().binary)
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
        if let Ok(output) = Command::new(hex_infer::local_provider().binary)
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
