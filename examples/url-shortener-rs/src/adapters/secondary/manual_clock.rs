//! A clock a test drives by hand.

use crate::ports::{Clock, Timestamp};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Time held still until you move it.
///
/// Clone it and both copies share one number, so a test keeps a handle after
/// it hands the clock to the application.
#[derive(Debug, Default, Clone)]
pub struct ManualClock {
    millis: Arc<AtomicU64>,
}

impl ManualClock {
    pub fn at_millis(ms: u64) -> ManualClock {
        ManualClock { millis: Arc::new(AtomicU64::new(ms)) }
    }

    pub fn set_millis(&self, ms: u64) {
        self.millis.store(ms, Ordering::SeqCst);
    }

    /// Move time forward. A test seam, so a plain add is enough: no test goes
    /// near the end of a `u64`.
    pub fn advance_millis(&self, delta: u64) {
        self.millis.fetch_add(delta, Ordering::SeqCst);
    }

    pub fn millis(&self) -> u64 {
        self.millis.load(Ordering::SeqCst)
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_millis(self.millis.load(Ordering::SeqCst))
    }
}
