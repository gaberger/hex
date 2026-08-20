//! L2 Adversarial swarm — pre-shadow Candidate review
//! (ADR-2026-04-26-1311 L2 / ADR-2026-04-26-1500 C6).
//!
//! Each [`AdversarialReviewer`] inspects a proposed swap and votes
//! `Approve` or `Reject(reason)`. The [`AdversarialSwarm`] runs all
//! reviewers concurrently and aggregates: any single `Reject` blocks the
//! ticket from entering Shadow. Reviewers are deliberately lightweight —
//! today they are pure-predicate adapters; future iterations will replace
//! them with LLM-backed reviewers without any framework change (the trait
//! surface stays the same).
//!
//! Wired into the propose-swap REST handler as a synchronous pre-flight
//! gate. If any reviewer rejects, the propose returns 422 and the swap
//! never reaches STDB. Logged at warn-level so the operator sees which
//! reviewer fired.
//!
//! The async-tick variant (reviewers operate on tickets that sit in
//! Candidate state until the swarm transitions them to Shadow) is the
//! natural extension when reviewers grow expensive — substrate ADR's
//! C6 framing supports it. For day one synchronous-pre-flight is the
//! smallest meaningful integration.

use std::sync::Arc;

use async_trait::async_trait;
use futures::future::join_all;
use hex_core::composition::{AdapterId, CompositionSwap};

use crate::ports::state::ISwapTicketStatePort;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewVerdict {
    Approve,
    Reject(String),
}

#[async_trait]
pub trait AdversarialReviewer: Send + Sync {
    /// Stable identifier — surfaces in the rejection message so the
    /// operator can find the reviewer in logs / source.
    fn name(&self) -> &str;

    async fn review(&self, swap: &CompositionSwap) -> ReviewVerdict;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwarmVerdict {
    pub approve: bool,
    /// Populated when `approve == false`. One entry per rejecting
    /// reviewer: `(reviewer_name, reason)`.
    pub rejections: Vec<(String, String)>,
}

pub struct AdversarialSwarm {
    reviewers: Vec<Arc<dyn AdversarialReviewer>>,
}

impl AdversarialSwarm {
    pub fn new(reviewers: Vec<Arc<dyn AdversarialReviewer>>) -> Self {
        Self { reviewers }
    }

    /// Day-one swarm — bundled non-LLM reviewers. Operators get baseline
    /// pre-flight validation without any wiring; LLM-backed reviewers
    /// can be appended later via [`Self::with_reviewer`].
    pub fn default_swarm() -> Self {
        Self::new(vec![
            Arc::new(NonEmptyAdapterIdReviewer),
            Arc::new(KnownPortReviewer::new(vec!["inference".into()])),
        ])
    }

    pub fn with_reviewer(mut self, reviewer: Arc<dyn AdversarialReviewer>) -> Self {
        self.reviewers.push(reviewer);
        self
    }

