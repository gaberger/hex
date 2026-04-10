use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MethodScore {
    pub method: String,
    pub request_type: String,
    pub attempts: u32,
    pub successes: u32,
    pub failures: u32,
    pub timeouts: u32,
    pub avg_latency_ms: f64,
    pub last_score: f64,
}

impl MethodScore {
    pub fn new(method: &str, request_type: &str) -> Self {
        Self {
            method: method.to_string(),
            request_type: request_type.to_string(),
            attempts: 0,
            successes: 0,
            failures: 0,
            timeouts: 0,
            avg_latency_ms: 0.0,
            last_score: 0.0,
        }
    }

    pub fn record(&mut self, success: bool, latency_ms: f64) {
        self.attempts += 1;
        if success {
            self.successes += 1;
        } else {
            self.failures += 1;
        }
        self.avg_latency_ms =
            (self.avg_latency_ms * (self.attempts - 1) as f64 + latency_ms) / self.attempts as f64;
        self.last_score = self.compute_score();
    }

    fn compute_score(&self) -> f64 {
        if self.attempts == 0 {
            return 0.0;
        }
        let success_rate = self.successes as f64 / self.attempts as f64;
        success_rate / (self.avg_latency_ms / 1000.0 + 1.0)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrainCapabilities {
    pub workers: Vec<WorkerInfo>,
    pub inference: Vec<InferenceInfo>,
    pub steering: SteeringStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerInfo {
    pub id: String,
    pub role: String,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InferenceInfo {
    pub id: String,
    pub model: String,
    pub status: String,
    pub latency_ms: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SteeringStatus {
    Running,
    Paused,
    Stopped,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum IntentType {
    Code,
    Doc,
    Test,
    Review,
    Agent,
    WriteFile,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Intent {
    pub intent_type: IntentType,
    pub entities: Vec<String>,
    pub confidence: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RoutingMethod {
    Worker,
    Inference,
    FileWrite,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutingDecision {
    pub method: RoutingMethod,
    pub target: String,
    pub model: Option<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Outcome {
    Success,
    Failure,
    Timeout,
}
