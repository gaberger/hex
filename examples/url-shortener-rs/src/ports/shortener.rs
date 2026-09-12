//! What the application offers, and how it is driven.

use crate::domain::{LongUrl, ShortCode, ShortenError};

/// The answer to one `shorten` call.
///
/// `created` is false when the address already had a live code. A caller that
/// polls therefore learns it changed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortened {
    pub code: ShortCode,
    pub created: bool,
}

/// The application, seen from outside.
///
/// A primary adapter holds one of these and never sees a lifetime, a width, a
/// clock or a store.
pub trait Shortener: Send + Sync {
    fn shorten(&self, raw_url: &str) -> Result<Shortened, ShortenError>;
    fn resolve(&self, raw_code: &str) -> Result<Option<LongUrl>, ShortenError>;
    fn expire(&self) -> usize;

    /// How many links are live right now.
    ///
    /// This sits on the port because a test must be able to count them without
    /// reaching around the port into a concrete store.
    fn live_count(&self) -> usize;
}

/// A driver that turns lines of text into calls on the application.
pub trait Console {
    fn handle(&self, line: &str) -> String;
}
