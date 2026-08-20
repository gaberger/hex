//! C4 — `IAdapterGenerator` port (per ADR-2026-04-26-1500).
//!
//! A port whose adapters are LLM agents that produce other adapters. The
//! generator's output is a hex-conformant adapter source tree + manifest +
//! shadow-test plan; the substrate then drives the produced adapter through
//! the shadow-promotion protocol (C5) like any other candidate.
//!
//! No implementation lives here — only the contract. The first generator
//! impl lands in workplan P7 (synthesizes `IModelProvider` adapters from a
//! natural-language `AdapterSpec`).

use crate::composition::{AdapterManifest, PortId};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AdapterSpec {
    pub target_port: PortId,
    pub name: String,
    pub description: String,
    pub requirements: Vec<String>,
    pub examples: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SourceTree {
    pub files: BTreeMap<PathBuf, String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum SuccessCriterion {
    /// Candidate response must agree with incumbent within tolerance on the
    /// shared evaluation set.
    ResponseEquivalence { tolerance: f64 },
    /// p99 latency in ms must stay below this ceiling.
    LatencyP99BelowMs(u64),
    /// Error rate (0.0..=1.0) must stay below this ceiling.
    ErrorRateBelow(f32),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ShadowTestPlan {
    pub duration_seconds: u64,
    pub traffic_fraction: f32,
    pub success_criteria: Vec<SuccessCriterion>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GeneratedAdapter {
    pub source: SourceTree,
    pub manifest: AdapterManifest,
    pub shadow_test_plan: ShadowTestPlan,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GeneratorCapabilities {
    pub supported_target_ports: Vec<PortId>,
    pub max_spec_complexity: u8,
}

#[derive(thiserror::Error, Debug, Clone)]
pub enum GenError {
    #[error("generator does not support target port {0:?}")]
    UnsupportedPort(PortId),
    #[error("adapter spec is invalid: {0}")]
    SpecInvalid(String),
    #[error("generation failed: {0}")]
    GenerationFailed(String),
}

pub trait IAdapterGenerator: Send + Sync {
    fn target_port(&self) -> PortId;
    fn generate(&self, spec: AdapterSpec) -> BoxFuture<'static, Result<GeneratedAdapter, GenError>>;
    fn capabilities(&self) -> GeneratorCapabilities;
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::future::FutureExt;

    /// Confirms a no-op generator implementing the trait compiles and
    /// satisfies the BoxFuture lifetime constraints.
    struct NoopGenerator;

    impl IAdapterGenerator for NoopGenerator {
        fn target_port(&self) -> PortId {
            PortId::new("inference")
        }

        fn generate(&self, _spec: AdapterSpec) -> BoxFuture<'static, Result<GeneratedAdapter, GenError>> {
            async move { Err(GenError::GenerationFailed("noop".into())) }.boxed()
        }

        fn capabilities(&self) -> GeneratorCapabilities {
            GeneratorCapabilities {
                supported_target_ports: vec![PortId::new("inference")],
                max_spec_complexity: 0,
            }
        }
    }

    #[test]
    fn noop_generator_compiles_and_advertises_port() {
        let g = NoopGenerator;
        assert_eq!(g.target_port(), PortId::new("inference"));
        assert_eq!(g.capabilities().max_spec_complexity, 0);
    }
}
