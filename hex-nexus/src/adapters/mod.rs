pub mod build;
pub mod capability_token;
pub mod inference;
pub mod live_context;
pub mod context_compressor;
pub mod docker_sandbox;
pub mod env_secret;
pub mod ssh_tunnel;
pub mod ws_transport;
pub mod spacetime_agent_comm;
pub mod spacetime_chat;
pub mod spacetime_composition;
pub mod spacetime_dead_letter;
pub mod spacetime_heartbeat;
pub mod spacetime_inference;
pub mod spacetime_persona;
pub mod spacetime_secrets;
pub mod spacetime_session;
pub mod spacetime_state;
pub mod spacetime_worker_pool;
pub mod stdb_endpoint;
pub mod remote_registry;
pub mod agent_lifecycle;
pub mod inference_router;
pub mod adr_review;
pub mod events;
pub mod in_memory_experiment;
pub mod spacetime_experiment;
pub mod stash_experiment;
pub mod telegram_notifier;

#[cfg(test)]
mod state_tests;

/// Process-wide mutex serialising tests that mutate env vars.
/// Used by `stdb_endpoint::tests` and `spacetime_state::p4_2_rediscovery_tests`
/// — both touch HEX_SPACETIMEDB_HOST / HEX_STDB_FALLBACK_HOST /
/// HEX_PROJECT_DIR. Without this shared lock, `cargo test --lib` races
/// the two suites (observed 2026-05-21: env_host_wins_when_set leaked
/// a `http://from-env:9000` value into a parallel rediscovery test
/// that expected the localhost default).
///
/// tokio::sync::Mutex so the lock can legally cross `.await` boundaries
/// in async tests (clippy::await_holding_lock forbids the std variant).
#[cfg(test)]
pub(crate) fn test_env_lock() -> std::sync::Arc<tokio::sync::Mutex<()>> {
    use std::sync::{Arc, OnceLock};
    static ONCE: OnceLock<Arc<tokio::sync::Mutex<()>>> = OnceLock::new();
    ONCE.get_or_init(|| Arc::new(tokio::sync::Mutex::new(()))).clone()
}
