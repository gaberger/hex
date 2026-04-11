use crate::domain::brain::{BrainCapabilities, Intent, MethodScore, Outcome, RoutingDecision};
use std::error::Error;

pub trait IBrainPort: Send + Sync {
    fn parse_intent(&self, request: &str) -> Intent;
    fn probe_capabilities(&self) -> Result<BrainCapabilities, Box<dyn Error + Send + Sync>>;
    fn route_request(&self, intent: &Intent, capabilities: &BrainCapabilities) -> RoutingDecision;
    fn record_outcome(
        &mut self,
        method: &str,
        intent_type: &str,
        outcome: Outcome,
        latency_ms: f64,
    );
    fn get_scores(&self) -> Vec<MethodScore>;
    fn get_best_method(&self, intent_type: &str) -> Option<String>;
}