    pub async fn review_all(&self, swap: &CompositionSwap) -> SwarmVerdict {
        let futures = self.reviewers.iter().map(|r| {
            let r = r.clone();
            let swap = swap.clone();
            async move {
                let name = r.name().to_string();
                let v = r.review(&swap).await;
                (name, v)
            }
        });
        let results = join_all(futures).await;
        let rejections: Vec<(String, String)> = results
            .into_iter()
            .filter_map(|(name, v)| match v {
                ReviewVerdict::Approve => None,
                ReviewVerdict::Reject(reason) => Some((name, reason)),
            })
            .collect();
        SwarmVerdict {
            approve: rejections.is_empty(),
            rejections,
        }
    }
}

// ── Day-one reviewers ─────────────────────────────────────────────────

pub struct NonEmptyAdapterIdReviewer;

#[async_trait]
impl AdversarialReviewer for NonEmptyAdapterIdReviewer {
    fn name(&self) -> &str {
        "non-empty-adapter-id"
    }
    async fn review(&self, swap: &CompositionSwap) -> ReviewVerdict {
        if swap.new_adapter_id.0.trim().is_empty() {
            ReviewVerdict::Reject("candidate_adapter_id is empty or whitespace".into())
        } else {
            ReviewVerdict::Approve
        }
    }
}

pub struct KnownPortReviewer {
    allowed: Vec<String>,
}

impl KnownPortReviewer {
    pub fn new(allowed: Vec<String>) -> Self {
        Self { allowed }
    }
}

#[async_trait]
impl AdversarialReviewer for KnownPortReviewer {
    fn name(&self) -> &str {
        "known-port"
    }
    async fn review(&self, swap: &CompositionSwap) -> ReviewVerdict {
        if self.allowed.iter().any(|p| p == &swap.port.0) {
            ReviewVerdict::Approve
        } else {
            ReviewVerdict::Reject(format!(
                "port '{}' not in allowed set {:?}",
                swap.port.0, self.allowed
            ))
        }
    }
}

/// Caps the number of concurrent shadow tickets per port. Each shadow at
/// fraction `f` mirrors `f` of incoming traffic to the candidate; N
/// concurrent shadows on the same port multiply mirrored load by N.
/// Operators stacking 5 candidates against one incumbent can quadruple
/// inference cost without realizing it. This reviewer rejects when the
/// proposed swap would push the port past `max_per_port` open shadows.
///
/// Reads from `ISwapTicketStatePort.shadow_tickets_due` — same surface
/// the promotion judge uses, so the count reflects exactly what the
/// substrate considers "currently shadowing".
pub struct MaxConcurrentSwapsReviewer {
    state: Arc<dyn ISwapTicketStatePort>,
    max_per_port: usize,
}

impl MaxConcurrentSwapsReviewer {
    pub fn new(state: Arc<dyn ISwapTicketStatePort>, max_per_port: usize) -> Self {
        Self { state, max_per_port }
    }
}

#[async_trait]
impl AdversarialReviewer for MaxConcurrentSwapsReviewer {
    fn name(&self) -> &str {
        "max-concurrent-swaps"
    }
    async fn review(&self, swap: &CompositionSwap) -> ReviewVerdict {
        // shadow_tickets_due returns state="shadow"; we use Utc::now()
        // because the upstream filter is server-side state, not window-
        // elapsed (the orchestrator does the window check client-side).
        let now = chrono::Utc::now().to_rfc3339();
        let tickets = match self.state.shadow_tickets_due(&now).await {
            Ok(t) => t,
            Err(e) => {
                // STDB read failure → abstain. The deterministic reviewers
                // upstream (NonEmptyAdapterId, KnownPort) still gate basic
                // sanity; we don't want to block proposes on infra failure.
                tracing::warn!(error = %e, "MaxConcurrentSwapsReviewer: shadow_tickets_due failed; abstaining");
                return ReviewVerdict::Approve;
            }
        };
        let same_port_count = tickets
            .iter()
            .filter(|t| t.port_id == swap.port.0)
            .count();
        if same_port_count >= self.max_per_port {
            ReviewVerdict::Reject(format!(
                "{} shadow ticket(s) already open for port '{}' (max {})",
                same_port_count, swap.port.0, self.max_per_port
            ))
        } else {
            ReviewVerdict::Approve
        }
    }
}

/// LLM-backed reviewer — calls an `IInferencePort` to evaluate the swap.
/// Demonstrates the framework's extensibility: the reviewer uses the same
/// inference port the substrate would otherwise be swapping. Note that
/// this means the LLM reviewer's verdict on a swap that targets the
/// inference port itself is being delivered BY the incumbent — fine for
/// pre-shadow review (the candidate isn't routing yet) but worth
/// remembering when interpreting verdicts.
///
/// The prompt format is deliberately compact and instructs the model to
/// answer with `APPROVE` or `REJECT: <reason>` on the first line. We
/// parse only the first non-empty line — anything else is ignored. If
/// the inference call itself fails, the reviewer abstains (returns
/// `Approve`) rather than blocking the swap on infrastructure failure;
/// the deterministic non-LLM reviewers retain veto power.
pub struct LlmReviewer {
    inference: Arc<dyn hex_core::ports::inference::IInferencePort>,
    model: String,
}

impl LlmReviewer {
    pub fn new(
        inference: Arc<dyn hex_core::ports::inference::IInferencePort>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            inference,
            model: model.into(),
        }
    }

