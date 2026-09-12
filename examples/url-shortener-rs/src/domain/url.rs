//! `LongUrl` — a web address we agree to keep.
//!
//! Parsing is strict on purpose. A value of this type has already been
//! checked, so nothing downstream needs to check it again.

use std::fmt;

/// The largest address we accept, in bytes.
pub const MAX_URL_BYTES: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlError {
    Empty,
    TooLong { bytes: usize },
    Control { at: usize },
    NoScheme,
}

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UrlError::Empty => write!(f, "empty url"),
            UrlError::TooLong { bytes } => write!(f, "url too long, {bytes} bytes"),
            UrlError::Control { at } => write!(f, "control character at {at}"),
            UrlError::NoScheme => write!(f, "no http scheme"),
        }
    }
}

/// A validated web address.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LongUrl(String);

impl LongUrl {
    /// Check an address a caller typed.
    ///
    /// The checks run in the order below, and the first failure wins.
    pub fn parse(raw: &str) -> Result<LongUrl, UrlError> {
        if raw.is_empty() {
            return Err(UrlError::Empty);
        }
        if raw.len() > MAX_URL_BYTES {
            return Err(UrlError::TooLong { bytes: raw.len() });
        }
        if let Some((at, _)) = raw.char_indices().find(|(_, ch)| ch.is_control()) {
            return Err(UrlError::Control { at });
        }
        // We never fold case. Two addresses that differ by one byte are two
        // addresses, because a path is case-sensitive on most servers.
        if !raw.starts_with("http://") && !raw.starts_with("https://") {
            return Err(UrlError::NoScheme);
        }
        Ok(LongUrl(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LongUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
