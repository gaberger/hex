//! The application's verbs for bookmarks.
//!
//! Rule 3: this imports `domain/` and `ports/` only. It takes both ports as
//! parameters and never chooses which adapter fills them.

use crate::domain::bookmark::{BookmarkId, Title};
use crate::domain::tag::Tag;
use crate::domain::url::NormalisedUrl;
use crate::ports::{ApiError, BookmarkApi, BookmarkStore, BookmarkValue, Clock};
use std::sync::Arc;

/// If a caller asks for no limit, this is what they get.
const DEFAULT_LIMIT: u32 = 100;
/// No caller may ask for more than this, whatever they type.
const MAX_LIMIT: u32 = 500;

pub struct BookmarkService {
    store: Arc<dyn BookmarkStore>,
    clock: Arc<dyn Clock>,
}

impl BookmarkService {
    pub fn new(store: Arc<dyn BookmarkStore>, clock: Arc<dyn Clock>) -> Self {
        BookmarkService { store, clock }
    }
}

/// Everything that fails down at the store becomes one outward answer. The
/// reason is for the log, never for the caller.
fn unavailable<E: std::fmt::Display>(error: E) -> ApiError {
    eprintln!("bookmark store failed: {error}");
    ApiError::Unavailable
}

impl BookmarkApi for BookmarkService {
    fn create(
        &self,
        raw_url: &str,
        title: &str,
        tags: &[String],
    ) -> Result<BookmarkValue, ApiError> {
        let url = NormalisedUrl::parse(raw_url).map_err(|e| ApiError::Invalid(e.to_string()))?;
        let title = Title::parse(title).map_err(|e| ApiError::Invalid(e.to_string()))?;
        let mut parsed_tags = Vec::with_capacity(tags.len());
        for tag in tags {
            parsed_tags.push(Tag::parse(tag).map_err(|e| ApiError::Invalid(e.to_string()))?);
        }

        let now = self.clock.now();
        let id = self.store.upsert(&url, &title, &parsed_tags, now).map_err(unavailable)?;

        // Read it back: the store owns the merge, so only the store knows the
        // full set of tags this bookmark now carries.
        self.store
            .get(&id)
            .map_err(unavailable)?
            .ok_or(ApiError::NotFound)
    }

    fn get(&self, id: &str) -> Result<BookmarkValue, ApiError> {
        // A malformed identity is a missing one. There is no bookmark called
        // `banana`, and telling the caller *why* their guess was badly shaped
        // is not information they need.
        let id = BookmarkId::parse(id).map_err(|_| ApiError::NotFound)?;
        self.store
            .get(&id)
            .map_err(unavailable)?
            .ok_or(ApiError::NotFound)
    }

    fn list_by_tag(&self, tag: &str, limit: Option<u32>) -> Result<Vec<BookmarkValue>, ApiError> {
        let tag = Tag::parse(tag).map_err(|e| ApiError::Invalid(e.to_string()))?;
        let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
        self.store.list_by_tag(&tag, limit).map_err(unavailable)
    }

    fn delete(&self, id: &str) -> Result<(), ApiError> {
        // Deleting a thing that was never there has already achieved the goal.
        let Ok(id) = BookmarkId::parse(id) else {
            return Ok(());
        };
        self.store.delete(&id).map_err(unavailable)
    }
}
