use crate::domain::{
    ApiRequestOptions, Message, ThinkingConfig, ToolDefinition, WorkloadClass,
};
use crate::ports::anthropic::{AnthropicPort, AnthropicResponse, AnthropicError};
use crate::ports::batch::{BatchError, BatchItem, BatchPort, BatchResult};
use crate::ports::rate_limiter::RateLimiterPort;
use crate::ports::token_metrics::TokenMetricsPort;
use std::sync::Arc;

/// Routes API requests to the optimal endpoint based on workload classification,
/// rate limit headroom, and token budget.
///
/// Interactive requests go to the Messages API (real-time).
/// Batch-eligible requests go to the Batch API (50% cost reduction, higher latency).
/// When rate limits are near capacity, the router can defer batch-eligible work
/// or recommend a fallback model.
pub struct WorkloadRouter {
    anthropic: Arc<dyn AnthropicPort>,
    batch: Arc<dyn BatchPort>,
    rate_limiter: Arc<dyn RateLimiterPort>,
    metrics: Arc<dyn TokenMetricsPort>,
    /// Utilization threshold above which batch-eligible work is deferred.
    batch_threshold: f64,
    /// Default thinking budget for extended thinking requests.
    default_thinking_budget: u32,
}

impl WorkloadRouter {
    pub fn new(
        anthropic: Arc<dyn AnthropicPort>,
        batch: Arc<dyn BatchPort>,
        rate_limiter: Arc<dyn RateLimiterPort>,
        metrics: Arc<dyn TokenMetricsPort>,
    ) -> Self {
        Self {
            anthropic,
            batch,
            rate_limiter,
            metrics,
            batch_threshold: 0.7,
            default_thinking_budget: 0,
        }
    }

    /// Set the rate limit utilization threshold for batch deferral.
    pub fn with_batch_threshold(mut self, threshold: f64) -> Self {
        self.batch_threshold = threshold;
        self
    }

    /// Set the default extended thinking budget.
    pub fn with_thinking_budget(mut self, budget: u32) -> Self {
        self.default_thinking_budget = budget;
        self
    }

    /// Classify a task and decide the routing strategy.
    pub async fn classify(&self, task_type: &str) -> RoutingDecision {
        let workload = WorkloadClass::classify(task_type);
        let utilization = self.rate_limiter.peak_utilization().await;

        match workload {
            WorkloadClass::Interactive => RoutingDecision::Realtime,
            WorkloadClass::Batch if utilization > self.batch_threshold => {
                // Under pressure — defer to batch to preserve real-time capacity
                RoutingDecision::Batch
            }
            WorkloadClass::Batch => {
                // Low utilization — can run batch-eligible work in real-time for speed
                RoutingDecision::Realtime
            }
        }
    }

    /// Send a single request through the optimal endpoint.
    pub async fn send(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        max_tokens: u32,
        model: Option<&str>,
        task_type: &str,
    ) -> Result<AnthropicResponse, WorkloadError> {
        let decision = self.classify(task_type).await;
        let options = ApiRequestOptions {
            enable_cache: true,
            thinking: ThinkingConfig::with_budget(self.default_thinking_budget),
            workload: Some(WorkloadClass::classify(task_type)),
        };

        match decision {
            RoutingDecision::Realtime => {
                let resp = self
                    .anthropic
                    .send_message(system, messages, tools, max_tokens, model, Some(&options))
                    .await
                    .map_err(WorkloadError::Api)?;

                self.metrics
                    .record_realtime(
                        model.unwrap_or("default"),
                        resp.usage.input_tokens,
                        resp.usage.output_tokens,
                        resp.usage.cache_read_tokens,
                        resp.usage.cache_write_tokens,
                    )
                    .await;

                Ok(resp)
            }
            RoutingDecision::Batch => {
                // For single requests, submit as a one-item batch
                let item = BatchItem {
                    custom_id: uuid::Uuid::new_v4().to_string(),
                    system: system.to_string(),
                    messages: messages.to_vec(),
                    tools: tools.to_vec(),
                    max_tokens,
                    model: model.map(String::from),
                    options,
                };

                let batch = self
                    .batch
                    .submit_batch(vec![item.clone()])
                    .await
                    .map_err(WorkloadError::Batch)?;

                // Poll for completion (batch API is async)
                let results = self.poll_batch(&batch.batch_id).await?;
                let result = results
                    .into_iter()
                    .find(|r| r.custom_id == item.custom_id)
                    .ok_or_else(|| WorkloadError::Batch(BatchError::NotFound("result missing".into())))?;

                let resp = result.response.map_err(WorkloadError::Batch)?;

                self.metrics
                    .record_batch(
                        model.unwrap_or("default"),
                        resp.usage.input_tokens,
                        resp.usage.output_tokens,
                    )
                    .await;

                Ok(resp)
            }
        }
    }

    /// Poll a batch until completion with exponential backoff.
    async fn poll_batch(&self, batch_id: &str) -> Result<Vec<BatchResult>, WorkloadError> {
        let mut delay = std::time::Duration::from_secs(5);
        let max_wait = std::time::Duration::from_secs(3600); // 1 hour max
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > max_wait {
                return Err(WorkloadError::Batch(BatchError::Expired));
            }

            match self.batch.get_batch_status(batch_id).await {
                Ok(batch) if batch.status == crate::domain::BatchStatus::Ended => {
                    return self
                        .batch
                        .get_batch_results(batch_id)
                        .await
                        .map_err(WorkloadError::Batch);
                }
                Ok(batch) if batch.status == crate::domain::BatchStatus::Cancelled => {
                    return Err(WorkloadError::Batch(BatchError::Api("batch cancelled".into())));
                }
                Ok(_) => {
                    tokio::time::sleep(delay).await;
                    delay = std::cmp::min(delay * 2, std::time::Duration::from_secs(60));
                }
                Err(e) => return Err(WorkloadError::Batch(e)),
            }
        }
    }
}

/// Routing decision for a workload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingDecision {
    /// Send via Messages API (real-time, standard pricing)
    Realtime,
    /// Send via Batch API (50% cost reduction, higher latency)
    Batch,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkloadError {
    #[error("API error: {0}")]
    Api(#[from] AnthropicError),
    #[error("Batch error: {0}")]
    Batch(#[from] BatchError),
}
