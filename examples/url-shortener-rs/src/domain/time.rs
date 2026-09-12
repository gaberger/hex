//! Time as two plain numbers of milliseconds.
//!
//! `Timestamp::plus` is the only piece of arithmetic in the whole system.
//! Everything else is a `<` comparison. Do not add a second one.

/// Milliseconds since 1970-01-01 UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Timestamp(u64);

impl Timestamp {
    pub const ZERO: Timestamp = Timestamp(0);

    pub fn from_millis(ms: u64) -> Timestamp {
        Timestamp(ms)
    }

    pub fn millis(self) -> u64 {
        self.0
    }

    /// The moment a mapping created now must die.
    ///
    /// `saturating_add`, so a huge clock plus a huge lifetime stops at the end
    /// of time instead of wrapping round to the start of it.
    pub fn plus(self, ttl: Ttl) -> Timestamp {
        Timestamp(self.0.saturating_add(ttl.millis()))
    }
}

/// How long a mapping lives, in milliseconds. Always greater than zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ttl(u64);

impl Ttl {
    /// `None` when `ms` is 0. A lifetime of zero means the link is dead the
    /// instant it is made, which is never what a caller wants.
    pub fn from_millis(ms: u64) -> Option<Ttl> {
        if ms == 0 {
            None
        } else {
            Some(Ttl(ms))
        }
    }

    pub fn millis(self) -> u64 {
        self.0
    }
}
