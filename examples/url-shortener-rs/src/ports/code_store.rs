//! The place mappings are kept.

use crate::domain::{LongUrl, Mapping, ShortCode, Timestamp};

/// What `bind` decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bind {
    /// This address already had a live code here. Nothing changed.
    Existing(ShortCode),
    /// A slot was claimed for this address.
    Created(ShortCode),
    /// Every candidate is held by a live mapping for another address.
    Exhausted,
}

/// Somewhere `code -> mapping` rows are kept.
///
/// There is one book, not two. There is no `url -> code` map, because two
/// books that must agree can disagree, and a stale back-pointer sends a
/// visitor to somebody else's website. Sameness comes from the hash instead.
///
/// Do not add a `get` and a `put` to this trait. Two calls leave a gap.
/// Another thread acts inside that gap, and one address loses its code. Every
/// method here is one whole decision.
pub trait CodeStore: Send + Sync {
    /// One call. One lock. The whole decision.
    ///
    /// The store is handed `expires_at` already worked out, so it never learns
    /// what a lifetime is. It only compares two numbers.
    fn bind(
        &self,
        url: &LongUrl,
        candidates: &[ShortCode],
        now: Timestamp,
        expires_at: Timestamp,
    ) -> Bind;

    /// The address behind a live code. A dead row answers `None`, so
    /// correctness never waits for a sweep to run.
    fn lookup(&self, code: &ShortCode, now: Timestamp) -> Option<LongUrl>;

    /// Drop dead rows and say how many went. This frees memory only.
    fn sweep(&self, now: Timestamp) -> usize;

    /// How many rows are live right now.
    ///
    /// The name says exactly what it counts, so no test can pass on an answer
    /// nobody wrote down. A bare `len` would count dead rows too.
    fn live_count(&self, now: Timestamp) -> usize;
}

/// Named here so a store implementation can build a row without reaching past
/// this contract into the domain.
pub type Row = Mapping;
