//! Secondary adapters — driven by the application.
//!
//! Rule 4: an adapter imports `ports/` only, never another adapter. Rule 5 is
//! the other half: nothing imports *this* except the composition root.

use crate::ports::{CounterStore, CountValue as Count};

/// A counter store that keeps the count in memory.
///
/// Swap this for a file or a database by writing another `CounterStore` and
/// changing one line in the composition root. No use case changes.
#[derive(Debug, Default)]
pub struct InMemoryCounterStore {
    count: Count,
}

impl CounterStore for InMemoryCounterStore {
    fn load(&self) -> Count {
        self.count
    }
    fn save(&mut self, count: Count) {
        self.count = count;
    }
}
