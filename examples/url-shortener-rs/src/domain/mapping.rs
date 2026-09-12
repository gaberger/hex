//! One row of the book: an address, and the moment its ticket stops working.

use crate::domain::time::Timestamp;
use crate::domain::url::LongUrl;

/// A stored address and its death time.
///
/// The row holds `expires_at`, not `created_at`. That single choice removes
/// four faults at once: the store never learns what a lifetime is, there is no
/// subtraction to underflow in the first hour, a backwards clock cannot wrap
/// to a huge number and delete data, and two callers cannot disagree about one
/// link because the answer is baked into the row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    url: LongUrl,
    expires_at: Timestamp,
}

impl Mapping {
    pub fn new(url: LongUrl, expires_at: Timestamp) -> Mapping {
        Mapping { url, expires_at }
    }

    pub fn url(&self) -> &LongUrl {
        &self.url
    }

    pub fn expires_at(&self) -> Timestamp {
        self.expires_at
    }

    /// A mapping is live when `now < expires_at`. At the exact moment
    /// `now == expires_at` the mapping is dead. So a lifetime of 100 ms means
    /// the link works for 100 ms and not for 101.
    ///
    /// This is the only place the liveness rule lives.
    pub fn is_live(&self, now: Timestamp) -> bool {
        now < self.expires_at
    }
}