    fn build_prompt(swap: &CompositionSwap) -> String {
        let manifest_json =
            serde_json::to_string(&swap.manifest).unwrap_or_else(|_| "{}".to_string());
        format!(
            "You are an adversarial reviewer for a runtime substrate swap.\n\
             Port:                {}\n\
             Candidate adapter:   {}\n\
             Candidate manifest:  {}\n\n\
             Decide whether the substrate should accept this candidate into shadow review.\n\
             Respond with EXACTLY one of these two formats on the FIRST line:\n\
                 APPROVE\n\
                 REJECT: <one-line reason>\n",
            swap.port.0, swap.new_adapter_id.0, manifest_json
        )
    }

    fn parse_verdict(response_text: &str) -> ReviewVerdict {
        let first = response_text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        if first.eq_ignore_ascii_case("approve") {
            ReviewVerdict::Approve
        } else if let Some(rest) = first.strip_prefix("REJECT:").or_else(|| first.strip_prefix("reject:")) {
            ReviewVerdict::Reject(rest.trim().to_string())
        } else if first.is_empty() {
            // Empty response → abstain (Approve). Don't block on a
            // confused model.
            ReviewVerdict::Approve
        } else {
            // Unparseable response — abstain rather than block. Log the
            // raw response so operators can investigate.
            tracing::warn!(
                response = %first,
                "LlmReviewer: unparseable verdict line; abstaining (Approve)"
            );
            ReviewVerdict::Approve
        }
    }
}

#[async_trait]
impl AdversarialReviewer for LlmReviewer {
    fn name(&self) -> &str {
        "llm-reviewer"
    }
    async fn review(&self, swap: &CompositionSwap) -> ReviewVerdict {
        run_llm_review(&self.inference, &self.model, &Self::build_prompt(swap), "llm-reviewer").await
    }
}

/// LLM-backed reviewer with a hex-architecture-aware prompt. Asks the
/// model whether the candidate would violate hexagonal rules (importing
/// across layers, sneaking in concrete deps that should live in a port,
/// etc.). Same abstain-on-failure semantics as `LlmReviewer`. Use case:
/// catches operator-shaped LLM-generated adapters that compile but
/// quietly violate the architecture invariants `hex analyze .` would
/// later flag.
pub struct ArchitectureLlmReviewer {
    inference: Arc<dyn hex_core::ports::inference::IInferencePort>,
    model: String,
}

impl ArchitectureLlmReviewer {
    pub fn new(
        inference: Arc<dyn hex_core::ports::inference::IInferencePort>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            inference,
            model: model.into(),
        }
    }

    fn build_prompt(swap: &CompositionSwap) -> String {
        let manifest_json =
            serde_json::to_string(&swap.manifest).unwrap_or_else(|_| "{}".to_string());
        format!(
            "You are a hex-architecture reviewer for a runtime substrate swap.\n\
             Hex rules — adapters MUST NOT import other adapters. Ports define \
             the contract; adapters implement them; only the composition root \
             wires adapters together. Adapter manifest deps should reference \
             ports/value-types or generic crates, never sibling adapters.\n\n\
             Port:                {}\n\
             Candidate adapter:   {}\n\
             Candidate manifest:  {}\n\n\
             Decide whether this candidate's manifest deps respect hexagonal \
             boundaries. Respond with EXACTLY one of these two formats on the \
             FIRST line:\n\
                 APPROVE\n\
                 REJECT: <one-line reason naming the violated rule>\n",
            swap.port.0, swap.new_adapter_id.0, manifest_json
        )
    }
}

