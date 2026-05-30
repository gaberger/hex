use crate::core::ports::ClockPort;

// Re-export so sibling modules can reach the port through this adapter module
// (`secondary/mod.rs` imports `self::stdb_client::ClockPort`). The symbol still
// originates in `core::ports::clock`; this is a re-export, not a new type.
pub use crate::core::ports::ClockPort as _ClockPortReexportMarker;
pub use crate::core::ports::ClockPort;

/// Thin SpacetimeDB client handle.
///
/// The previous version imported a `connection` submodule that does not exist
/// in this crate, so the whole module failed to resolve. Hold the connection
/// string directly until the real STDB transport lands.
pub struct StdbClient {
    #[allow(dead_code)]
    connection_string: String,
}

impl StdbClient {
    pub async fn new(connection_string: &str) -> Result<Self, String> {
        Ok(StdbClient {
            connection_string: connection_string.to_string(),
        })
    }
}

/// Wall-clock implementation of `ClockPort` backed by the system clock.
pub struct SystemClock;

#[async_trait::async_trait]
impl ClockPort for SystemClock {
    async fn now_unix_millis(&self) -> u128 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    }
}