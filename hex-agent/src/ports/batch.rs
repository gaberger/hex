use crate::domain::{BatchRequest, Message, ToolDefinition, ApiRequestOptions};
use crate::ports::anthropic::AnthropicResponse;
use async_trait::async_trait;

/// Port for the Anthropic Message Batches API.
///
/// Batch requests get 50% cost reduction but have higher latency (up to 24h).
/// Use for non-interactive workloads: code analysis, bulk summarization,
/// test generation, dead-code detection.
#[async_trait]
pub trait BatchPort: Send + Sync {
    /// Submit a batch of message requests.
    ///
    /// Each request is identified by a `custom_id` for result correlation.
    /// Returns the batch metadata with the batch_id for polling.
    async fn submit_batch(
        &self,
        requests: Vec<BatchItem>,
    ) -> Result<BatchRequest, BatchError>;

    /// Poll the status of a submitted batch.
    async fn get_batch_status(&self, batch_id: &str) -> Result<BatchRequest, BatchError>;

    /// Retrieve results for a completed batch.
    ///
    /// Returns a map of custom_id → response for all completed requests.
    async fn get_batch_results(
        &self,
        batch_id: &str,
    ) -> Result<Vec<BatchResult>, BatchError>;

    /// Cancel a running batch.
    async fn cancel_batch(&self, batch_id: &str) -> Result<(), BatchError>;
}

/// A single request within a batch submission.
#[derive(Debug, Clone)]
pub struct BatchItem {
    /// Caller-provided ID to correlate results back to original requests.
    pub custom_id: String,
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: u32,
    pub model: Option<String>,
    pub options: ApiRequestOptions,
}

/// A single result from a completed batch.
#[derive(Debug, Clone)]
pub struct BatchResult {
    pub custom_id: String,
    pub response: Result<AnthropicResponse, BatchError>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum BatchError {
    #[error("Batch API error: {0}")]
    Api(String),
    #[error("Batch not found: {0}")]
    NotFound(String),
    #[error("Batch still in progress")]
    InProgress,
    #[error("Batch expired")]
    Expired,
    #[error("HTTP error: {0}")]
    Http(String),
}
