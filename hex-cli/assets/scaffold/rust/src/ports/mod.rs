//! Ports — the contracts between layers.
//!
//! Rule 2: `ports/` imports `domain/` only, for value types. A port names
//! *what* is needed, never *how* it is done — so a port that mentions a
//! database, a URL, or a file path has already leaked.

use crate::domain::Count;

/// Re-exported so an adapter needs to import nothing but this module.
///
/// Rule 4 is that a secondary adapter imports `ports/` **only** — not
/// `domain/`. Without this line an adapter cannot name the type its own port
/// signature uses, and every implementation reaches past the contract into the
/// domain. The port is the adapter's whole world, so the port hands it over.
pub use crate::domain::Count as CountValue;

/// Somewhere a count can be kept.
///
/// Note what is absent: no table, no path, no connection string. An in-memory
/// map and a Postgres row satisfy this identically, which is the point.
pub trait CounterStore {
    fn load(&self) -> Count;
    fn save(&mut self, count: Count);
}
