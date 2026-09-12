//! Drop the rows that have died.

use crate::ports::{Clock, CodeStore};

/// Remove dead rows and say how many went.
///
/// Expiry is lazy first and eager second. A read already refuses a dead row,
/// so correctness never waits for this to run. This frees memory.
pub fn expire(store: &dyn CodeStore, clock: &dyn Clock) -> usize {
    let now = clock.now();
    store.sweep(now)
}

/// How many links are live right now.
pub fn live_count(store: &dyn CodeStore, clock: &dyn Clock) -> usize {
    let now = clock.now();
    store.live_count(now)
}
