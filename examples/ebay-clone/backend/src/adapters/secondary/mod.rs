// ADR-2026-05-19-0721
// adapters::secondary
//
// Secondary adapters translate domain primitives into concrete infrastructure calls.
// Module skeleton for step-1 of feat-ebay-mvp (docs/specs/ebay-mvp.json).
// No business logic yet; provides the entry points for database and storage backends.

pub mod stdb_client;
pub mod image_store_fs;
pub mod password_hasher_argon2;

// In-memory marketplace adapter — stands in for the SpacetimeDB `marketplace`
// module, satisfying the same port contracts so the backend runs and is
// acceptance-tested without a live STDB. Wired by the composition root.
pub mod in_memory;

// Only re-export adapters that actually exist in this cluster. `ClockPort` /
// `TokenIssuerPort` are PORTS — they live in `core::ports`, not here — and the
// `jwt_signer_hs256` module is not part of this repair cluster, so the previous
// re-exports were unresolved (E0432). Consumers should reach ports via
// `crate::core::ports::*`.
pub use self::password_hasher_argon2::PasswordHasherArgon2;