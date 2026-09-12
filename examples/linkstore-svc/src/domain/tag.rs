//! A label a person puts on a bookmark.

/// Why a piece of text is not a valid tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagError {
    Empty,
    ControlCharacter,
    TooLong,
}

impl std::fmt::Display for TagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TagError::Empty => write!(f, "the tag is empty"),
            TagError::ControlCharacter => write!(f, "the tag has a control character"),
            TagError::TooLong => write!(f, "the tag is longer than 64 characters"),
        }
    }
}

/// A label, trimmed and lowercased so `Rust` and `rust` are one tag.
///
/// Spaces are allowed. `machine learning` is a tag a person really writes,
/// and nothing in this system needs it to be one word.
///
/// Control characters are rejected, and that rejection is load-bearing
/// elsewhere: the storage adapter joins tags with ASCII 31, so the join can
/// never be ambiguous.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tag(String);

impl Tag {
    pub const MAX_CHARS: usize = 64;

    pub fn parse(raw: &str) -> Result<Self, TagError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(TagError::Empty);
        }
        if trimmed.chars().any(|c| c.is_control()) {
            return Err(TagError::ControlCharacter);
        }
        if trimmed.chars().count() > Self::MAX_CHARS {
            return Err(TagError::TooLong);
        }
        Ok(Tag(trimmed.to_lowercase()))
    }

    /// Rebuild a value that was already judged. Callers must have already
    /// validated this value. Only the storage adapter may call it.
    pub fn rehydrate(value: String) -> Self {
        Tag(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
