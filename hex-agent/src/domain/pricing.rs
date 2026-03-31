use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub input_per_million: f64,
    pub output_per_million: f64,
}

pub fn default_pricing() -> HashMap<String, ModelPricing> {
    let mut m = HashMap::new();
    m.insert(
        "claude-opus-4-5-20250514".into(),
        ModelPricing {
            input_per_million: 15.0,
            output_per_million: 75.0,
        },
    );
    m.insert(
        "claude-sonnet-4-20250514".into(),
        ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
        },
    );
    m.insert(
        "claude-haiku-4-5".into(),
        ModelPricing {
            input_per_million: 0.8,
            output_per_million: 4.0,
        },
    );
    m.insert(
        "gpt-4o".into(),
        ModelPricing {
            input_per_million: 5.0,
            output_per_million: 15.0,
        },
    );
    m.insert(
        "gpt-4o-mini".into(),
        ModelPricing {
            input_per_million: 0.15,
            output_per_million: 0.6,
        },
    );
    m.insert(
        "ollama".into(),
        ModelPricing {
            input_per_million: 0.0,
            output_per_million: 0.0,
        },
    );
    m
}

pub fn calculate_cost(input_tokens: u32, output_tokens: u32, pricing: &ModelPricing) -> f64 {
    let input_cost = (input_tokens as f64 / 1_000_000.0) * pricing.input_per_million;
    let output_cost = (output_tokens as f64 / 1_000_000.0) * pricing.output_per_million;
    input_cost + output_cost
}
