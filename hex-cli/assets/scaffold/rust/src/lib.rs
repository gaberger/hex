//! {{name}} — a hexagonal skeleton that runs.
//!
//! `lib.rs` is the **composition root**: the only place allowed to name a
//! concrete adapter. Everything else depends on the port.
//!
//! ```text
//!   domain  ←  ports  ←  usecases
//!                ↑
//!           adapters (secondary)
//!                ↑
//!        lib.rs — wires them, once
//! ```
//!
//! Check it with `hex analyze .`.

pub mod adapters;
pub mod domain;
pub mod ports;
pub mod usecases;

use adapters::secondary::InMemoryCounterStore;
use domain::Count;

/// Build the application with its real adapters.
///
/// The one line below is the whole composition decision. Change
/// `InMemoryCounterStore` to a file-backed store and nothing else moves.
pub fn counter() -> impl ports::CounterStore {
    InMemoryCounterStore::default()
}

/// Run the use case against a freshly composed application.
pub fn increment_once() -> Count {
    let mut store = counter();
    usecases::increment(&mut store)
}
