//! The real clock.

use crate::ports::{Clock, Timestamp};
use std::time::{SystemTime, UNIX_EPOCH};

/// Wall-clock milliseconds since 1970.
///
/// Not `Instant`. An `Instant` cannot be written to a file or a database row,
/// so a store that swaps its backing later would find every stored time
/// meaningless.
///
/// The wall clock can jump backwards, and that is safe here. A backwards jump
/// makes `now` smaller, so more links look alive. A bad clock can never delete
/// your data. A forward jump expires links early, which is the acceptable
/// direction to fail in.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        // `u64::try_from`, not `as u64`. A silent truncation here would move a
        // link's death by centuries.
        Timestamp::from_millis(u64::try_from(since_epoch.as_millis()).unwrap_or(u64::MAX))
    }
}
