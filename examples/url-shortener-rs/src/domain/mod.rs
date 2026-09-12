//! Domain — value objects and entities. Pure data, no I/O.
//!
//! Rule 1: `domain/` imports only `domain/`. Nothing here may reach for a
//! port, a use case, or an adapter. That is what makes it testable without
//! wiring anything up.

pub mod code;
pub mod error;
pub mod hash;
pub mod mapping;
pub mod time;
pub mod url;

pub use code::{decode, encode, CodeError, CodeWidth, ShortCode, ALPHABET, BITS_PER_CHAR};
pub use error::ShortenError;
pub use hash::{candidates, fnv1a_64, splitmix64_finalise, FNV_OFFSET_BASIS, FNV_PRIME, MAX_ATTEMPTS};
pub use mapping::Mapping;
pub use time::{Timestamp, Ttl};
pub use url::{LongUrl, UrlError, MAX_URL_BYTES};

/// How many times something has happened. Never negative, by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Count(u64);

impl Count {
    pub const ZERO: Count = Count(0);

    /// The next count. Saturates rather than wrapping: a counter that silently
    /// restarts at zero is worse than one that stops.
    pub fn next(self) -> Count {
        Count(self.0.saturating_add(1))
    }

    pub fn value(self) -> u64 {
        self.0
    }
}
