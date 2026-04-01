//! Composition root — wires adapters to ports at startup.
//!
//! This module owns all `Arc<dyn IPort>` construction so `build_app` (lib.rs)
//! stays declarative: it calls factory functions here and stores the results
//! in `AppState`. No business logic belongs here.

use std::sync::Arc;

use crate::adapters::live_context::NexusLiveContextAdapter;
use crate::ports::live_context::ILiveContextPort;

/// Build the live context adapter pointed at the running nexus REST API.
///
/// The adapter self-calls this nexus instance to enrich workplan task prompts
/// with architecture score, relevant ADRs, and the current git diff.
/// Enrichment is best-effort — offline nexus or endpoint failures produce an
/// empty string so prompt dispatch always proceeds.
pub fn build_live_context_adapter(nexus_port: u16) -> Arc<dyn ILiveContextPort> {
    Arc::new(NexusLiveContextAdapter::new(format!(
        "http://127.0.0.1:{}",
        nexus_port
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_live_context_adapter_uses_port() {
        // Factory must produce a usable Arc without panicking.
        let _adapter = build_live_context_adapter(5555);
    }
}
