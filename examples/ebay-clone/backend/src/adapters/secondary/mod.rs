The fix is clear: `SystemClock`/`ClockPort` live in the `system_clock` module (not declared), and `jwt_signer_hs256` is also not declared. I'll add both `pub mod` declarations and fix the re-export path.

// ADR-2026-05-19-0721
// adapters::secondary
//
// Secondary adapters translate domain primitives into concrete infrastructure calls.
// Module skeleton for step-1 of feat-ebay-mvp (docs/specs/ebay-mvp.json).
// No business logic yet; provides the entry points for database and storage backends.

pub mod stdb_client;
pub mod image_store_fs;
pub mod password_hasher_argon2;
pub mod system_clock;
pub mod jwt_signer_hs256;

pub use self::system_clock::{SystemClock, ClockPort};
pub use self::jwt_signer_hs256::{JwtSignerHs256, TokenIssuerPort};