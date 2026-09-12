//! A code store that keeps its rows in memory.

use crate::ports::{Bind, CodeStore, LongUrl, Mapping, ShortCode, Timestamp};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

/// One lock around one map.
///
/// Operations serialise. I will say the cost plainly, because it is a real
/// cost: two threads never work at the same time in here. This is a library
/// core, not a web server. A clever shard scheme lost a mapping in the design
/// this replaced. One lock cannot.
#[derive(Debug, Default)]
pub struct InMemoryCodeStore {
    slots: Mutex<HashMap<ShortCode, Mapping>>,
}

impl InMemoryCodeStore {
    /// Take the lock, and take it back after a panic.
    ///
    /// A panic while a thread holds a Rust `Mutex` poisons it, and every later
    /// `unwrap` then panics too. One bad caller would brick the store until
    /// restart. Be honest about today: the map is private and no caller runs
    /// code inside the lock, so nothing can poison it. This is insurance
    /// against a future change.
    fn guard(&self) -> MutexGuard<'_, HashMap<ShortCode, Mapping>> {
        self.slots.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl CodeStore for InMemoryCodeStore {
    fn bind(
        &self,
        url: &LongUrl,
        candidates: &[ShortCode],
        now: Timestamp,
        expires_at: Timestamp,
    ) -> Bind {
        let mut slots = self.guard();

        // Pass one: does this address already have a live code here?
        //
        // Skipping this pass gives one address two live codes, because a
        // freed earlier candidate would be claimed while a later one still
        // holds the same address.
        for code in candidates {
            if slots.get(code).is_some_and(|row| row.is_live(now) && row.url() == url) {
                return Bind::Existing(code.clone());
            }
        }

        // Pass two: the first slot that is empty or dead.
        //
        // Empty, dead-and-ours (a renewal) and dead-and-another's (reuse after
        // expiry) are all the same action. Live-and-another's moves on. The
        // end of the list, and an empty list, mean exhausted.
        for code in candidates {
            let free = slots.get(code).is_none_or(|row| !row.is_live(now));
            if free {
                slots.insert(code.clone(), Mapping::new(url.clone(), expires_at));
                return Bind::Created(code.clone());
            }
        }

        Bind::Exhausted
    }

    fn lookup(&self, code: &ShortCode, now: Timestamp) -> Option<LongUrl> {
        self.guard()
            .get(code)
            .filter(|row| row.is_live(now))
            .map(|row| row.url().clone())
    }

    fn sweep(&self, now: Timestamp) -> usize {
        let mut slots = self.guard();
        // Count inside the closure. A `before - after` subtraction would be a
        // second piece of arithmetic, and this design has exactly one.
        let mut removed = 0usize;
        slots.retain(|_, row| {
            if row.is_live(now) {
                true
            } else {
                removed += 1;
                false
            }
        });
        removed
    }

    fn live_count(&self, now: Timestamp) -> usize {
        self.guard().values().filter(|row| row.is_live(now)).count()
    }
}
