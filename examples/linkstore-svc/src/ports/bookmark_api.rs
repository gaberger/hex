//! The driving port: what the outside world may ask this application to do.
//!
//! Two decisions here are load-bearing.
//!
//! The calls take `&str`, not domain types. So the HTTP adapter never builds a
//! domain value, and never needs to import the domain to do it.
//!
//! `ApiError` is a typed enum, not a string. If it were a string, the HTTP
//! adapter would have to choose a status code by reading the message text, and
//! rewording an error would silently change the contract.

pub use crate::domain::bookmark::Bookmark as BookmarkValue;

#[derive(Debug)]
pub enum ApiError {
    /// The caller sent something the rules refuse. The message is safe to show.
    Invalid(String),
    NotFound,
    /// Something failed underneath. The reason is logged, never returned.
    Unavailable,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Invalid(why) => write!(f, "invalid: {why}"),
            ApiError::NotFound => write!(f, "not found"),
            ApiError::Unavailable => write!(f, "unavailable"),
        }
    }
}

impl std::error::Error for ApiError {}

pub trait BookmarkApi: Send + Sync + 'static {
    fn create(&self, raw_url: &str, title: &str, tags: &[String])
        -> Result<BookmarkValue, ApiError>;
    fn get(&self, id: &str) -> Result<BookmarkValue, ApiError>;
    fn list_by_tag(&self, tag: &str, limit: Option<u32>) -> Result<Vec<BookmarkValue>, ApiError>;
    fn delete(&self, id: &str) -> Result<(), ApiError>;
}
