// Library shim — exposes crate internals to integration tests.
// The binary entry point is src/main.rs; this file re-exports the modules
// needed by tests in hex-cli/tests/.

pub mod assets;
pub mod pipeline;
pub mod fmt;
pub mod prompts;
pub mod session;
pub mod nexus_client;
pub mod tui;
pub mod commands;
