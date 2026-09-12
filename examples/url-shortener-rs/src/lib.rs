//! url-shortener-rs — a hexagonal skeleton that runs.
//!
//! `lib.rs` is the **composition root**: the only place allowed to name a
//! concrete adapter. Everything else depends on the port.
//!
//! ```text
//!   domain  ←  ports  ←  usecases
//!                ↑
//!           adapters (primary, secondary)
//!                ↑
//!        lib.rs — wires them, once
//! ```
//!
//! Check it with `hex analyze .`.

pub mod adapters;
pub mod domain;
pub mod ports;
pub mod usecases;

use adapters::primary::TextConsole as ConsoleAdapter;
use adapters::secondary::{InMemoryCodeStore, ManualClock as ManualClockAdapter, SystemClock};
use domain::{CodeWidth, Count, Ttl};
use usecases::Shortening;

/// Re-exported so a test can drive time, or read a line, without naming an
/// adapter itself. The wiring decision stays here, in one file.
pub use adapters::primary::TextConsole;
pub use adapters::secondary::ManualClock;

/// How long a link lives by default: 24 hours.
pub const DEFAULT_TTL_MS: u64 = 86_400_000;

/// Seven characters, five bits each, is 35 bits — about 34 billion codes.
///
/// Collisions become likely at roughly 185,000 stored links. That is the
/// birthday bound, not the capacity: the probe walk handles a collision, this
/// is just where it starts happening.
pub const DEFAULT_CODE_WIDTH: u8 = 7;

/// Build the application with its real adapters.
///
/// The one line below is the whole composition decision. Change
/// `InMemoryCounterStore` to a file-backed store and nothing else moves.
pub fn counter() -> impl ports::CounterStore {
    adapters::secondary::InMemoryCounterStore::default()
}

/// Run the use case against a freshly composed application.
pub fn increment_once() -> Count {
    let mut store = counter();
    usecases::increment(&mut store)
}

/// The shortener, wired to the real clock and the default policy.
pub fn shortener() -> impl ports::Shortener + 'static {
    Shortening::new(InMemoryCodeStore::default(), SystemClock, default_ttl(), width_or_default(DEFAULT_CODE_WIDTH))
}

/// The shortener with time and policy chosen by the caller.
///
/// This is the seam that makes the hard tests possible. Pass `width = 1` and
/// the whole code space is 32 codes, so collisions happen on purpose on every
/// run instead of never.
pub fn shortener_with(clock: ManualClockAdapter, ttl_ms: u64, width: u8) -> impl ports::Shortener + 'static {
    Shortening::new(InMemoryCodeStore::default(), clock, ttl_or_default(ttl_ms), width_or_default(width))
}

/// The text console, wired to a real shortener.
pub fn console() -> impl ports::Console + 'static {
    ConsoleAdapter::new(shortener())
}

fn default_ttl() -> Ttl {
    ttl_or_default(DEFAULT_TTL_MS)
}

/// A lifetime of zero is not a lifetime. Fall back rather than refuse, because
/// the constant is known good.
fn ttl_or_default(ms: u64) -> Ttl {
    Ttl::from_millis(ms)
        .or_else(|| Ttl::from_millis(DEFAULT_TTL_MS))
        .unwrap_or_else(|| unreachable!("DEFAULT_TTL_MS is greater than zero"))
}

fn width_or_default(chars: u8) -> CodeWidth {
    CodeWidth::new(chars).unwrap_or_default()
}
