// ADR-2026-05-19-0721
// adapters::secondary
//
// Secondary adapters translate domain primitives into concrete infrastructure calls.
// Module skeleton for step-1 of feat-ebay-mvp (docs/specs/ebay-mvp.json).
// No business logic yet; provides the entry points for database and storage backends.

pub mod stdb_client;
pub mod image_store_fs;
pub mod password_hasher_argon2;

pub use adapters::secondary::clock_system::{SystemClock, ClockPort};
pub use adapters::secondary::token_issuer_jwt::{JwtSignerHs256, TokenIssuerPort};
