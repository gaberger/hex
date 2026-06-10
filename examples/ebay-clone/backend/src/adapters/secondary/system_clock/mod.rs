use std::time::{SystemTime, UNIX_EPOCH};

/// Secondary adapter: system clock.
///
/// Implements ClockPort.now_unix_millis() via SystemTime::now().
/// Trivial but kept as an adapter so use cases can inject a TestClock in unit tests (ebay-spec-014 race window needs deterministic time control).
pub trait ClockPort {
    fn now_unix_millis(&self) -> i64;
}

/// A system clock implementation using SystemTime.
#[derive(Default)]
pub struct SystemClock;

impl ClockPort for SystemClock {
    fn now_unix_millis(&self) -> i64 {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("System time is before UNIX epoch");
        duration.as_millis() as i64
    }
}

// ADR-2026-05-19-0721
// docs/specs/ebay-spec-014
// docs/specs/ebay-spec-022