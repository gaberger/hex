#[cfg(test)]
mod brain_tests {
    use hex_core::brain::*;

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
        assert_eq!(score.failures, 0);
    }

    #[test]
    fn method_score_record_failure() {
        let mut score = MethodScore::new("worker", "code");
        score.record(false, 200.0);
        assert_eq!(score.attempts, 1);
        assert_eq!(score.successes, 0);
        assert_eq!(score.failures, 1);
    }

    #[test]
    fn method_score_compute_score() {
        let mut score = MethodScore::new("worker", "code");
        score.record(true, 100.0);
        score.record(true, 100.0);
        score.record(false, 100.0);
        // 2 successes, 1 failure = 2/3 = 0.67 rate
        // latency = 100ms
        // score = 0.67 / 1.1 = ~0.6
        assert!(score.last_score > 0.5);
        assert!(score.last_score < 0.7);
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
        assert_eq!(intent.confidence, 0.0);
    }

    #[test]
    fn routing_decision_worker() {
        let caps = BrainCapabilities {
            workers: vec![WorkerInfo {
                id: "w1".into(),
                role: "hex-coder".into(),
                status: "running".into(),
            }],
            inference: vec![InferenceInfo {
                id: "i1".into(),
                model: "nemotron-mini".into(),
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
        let decision = BrainCapabilities::route(&intent, &caps);
        assert!(matches!(decision.method, RoutingMethod::Worker));
    }

    #[test]
    fn routing_decision_inference_fallback() {
        let caps = BrainCapabilities {
            workers: vec![],
            inference: vec![InferenceInfo {
                id: "i1".into(),
                model: "nemotron-mini".into(),
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
        let decision = BrainCapabilities::route(&intent, &caps);
        assert!(matches!(decision.method, RoutingMethod::Inference));
    }

    #[test]
    fn routing_decision_file_write_fallback() {
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
        let decision = BrainCapabilities::route(&intent, &caps);
        assert!(matches!(decision.method, RoutingMethod::FileWrite));
    }
}
