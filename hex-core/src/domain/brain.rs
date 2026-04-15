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

impl BrainCapabilities {
    pub fn route(&self, _intent: &Intent) -> RoutingDecision {
        if !self.workers.is_empty() && !self.inference.is_empty() {
            RoutingDecision {
                method: RoutingMethod::Worker,
                target: self.workers[0].id.clone(),
                model: Some(self.inference[0].model.clone()),
                reason: "workers and inference available".to_string(),
            }
        } else if !self.inference.is_empty() {
            RoutingDecision {
                method: RoutingMethod::Inference,
                target: self.inference[0].id.clone(),
                model: Some(self.inference[0].model.clone()),
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

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
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

#[allow(dead_code)]
pub struct IntentRule {
    pub label: &'static str,
    pub intent_type: IntentType,
    pub confidence: f64,
    pub signals: &'static [&'static str],
    pub matches: fn(&str) -> bool,
}

fn match_code_primary(s: &str) -> bool {
    s.contains("function") || s.contains("implement")
}
fn match_doc_primary(s: &str) -> bool {
    s.contains("documentation") || s.contains("readme")
}
fn match_review(s: &str) -> bool { s.contains("review") }
fn match_test(s: &str) -> bool { s.contains("test") }
fn match_write_file(s: &str) -> bool {
    s.contains("write") || s.contains("file")
}
fn match_agent(s: &str) -> bool { s.contains("agent") }
fn match_code_fallback(s: &str) -> bool { s.contains("code") }
fn match_doc_fallback(s: &str) -> bool { s.contains("doc") }

pub static INTENT_RULES: &[IntentRule] = &[
    IntentRule { label: "code_primary", intent_type: IntentType::Code, confidence: 0.9, signals: &["function", "implement"], matches: match_code_primary },
    IntentRule { label: "doc_primary", intent_type: IntentType::Doc, confidence: 0.9, signals: &["documentation", "readme"], matches: match_doc_primary },
    IntentRule { label: "review", intent_type: IntentType::Review, confidence: 0.9, signals: &["review"], matches: match_review },
    IntentRule { label: "test", intent_type: IntentType::Test, confidence: 0.9, signals: &["test"], matches: match_test },
    IntentRule { label: "write_file", intent_type: IntentType::WriteFile, confidence: 0.8, signals: &["write", "file"], matches: match_write_file },
    IntentRule { label: "agent", intent_type: IntentType::Agent, confidence: 0.7, signals: &["agent"], matches: match_agent },
    IntentRule { label: "code_fallback", intent_type: IntentType::Code, confidence: 0.9, signals: &["code"], matches: match_code_fallback },
    IntentRule { label: "doc_fallback", intent_type: IntentType::Doc, confidence: 0.9, signals: &["doc"], matches: match_doc_fallback },
];

impl Intent {
    pub fn parse(request: &str) -> Self {
        let lower = request.to_lowercase();

        let matched = INTENT_RULES.iter().find(|r| (r.matches)(&lower));

        Intent {
            intent_type: matched.map(|r| r.intent_type).unwrap_or(IntentType::Unknown),
            entities: vec![],
            confidence: matched.map(|r| r.confidence).unwrap_or(0.0),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_score_new() {
        let score = MethodScore::new("worker", "code");
        assert_eq!(score.method, "worker");
        assert_eq!(score.request_type, "code");
        assert_eq!(score.attempts, 0);
    }

    #[test]
    fn method_score_record_success() {
        let mut score = MethodScore::new("worker", "code");
        score.record(true, 100.0);
        assert_eq!(score.attempts, 1);
        assert_eq!(score.successes, 1);
    }

    #[test]
    fn method_score_record_failure() {
        let mut score = MethodScore::new("worker", "code");
        score.record(false, 200.0);
        assert_eq!(score.attempts, 1);
        assert_eq!(score.failures, 1);
    }

    #[test]
    fn intent_parse_code() {
        let intent = Intent::parse("write a function");
        assert!(matches!(intent.intent_type, IntentType::Code));
        assert!(intent.confidence > 0.8);
    }

    #[test]
    fn intent_parse_doc() {
        let intent = Intent::parse("write documentation");
        assert!(matches!(intent.intent_type, IntentType::Doc));
    }

    #[test]
    fn intent_parse_test() {
        let intent = Intent::parse("write tests");
        assert!(matches!(intent.intent_type, IntentType::Test));
    }

    #[test]
    fn intent_parse_review() {
        let intent = Intent::parse("review code");
        assert!(matches!(intent.intent_type, IntentType::Review));
    }

    #[test]
    fn intent_parse_unknown() {
        let intent = Intent::parse("gibberish xyz");
        assert!(matches!(intent.intent_type, IntentType::Unknown));
    }

    #[test]
    fn intent_rule_table_invariants() {
        assert!(INTENT_RULES.len() >= 8, "expected at least 8 intent rules");
        for rule in INTENT_RULES {
            assert!(!rule.label.is_empty());
            assert!(!rule.signals.is_empty(), "rule {:?} has no signals", rule.label);
            assert!(rule.confidence > 0.0, "rule {:?} has zero confidence", rule.label);
        }
        assert_eq!(INTENT_RULES[0].label, "code_primary",
            "primary code rule must precede fallback");
        let fallback_idx = INTENT_RULES.iter().position(|r| r.label == "code_fallback").unwrap();
        let primary_idx = INTENT_RULES.iter().position(|r| r.label == "code_primary").unwrap();
        assert!(primary_idx < fallback_idx,
            "code_primary must precede code_fallback to avoid short-substring match");
    }

    #[test]
    fn routing_worker_with_resources() {
        let caps = BrainCapabilities {
            workers: vec![WorkerInfo {
                id: "w1".into(),
                role: "hex-coder".into(),
                status: "running".into(),
            }],
            inference: vec![InferenceInfo {
                id: "i1".into(),
                model: "nemotron".into(),
                status: "healthy".into(),
                latency_ms: 100,
            }],
            steering: SteeringStatus::Running,
        };
        let intent = Intent {
            intent_type: IntentType::Code,
            entities: vec![],
            confidence: 0.9,
        };
        let decision = caps.route(&intent);
        assert!(matches!(decision.method, RoutingMethod::Worker));
    }

    #[test]
    fn routing_inference_fallback() {
        let caps = BrainCapabilities {
            workers: vec![],
            inference: vec![InferenceInfo {
                id: "i1".into(),
                model: "nemotron".into(),
                status: "healthy".into(),
                latency_ms: 100,
            }],
            steering: SteeringStatus::Running,
        };
        let intent = Intent {
            intent_type: IntentType::Code,
            entities: vec![],
            confidence: 0.9,
        };
        let decision = caps.route(&intent);
        assert!(matches!(decision.method, RoutingMethod::Inference));
    }

    #[test]
    fn routing_file_write_fallback() {
        let caps = BrainCapabilities {
            workers: vec![],
            inference: vec![],
            steering: SteeringStatus::Running,
        };
        let intent = Intent {
            intent_type: IntentType::Code,
            entities: vec![],
            confidence: 0.9,
        };
        let decision = caps.route(&intent);
        assert!(matches!(decision.method, RoutingMethod::FileWrite));
    }
}
