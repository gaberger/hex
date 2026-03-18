//! SpacetimeDB generated bindings placeholder.
//!
//! When the `spacetimedb` feature is enabled and modules are published,
//! run `spacetime generate --lang rust --out-dir src/spacetime_bindings/`
//! to replace this file with real generated types.
//!
//! For now, this module re-exports the port types which mirror the WASM
//! module table schemas exactly (by design — ADR-025).

// The generated bindings will provide:
// - DbConnection with .db() for table access and .reducers() for calls
// - Table types matching each WASM module's #[table] structs
// - Reducer call methods matching each #[reducer] function
//
// Until codegen is available, the SpacetimeStateAdapter uses the port
// types directly and the feature-gated code path is compile-checked
// but not functional without a running SpacetimeDB instance.
