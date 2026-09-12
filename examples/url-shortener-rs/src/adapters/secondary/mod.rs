//! Secondary adapters — driven by the application.
//!
//! Rule 4: an adapter imports `ports/` only, never another adapter. Rule 5 is
//! the other half: nothing imports *this* except the composition root.

use crate::ports::{CounterStore, CountValue as Count};

pub mod in_memory_code_store;
pub mod manual_clock;
pub mod system_clock;

pub use in_memory_code_store::InMemoryCodeStore;
pub use manual_clock::ManualClock;
pub use system_clock::SystemClock;

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
