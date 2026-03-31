use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub input_per_million: f64,
    pub output_per_million: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingConfig {
    pub models: HashMap<String, ModelPricing>,
}

impl Default for PricingConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl PricingConfig {
    pub fn new() -> Self {
        let mut models = HashMap::new();
        models.insert(
            "claude-opus-4-5-20250514".into(),
            ModelPricing {
                input_per_million: 15.0,
                output_per_million: 75.0,
            },
        );
        models.insert(
            "claude-sonnet-4-20250514".into(),
            ModelPricing {
                input_per_million: 3.0,
                output_per_million: 15.0,
            },
        );
        models.insert(
            "claude-haiku-4-5".into(),
            ModelPricing {
                input_per_million: 0.8,
                output_per_million: 4.0,
            },
        );
        models.insert(
            "gpt-4o".into(),
            ModelPricing {
                input_per_million: 5.0,
                output_per_million: 15.0,
            },
        );
        models.insert(
            "gpt-4o-mini".into(),
            ModelPricing {
                input_per_million: 0.15,
                output_per_million: 0.6,
            },
        );
        models.insert(
            "ollama".into(),
            ModelPricing {
                input_per_million: 0.0,
                output_per_million: 0.0,
            },
        );
        Self { models }
    }
}

pub fn default_pricing() -> HashMap<String, ModelPricing> {
    PricingConfig::new().models
}

pub fn load_pricing(path: &str) -> PricingConfig {
    if Path::new(path).exists() {
        if let Ok(content) = fs::read_to_string(path) {
            if let Ok(config) = serde_json::from_str(&content) {
                tracing::info!("Loaded pricing from {}", path);
                return config;
            }
        }
    }
    tracing::info!("Using default pricing (no config at {})", path);
    PricingConfig::default()
}

pub fn calculate_cost(input_tokens: u32, output_tokens: u32, pricing: &ModelPricing) -> f64 {
    let input_cost = (input_tokens as f64 / 1_000_000.0) * pricing.input_per_million;
    let output_cost = (output_tokens as f64 / 1_000_000.0) * pricing.output_per_million;
    input_cost + output_cost
}
