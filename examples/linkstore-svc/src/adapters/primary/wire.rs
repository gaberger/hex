//! The shapes that go over the wire.
//!
//! These live here, not in the domain. JSON field names are a transport
//! detail: a domain type carrying serde attributes means renaming a field
//! silently changes what every client receives.

use crate::ports::BookmarkValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    /// Optional on purpose. A missing tag gets this adapter's own 400, not
    /// axum's rejection, so the client always sees the same error shape.
    pub tag: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct BookmarkResponse {
    pub id: String,
    pub url: String,
    pub title: String,
    pub created_at: i64,
    pub tags: Vec<String>,
}

impl BookmarkResponse {
    /// Read the bookmark through its accessors. The adapter never builds a
    /// domain value, and never needs to import the domain to read one.
    pub fn from_port(bookmark: &BookmarkValue) -> Self {
        BookmarkResponse {
            id: bookmark.id().as_str().to_string(),
            url: bookmark.url().as_str().to_string(),
            title: bookmark.title().as_str().to_string(),
            created_at: bookmark.created_at().millis(),
            tags: bookmark.tags().iter().map(|tag| tag.as_str().to_string()).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
}

impl ErrorBody {
    pub fn new(message: impl Into<String>) -> Self {
        ErrorBody { error: message.into() }
    }
}
