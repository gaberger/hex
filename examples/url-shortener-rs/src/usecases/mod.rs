//! Use cases — the application's verbs.
//!
//! Rule 3: `usecases/` imports `domain/` and `ports/` only. It takes the port
//! as a parameter and never chooses which adapter fills it; that choice
//! belongs to the composition root alone.

pub mod expire;
pub mod resolve;
pub mod shorten;

pub use expire::{expire, live_count};
pub use resolve::resolve;
pub use shorten::shorten;

use crate::domain::{Count, CodeWidth, LongUrl, ShortenError, Ttl};
use crate::ports::{Clock, CodeStore, CounterStore, Shortened, Shortener};

/// Advance the count by one and return the new value.
pub fn increment(store: &mut dyn CounterStore) -> Count {
    let next = store.load().next();
    store.save(next);
    next
}

/// The three verbs, with their store, their clock and their policy held
/// together.
///
/// It owns `S` and `C` by value. That avoids lifetimes and makes the struct
/// `Send + Sync` on its own, because both port traits already require it.
pub struct Shortening<S: CodeStore, C: Clock> {
    store: S,
    clock: C,
    ttl: Ttl,
    width: CodeWidth,
}

impl<S: CodeStore, C: Clock> Shortening<S, C> {
    pub fn new(store: S, clock: C, ttl: Ttl, width: CodeWidth) -> Shortening<S, C> {
        Shortening { store, clock, ttl, width }
    }
}

impl<S: CodeStore, C: Clock> Shortener for Shortening<S, C> {
    fn shorten(&self, raw_url: &str) -> Result<Shortened, ShortenError> {
        shorten(&self.store, &self.clock, self.ttl, self.width, raw_url)
    }

    fn resolve(&self, raw_code: &str) -> Result<Option<LongUrl>, ShortenError> {
        resolve(&self.store, &self.clock, raw_code)
    }

    fn expire(&self) -> usize {
        expire(&self.store, &self.clock)
    }

    fn live_count(&self) -> usize {
        live_count(&self.store, &self.clock)
    }
}
