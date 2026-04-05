use axum::Json;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct CostMetrics {
    pub total_cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub realtime_requests: u32,
    pub batch_requests: u32,
}

#[derive(Serialize, Deserialize)]
pub struct CostResponse {
    pub cost: CostMetrics,
    pub source: String,
}

pub async fn get_cost_metrics() -> Json<CostResponse> {
    Json(CostResponse {
        cost: CostMetrics {
            total_cost_usd: 0.0,
            input_tokens: 0,
            output_tokens: 0,
            realtime_requests: 0,
            batch_requests: 0,
        },
        source: "hex-agent-v1 (not yet connected)".to_string(),
    })
}