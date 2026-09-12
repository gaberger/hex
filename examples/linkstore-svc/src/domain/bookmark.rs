//! The bookmark itself, and the small values it is made of.

use crate::domain::tag::Tag;
use crate::domain::url::NormalisedUrl;

/// Why a piece of text is not a valid identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdError {
    Malformed,
}

/// Why a piece of text is not a valid title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TitleError {
    Empty,
    ControlCharacter,
    TooLong,
}

impl std::fmt::Display for TitleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TitleError::Empty => write!(f, "the title is empty"),
            TitleError::ControlCharacter => write!(f, "the title has a control character"),
            TitleError::TooLong => write!(f, "the title is longer than 512 characters"),
        }
    }
}

/// The identity of one bookmark: a lowercase hyphenated UUID.
///
/// There is no generator here. A random number is a side effect, and the
/// domain stays pure — the storage adapter mints the value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BookmarkId(String);

impl BookmarkId {
    /// Accept 36 characters in 8-4-4-4-12 groups, lowercase hex only.
    ///
    /// `GET /bookmarks/{id}` hands you text and nothing else, so this is the
    /// door every read comes through.
    pub fn parse(raw: &str) -> Result<Self, IdError> {
        if raw.len() != 36 {
            return Err(IdError::Malformed);
        }
        for (index, character) in raw.chars().enumerate() {
            let allowed = match index {
                8 | 13 | 18 | 23 => character == '-',
                _ => matches!(character, '0'..='9' | 'a'..='f'),
            };
            if !allowed {
                return Err(IdError::Malformed);
            }
        }
        Ok(BookmarkId(raw.to_string()))
    }

    /// Rebuild a value that was already judged. Callers must have already
    /// validated this value. Only the storage adapter may call it.
    pub fn rehydrate(value: String) -> Self {
        BookmarkId(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a person calls the link.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Title(String);

impl Title {
    pub const MAX_CHARS: usize = 512;

    pub fn parse(raw: &str) -> Result<Self, TitleError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(TitleError::Empty);
        }
        if trimmed.chars().any(|c| c.is_control()) {
            return Err(TitleError::ControlCharacter);
        }
        if trimmed.chars().count() > Self::MAX_CHARS {
            return Err(TitleError::TooLong);
        }
        Ok(Title(trimmed.to_string()))
    }

    /// Rebuild a value that was already judged. Callers must have already
    /// validated this value. Only the storage adapter may call it.
    pub fn rehydrate(value: String) -> Self {
        Title(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A moment, counted in milliseconds since 1970, UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

impl Timestamp {
    pub fn from_millis(millis: i64) -> Self {
        Timestamp(millis)
    }

    /// Rebuild a value that was already judged. Callers must have already
    /// validated this value.
    pub fn rehydrate(millis: i64) -> Self {
        Timestamp(millis)
    }

    pub fn millis(self) -> i64 {
        self.0
    }
}

/// One saved link, with everything known about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    id: BookmarkId,
    url: NormalisedUrl,
    title: Title,
    created_at: Timestamp,
    tags: Vec<Tag>,
}

impl Bookmark {
    /// Assemble a bookmark from values that are already typed.
    ///
    /// There is nothing to re-judge here: every field arrived as a value type,
    /// and a value type can only exist if its own rule ran or a storage
    /// adapter rehydrated it. The tags are sorted so two equal bookmarks
    /// compare equal whatever order the rows came back in.
    pub fn rehydrate(
        id: BookmarkId,
        url: NormalisedUrl,
        title: Title,
        created_at: Timestamp,
        tags: Vec<Tag>,
    ) -> Self {
        let mut tags = tags;
        tags.sort();
        tags.dedup();
        Bookmark { id, url, title, created_at, tags }
    }

    pub fn id(&self) -> &BookmarkId {
        &self.id
    }
    pub fn url(&self) -> &NormalisedUrl {
        &self.url
    }
    pub fn title(&self) -> &Title {
        &self.title
    }
    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }
    pub fn tags(&self) -> &[Tag] {
        &self.tags
    }
}
