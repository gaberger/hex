//! The driven port: somewhere bookmarks can be kept.
//!
//! Note what is absent: no table, no file path, no connection. SQLite and a
//! hash map satisfy this identically, which is the point.

/// Re-exported so an adapter needs to import nothing but this module.
///
/// Rule 4 is that an adapter imports `ports/` **only** — never `domain/`.
/// Without these lines every implementation reaches past the contract into the
/// domain. The port is the adapter's whole world, so the port hands it over.
pub use crate::domain::bookmark::{
    Bookmark as BookmarkValue, BookmarkId as BookmarkIdValue, Timestamp as TimestampValue,
    Title as TitleValue,
};
pub use crate::domain::tag::Tag as TagValue;
pub use crate::domain::url::NormalisedUrl as NormalisedUrlValue;

/// What can go wrong down at the storage end.
#[derive(Debug)]
pub enum StoreError {
    /// The store cannot be reached or cannot be written to right now.
    Unavailable(String),
    /// The store holds something this build does not understand.
    Corrupt(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Unavailable(why) => write!(f, "store unavailable: {why}"),
            StoreError::Corrupt(why) => write!(f, "store corrupt: {why}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// `Send + Sync + 'static` is load-bearing: the store lives behind an `Arc`
/// shared by every request thread, and it is handed to a blocking task.
pub trait BookmarkStore: Send + Sync + 'static {
    /// Save the link, or merge into the one already saved under this URL.
    /// Returns the identity of the row that now holds it.
    fn upsert(
        &self,
        url: &NormalisedUrlValue,
        title: &TitleValue,
        tags: &[TagValue],
        now: TimestampValue,
    ) -> Result<BookmarkIdValue, StoreError>;

    fn get(&self, id: &BookmarkIdValue) -> Result<Option<BookmarkValue>, StoreError>;

    /// Newest first. `limit` is required, not advisory: an unbounded list is
    /// how one popular tag builds a 200,000-object reply on one thread.
    fn list_by_tag(&self, tag: &TagValue, limit: u32) -> Result<Vec<BookmarkValue>, StoreError>;

    fn delete(&self, id: &BookmarkIdValue) -> Result<(), StoreError>;
}
