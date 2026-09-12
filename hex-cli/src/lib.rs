// Library shim — exposes crate internals to integration tests.
// The binary entry point is src/main.rs; this file re-exports the modules
// needed by tests in hex-cli/tests/.

pub mod assets;
pub mod fmt;
pub mod commands;
