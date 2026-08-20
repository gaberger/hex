use crate::core::domain::*;
use async_trait::async_trait;

/// ClockPort provides a trait for accessing the current time in milliseconds since Unix epoch.
///
/// This is necessary to timestamp events and manage auction durations consistently across different adapters.
/// Adapters must implement this trait, ensuring they return the correct current time as defined by their environment.
#[async_trait]
pub trait ClockPort: Send + Sync {
    /// Returns the current Unix time in milliseconds.
    ///
    /// This method should be used whenever a timestamp is required within the application to ensure
    /// consistency and accuracy of time-based logic.
    async fn now_unix_millis(&self) -> u128;
}

// docs/specs/ebay-spec-019