//! The one error a caller of the application sees.

use crate::domain::code::CodeError;
use crate::domain::url::UrlError;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortenError {
    BadUrl(UrlError),
    BadCode(CodeError),
    /// Every candidate code for this address is held by a live mapping.
    ///
    /// This is temporary, not a ban on one address. When the holders expire,
    /// the same address succeeds again.
    CodeSpaceExhausted,
}

impl fmt::Display for ShortenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShortenError::BadUrl(e) => write!(f, "bad url, {e}"),
            ShortenError::BadCode(e) => write!(f, "bad code, {e}"),
            ShortenError::CodeSpaceExhausted => write!(f, "code space exhausted"),
        }
    }
}

impl std::error::Error for ShortenError {}
