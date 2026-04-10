use crate::adapters::spacetime_inference::SpacetimeInferenceAdapter;
use crate::domain::brain::{
    BrainCapabilities, InferenceInfo, Intent, IntentType, MethodScore, Outcome, RoutingDecision,
    RoutingMethod, SteeringStatus, WorkerInfo,
};
use std::collections::HashMap;
use std::error::Error;

pub struct BrainAdapter {
    scores: HashMap<String, MethodScore>,
    nexus_url: String,
}

impl BrainAdapter {
    pub fn new(nexus_url: &str) -> Self {
        Self {
            scores: HashMap::new(),
            nexus_url: nexus_url.to_string(),
        }
    }

    pub fn parse_intent(&self, request: &str) -> Intent {
        let lower = request.to_lowercase();

        let intent_type = if lower.contains("code")
            || lower.contains("function")
            || lower.contains("implement")
        {
            IntentType::Code
        } else if lower.contains("doc") || lower.contains("readme") || lower.contains("document") {
            IntentType::Doc
        } else if lower.contains("test") {
            IntentType::Test
        } else if lower.contains("review") {
            IntentType::Review
        } else if lower.contains("file") || lower.contains("write") {
            IntentType::WriteFile
        } else {
            IntentType::Unknown
        };

        let confidence = match intent_type {
            IntentType::Code | IntentType::Doc | IntentType::Test | IntentType::Review => 0.9,
            IntentType::WriteFile => 0.8,
            IntentType::Unknown => 0.0,
            IntentType::Agent => 0.7,
        };

        Intent {
            intent_type,
            entities: vec![],
            confidence,
        }
    }

    pub fn probe_capabilities(&self) -> Result<BrainCapabilities, Box<dyn Error + Send + Sync>> {
        Ok(BrainCapabilities {
            workers: vec![],
            inference: vec![],
            steering: SteeringStatus::Running,
        })
    }

    pub fn route_request(
        &self,
        intent: &Intent,
        capabilities: &BrainCapabilities,
    ) -> RoutingDecision {
        if !capabilities.workers.is_empty() && !capabilities.inference.is_empty() {
            RoutingDecision {
                method: RoutingMethod::Worker,
                target: capabilities.workers[0].id.clone(),
                model: Some(capabilities.inference[0].model.clone()),
                reason: "workers and inference available".to_string(),
            }
        } else if !capabilities.inference.is_empty() {
            RoutingDecision {
                method: RoutingMethod::Inference,
                target: capabilities.inference[0].id.clone(),
                model: Some(capabilities.inference[0].model.clone()),
                reason: "direct inference fallback".to_string(),
            }
        } else {
            RoutingDecision {
                method: RoutingMethod::FileWrite,
                target: "fallback".to_string(),
                model: None,
                reason: "no workers or inference".to_string(),
            }
        }
    }

    pub fn record_outcome(
        &mut self,
        method: &str,
        intent_type: &str,
        outcome: Outcome,
        latency_ms: f64,
    ) {
        let key = format!("{}:{}", method, intent_type);
        let entry = self
            .scores
            .entry(key.clone())
            .or_insert_with(|| MethodScore::new(method, intent_type));

        match outcome {
            Outcome::Success => entry.successes += 1,
            Outcome::Failure => entry.failures += 1,
            Outcome::Timeout => entry.timeouts += 1,
        }
        entry.attempts += 1;
        entry.last_score = Self::compute_score(entry);
    }

    fn compute_score(state: &MethodScore) -> f64 {
        if state.attempts == 0 {
            return 0.0;
        }
        let success_rate = state.successes as f64 / state.attempts as f64;
        success_rate / (state.avg_latency_ms / 1000.0 + 1.0)
    }

    pub fn get_scores(&self) -> Vec<MethodScore> {
        self.scores.values().cloned().collect()
    }

    pub fn get_best_method(&self, _intent_type: &str) -> Option<String> {
        self.scores
            .values()
            .max_by(|a, b| a.last_score.partial_cmp(&b.last_score).unwrap())
            .map(|s| s.method.clone())
    }
}