#[async_trait]
impl AdversarialReviewer for ArchitectureLlmReviewer {
    fn name(&self) -> &str {
        "architecture-llm-reviewer"
    }
    async fn review(&self, swap: &CompositionSwap) -> ReviewVerdict {
        run_llm_review(
            &self.inference,
            &self.model,
            &Self::build_prompt(swap),
            "architecture-llm-reviewer",
        )
        .await
    }
}

/// Shared LLM-reviewer dispatch — both LLM reviewers share request shape
/// and verdict parsing. Extracted so a new prompt-shaped reviewer is just
/// a new struct with a `build_prompt` method.
async fn run_llm_review(
    inference: &Arc<dyn hex_core::ports::inference::IInferencePort>,
    model: &str,
    prompt: &str,
    reviewer_name: &str,
) -> ReviewVerdict {
    use hex_core::domain::messages::{ContentBlock, Message};
    use hex_core::ports::inference::{InferenceRequest, Priority};

    let req = InferenceRequest {
        model: model.to_string(),
        system_prompt: String::new(),
        messages: vec![Message::user(prompt)],
        tools: vec![],
        max_tokens: 128,
        temperature: 0.0,
        thinking_budget: None,
        cache_control: false,
        priority: Priority::Normal,
        grammar: None,
    };

    let resp = match inference.complete(req).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(reviewer = %reviewer_name, error = %e, "LLM reviewer: inference failed; abstaining (Approve)");
            return ReviewVerdict::Approve;
        }
    };

    let text: String = resp
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    LlmReviewer::parse_verdict(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::composition::{AdapterId, AdapterManifest, PortId};

    fn swap(port: &str, adapter: &str) -> CompositionSwap {
        CompositionSwap {
            port: PortId::new(port),
            new_adapter_id: AdapterId::new(adapter),
            manifest: AdapterManifest {
                adapter_id: AdapterId::new(adapter),
                port: PortId::new(port),
                version: "0.1.0".into(),
                deps: vec![],
            },
        }
    }

    #[tokio::test]
    async fn default_swarm_approves_well_formed_swap() {
        let s = AdversarialSwarm::default_swarm();
        let v = s.review_all(&swap("inference", "mock-b")).await;
        assert!(v.approve, "rejections: {:?}", v.rejections);
    }

    #[tokio::test]
    async fn default_swarm_rejects_empty_adapter_id() {
        let s = AdversarialSwarm::default_swarm();
        let v = s.review_all(&swap("inference", "  ")).await;
        assert!(!v.approve);
        assert!(v.rejections.iter().any(|(n, _)| n == "non-empty-adapter-id"));
    }

    #[tokio::test]
    async fn default_swarm_rejects_unknown_port() {
        let s = AdversarialSwarm::default_swarm();
        let v = s.review_all(&swap("storage", "candidate")).await;
        assert!(!v.approve);
        assert!(v.rejections.iter().any(|(n, _)| n == "known-port"));
    }

    #[tokio::test]
    async fn swarm_aggregates_multiple_rejections() {
        let s = AdversarialSwarm::default_swarm();
        let v = s.review_all(&swap("storage", "")).await;
        assert!(!v.approve);
        // Both reviewers should fire on this swap.
        assert!(v.rejections.iter().any(|(n, _)| n == "non-empty-adapter-id"));
        assert!(v.rejections.iter().any(|(n, _)| n == "known-port"));
    }

    /// Custom reviewer that always rejects — confirms `with_reviewer`
    /// extension point and that one rejection is enough to block.
    struct AlwaysReject;
    #[async_trait]
    impl AdversarialReviewer for AlwaysReject {
        fn name(&self) -> &str {
            "always-reject"
        }
        async fn review(&self, _swap: &CompositionSwap) -> ReviewVerdict {
            ReviewVerdict::Reject("nope".into())
        }
    }

    #[tokio::test]
    async fn extension_reviewer_can_block_otherwise_approved_swap() {
        let s = AdversarialSwarm::default_swarm().with_reviewer(Arc::new(AlwaysReject));
        let v = s.review_all(&swap("inference", "ok-adapter")).await;
        assert!(!v.approve);
        assert_eq!(v.rejections.len(), 1);
        assert_eq!(v.rejections[0].0, "always-reject");
    }

    #[tokio::test]
    async fn empty_swarm_approves_everything() {
        let s = AdversarialSwarm::new(vec![]);
        let v = s.review_all(&swap("any-port", "any-adapter")).await;
        assert!(v.approve);
        assert!(v.rejections.is_empty());
    }

    // ── LlmReviewer tests ────────────────────────────────────

    #[test]
    fn llm_reviewer_parses_approve() {
        assert_eq!(LlmReviewer::parse_verdict("APPROVE"), ReviewVerdict::Approve);
        assert_eq!(LlmReviewer::parse_verdict("approve\nextra fluff"), ReviewVerdict::Approve);
    }

    #[test]
    fn llm_reviewer_parses_reject_with_reason() {
        match LlmReviewer::parse_verdict("REJECT: candidate looks suspicious") {
            ReviewVerdict::Reject(reason) => assert_eq!(reason, "candidate looks suspicious"),
            other => panic!("expected Reject, got {:?}", other),
        }
        match LlmReviewer::parse_verdict("reject: lowercase prefix works too") {
            ReviewVerdict::Reject(reason) => assert!(reason.contains("lowercase")),
            other => panic!("expected Reject, got {:?}", other),
        }
    }

    #[test]
    fn llm_reviewer_abstains_on_unparseable_response() {
        // Model returned freeform prose instead of APPROVE/REJECT — abstain,
        // don't block. Other reviewers retain veto power.
        assert_eq!(
            LlmReviewer::parse_verdict("Well, it depends on the context..."),
            ReviewVerdict::Approve
        );
    }

    #[test]
    fn llm_reviewer_abstains_on_empty_response() {
        assert_eq!(LlmReviewer::parse_verdict(""), ReviewVerdict::Approve);
        assert_eq!(LlmReviewer::parse_verdict("   \n\n  "), ReviewVerdict::Approve);
    }

    #[tokio::test]
    async fn llm_reviewer_calls_inference_and_returns_approve_for_approving_model() {
        use hex_core::ports::inference::mock::MockInferencePort;
        use hex_core::ports::inference::IInferencePort;
        let mock: Arc<dyn IInferencePort> = Arc::new(MockInferencePort::with_response("APPROVE"));
        let reviewer = LlmReviewer::new(mock, "test-model");
        let v = reviewer.review(&swap("inference", "candidate-x")).await;
        assert_eq!(v, ReviewVerdict::Approve);
    }

    #[tokio::test]
    async fn llm_reviewer_rejects_when_model_says_reject() {
        use hex_core::ports::inference::mock::MockInferencePort;
        use hex_core::ports::inference::IInferencePort;
        let mock: Arc<dyn IInferencePort> =
            Arc::new(MockInferencePort::with_response("REJECT: traffic_fraction too high"));
        let reviewer = LlmReviewer::new(mock, "test-model");
        match reviewer.review(&swap("inference", "candidate-x")).await {
            ReviewVerdict::Reject(reason) => assert!(reason.contains("traffic_fraction")),
            other => panic!("expected Reject, got {:?}", other),
        }
    }

    // ── MaxConcurrentSwapsReviewer tests ───────────────────

    #[derive(Default)]
    struct StubSwapState {
        tickets: std::sync::Mutex<Vec<crate::ports::state::SwapTicketRecord>>,
        fail: std::sync::Mutex<bool>,
    }

    #[async_trait]
    impl ISwapTicketStatePort for StubSwapState {
        async fn swap_ticket_create(
            &self, _: &str, _: &str, _: &str, _: &str, _: &str, _: &str,
            _: f32, _: u64, _: &str, _: &str,
        ) -> Result<(), crate::ports::state::StateError> { Ok(()) }
        async fn swap_ticket_transition(&self, _: &str, _: &str, _: &str) -> Result<(), crate::ports::state::StateError> { Ok(()) }
        async fn swap_ticket_set_shadow_started(&self, _: &str, _: &str) -> Result<(), crate::ports::state::StateError> { Ok(()) }
        async fn swap_ticket_set_config(&self, _: &str, _: &str, _: f32, _: u64, _: &str) -> Result<(), crate::ports::state::StateError> { Ok(()) }
        async fn shadow_sample_record(
            &self, _: &str, _: u64, _: &str, _: &str, _: &str, _: &str, _: bool, _: &str, _: &str,
        ) -> Result<(), crate::ports::state::StateError> { Ok(()) }
        async fn shadow_tickets_due(&self, _: &str) -> Result<Vec<crate::ports::state::SwapTicketRecord>, crate::ports::state::StateError> {
            if *self.fail.lock().unwrap() {
                return Err(crate::ports::state::StateError::Storage("injected".into()));
            }
            Ok(self.tickets.lock().unwrap().clone())
        }
        async fn shadow_samples_for(&self, _: &str) -> Result<Vec<crate::ports::state::ShadowSampleRecord>, crate::ports::state::StateError> { Ok(vec![]) }
        async fn shadow_green_tickets(&self) -> Result<Vec<crate::ports::state::SwapTicketRecord>, crate::ports::state::StateError> { Ok(vec![]) }
    }

    fn ticket_for_port(id: &str, port: &str) -> crate::ports::state::SwapTicketRecord {
        crate::ports::state::SwapTicketRecord {
            id: id.into(),
            project_id: "test".into(),
            port_id: port.into(),
            incumbent_adapter_id: "mock-a".into(),
            candidate_adapter_id: format!("cand-{}", id),
            candidate_manifest_json: "{}".into(),
            state: "shadow".into(),
            shadow_traffic_fraction: 1.0,
            shadow_window_seconds: 300,
            shadow_started_at: "2026-04-26T18:00:00Z".into(),
            success_criteria_json: "[]".into(),
            created_at: "2026-04-26T18:00:00Z".into(),
            updated_at: "2026-04-26T18:00:00Z".into(),
        }
    }

    #[tokio::test]
    async fn max_concurrent_under_limit_approves() {
        let state = Arc::new(StubSwapState::default());
        // 1 in flight, max=2 → approve another.
        state.tickets.lock().unwrap().push(ticket_for_port("t1", "inference"));
        let r = MaxConcurrentSwapsReviewer::new(state, 2);
        assert_eq!(r.review(&swap("inference", "candidate-x")).await, ReviewVerdict::Approve);
    }

    #[tokio::test]
    async fn max_concurrent_at_limit_rejects() {
        let state = Arc::new(StubSwapState::default());
        // 2 in flight on "inference", max=2 → reject another on inference.
        state.tickets.lock().unwrap().push(ticket_for_port("t1", "inference"));
        state.tickets.lock().unwrap().push(ticket_for_port("t2", "inference"));
        let r = MaxConcurrentSwapsReviewer::new(state, 2);
        match r.review(&swap("inference", "candidate-x")).await {
            ReviewVerdict::Reject(reason) => {
                assert!(reason.contains("inference"));
                assert!(reason.contains("max 2"));
            }
            other => panic!("expected Reject, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn max_concurrent_per_port_isolation() {
        let state = Arc::new(StubSwapState::default());
        // 2 on "inference", 0 on "storage", max=2. Swap on storage approves.
        state.tickets.lock().unwrap().push(ticket_for_port("t1", "inference"));
        state.tickets.lock().unwrap().push(ticket_for_port("t2", "inference"));
        let r = MaxConcurrentSwapsReviewer::new(state, 2);
        assert_eq!(r.review(&swap("storage", "candidate-x")).await, ReviewVerdict::Approve);
    }

    #[tokio::test]
    async fn max_concurrent_abstains_on_state_failure() {
        let state = Arc::new(StubSwapState::default());
        *state.fail.lock().unwrap() = true;
        let r = MaxConcurrentSwapsReviewer::new(state, 2);
        // STDB read fails → abstain (Approve), don't block on infra.
        assert_eq!(r.review(&swap("inference", "candidate-x")).await, ReviewVerdict::Approve);
    }

    #[tokio::test]
    async fn llm_reviewer_abstains_when_inference_fails() {
        use hex_core::ports::inference::mock::MockInferencePort;
        use hex_core::ports::inference::IInferencePort;
        let mock: Arc<dyn IInferencePort> = Arc::new(MockInferencePort::unreachable());
        let reviewer = LlmReviewer::new(mock, "test-model");
        // Inference call fails with ProviderUnavailable → abstain (Approve).
        // Don't block swaps on infrastructure failure.
        let v = reviewer.review(&swap("inference", "candidate-x")).await;
        assert_eq!(v, ReviewVerdict::Approve);
    }

    // ── ArchitectureLlmReviewer tests ──────────────────────

    #[test]
    fn architecture_reviewer_prompt_mentions_hex_rules() {
        let prompt = ArchitectureLlmReviewer::build_prompt(&swap("inference", "mock-b"));
        assert!(prompt.contains("hex"));
        assert!(prompt.contains("hexagonal"));
        assert!(prompt.contains("adapters MUST NOT import other adapters"));
        assert!(prompt.contains("inference"));
        assert!(prompt.contains("mock-b"));
    }

    #[tokio::test]
    async fn architecture_reviewer_approves_when_model_says_approve() {
        use hex_core::ports::inference::mock::MockInferencePort;
        use hex_core::ports::inference::IInferencePort;
        let mock: Arc<dyn IInferencePort> = Arc::new(MockInferencePort::with_response("APPROVE"));
        let r = ArchitectureLlmReviewer::new(mock, "test-model");
        assert_eq!(r.review(&swap("inference", "ok")).await, ReviewVerdict::Approve);
    }

    #[tokio::test]
    async fn architecture_reviewer_rejects_with_named_rule() {
        use hex_core::ports::inference::mock::MockInferencePort;
        use hex_core::ports::inference::IInferencePort;
        let mock: Arc<dyn IInferencePort> = Arc::new(MockInferencePort::with_response(
            "REJECT: deps include sibling adapter `ollama-rs` — adapters must not import other adapters",
        ));
        let r = ArchitectureLlmReviewer::new(mock, "test-model");
        match r.review(&swap("inference", "broken")).await {
            ReviewVerdict::Reject(reason) => assert!(reason.contains("sibling adapter")),
            other => panic!("expected Reject, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn architecture_reviewer_abstains_on_infra_failure() {
        use hex_core::ports::inference::mock::MockInferencePort;
        use hex_core::ports::inference::IInferencePort;
        let mock: Arc<dyn IInferencePort> = Arc::new(MockInferencePort::unreachable());
        let r = ArchitectureLlmReviewer::new(mock, "test-model");
        assert_eq!(
            r.review(&swap("inference", "x")).await,
            ReviewVerdict::Approve
        );
    }

    #[tokio::test]
    async fn architecture_reviewer_has_distinct_name_from_llm_reviewer() {
        // Both LLM reviewers can co-exist in one swarm — make sure their
        // names don't collide so the rejection-attribution tells the
        // operator which prompt fired.
        use hex_core::ports::inference::mock::MockInferencePort;
        use hex_core::ports::inference::IInferencePort;
        let mock: Arc<dyn IInferencePort> = Arc::new(MockInferencePort::with_response("APPROVE"));
        let llm = LlmReviewer::new(mock.clone(), "m");
        let arch = ArchitectureLlmReviewer::new(mock, "m");
        assert_ne!(llm.name(), arch.name());
    }
}

// Suppress dead-code warning: AdapterId import path is needed by the
// trait definition's signature inference even though no top-level item
// references it directly. (Rust 1.x analyzer false-positive for trait-
// associated path imports.)
#[allow(dead_code)]
fn _unused_marker(_: AdapterId) {}
