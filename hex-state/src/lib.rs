//! hex-state — the SpacetimeDB state adapter (ADR-2606071340 P1).
//!
//! Implements the hex-core state-port contract (`IStatePort` + focused
//! sub-traits) over SpacetimeDB's HTTP/reducer surface, plus STDB-endpoint
//! discovery (`stdb_endpoint`) and the composition adapter. Reusable outside
//! hex-nexus; the daemon depends on this crate (re-exported as
//! `crate::adapters::{spacetime_state, …}`).

pub mod spacetime_state;
pub mod spacetime_composition;
pub mod stdb_endpoint;

/// Test-only lock serializing env-mutating tests within this crate — both
/// `stdb_endpoint`'s rediscovery tests and `spacetime_state`'s p4_2 tests poke
/// `HEX_SPACETIMEDB_HOST`. `tokio::sync::Mutex` so it can cross `.await`.
#[cfg(test)]
pub(crate) fn test_env_lock() -> std::sync::Arc<tokio::sync::Mutex<()>> {
    use std::sync::{Arc, OnceLock};
    static ONCE: OnceLock<Arc<tokio::sync::Mutex<()>>> = OnceLock::new();
    ONCE.get_or_init(|| Arc::new(tokio::sync::Mutex::new(()))).clone()
}
