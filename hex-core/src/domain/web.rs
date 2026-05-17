//! Web search + fetch domain types (ADR-2604142100: hex-native web search).
//!
//! These value objects describe search results, fetched pages, and the
//! provider/format options consumed by the primary/secondary adapters that
//! implement web search and HTTP fetch. hex-core deliberately keeps zero
//! runtime dependencies beyond serde/thiserror, so URLs are represented as
//! plain `String` here — adapters are responsible for parsing/validation.

use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

/// String representation of a URL. Parsing/validation is the responsibility
/// of adapters (e.g. via the `url` crate) — the domain stays dependency-free.
pub type Url = String;

/// Which external web-search provider fulfilled a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WebProvider {
    Brave,
    Tavily,
    SerpApi,
    DuckDuckGo,
}

/// One entry in a search-result list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    pub url: Url,
    pub title: String,
    pub snippet: String,
    /// 1-based rank assigned by the provider (lower = more relevant).
    pub rank: u32,
}

/// Options controlling a search query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchOptions {
    /// Maximum number of results to return.
    pub limit: u32,
    /// Optional BCP-47 locale hint (e.g. `"en-US"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    /// Whether the provider's safe-search filter should be enabled.
    pub safe_search: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            limit: 10,
            locale: None,
            safe_search: true,
        }
    }
}

/// Desired body format of a fetched page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FetchFormat {
    /// Raw response body, no post-processing.
    Raw,
    /// HTML stripped to readable text.
    Text,
    /// HTML converted to Markdown.
    Markdown,
}

/// Options for a single page fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchOptions {
    pub format: FetchFormat,
    /// Maximum number of bytes to read from the response body.
    pub max_bytes: usize,
    /// Request timeout. Serialized as `{ secs, nanos }` by default.
    pub timeout: Duration,
    /// Whether the fetch layer may serve this URL from cache.
    pub cache: bool,
}

impl Default for FetchOptions {
    fn default() -> Self {
        Self {
            format: FetchFormat::Markdown,
            max_bytes: 2 * 1024 * 1024,
            timeout: Duration::from_secs(20),
            cache: true,
        }
    }
}

/// The outcome of fetching a single URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchedPage {
    pub url: Url,
    /// HTTP status code (e.g. `200`, `404`).
    pub status: u16,
    /// `Content-Type` header, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// Response body as text (decoded per the response's charset).
    pub text: String,
    /// Markdown rendering of `text`, populated when
    /// [`FetchOptions::format`] is [`FetchFormat::Markdown`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
    /// When the fetch completed.
    pub fetched_at: SystemTime,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_options_default_is_safe_and_bounded() {
        let o = SearchOptions::default();
        assert_eq!(o.limit, 10);
        assert!(o.safe_search);
        assert!(o.locale.is_none());
    }

    #[test]
    fn fetch_options_default_is_markdown_with_timeout() {
        let o = FetchOptions::default();
        assert_eq!(o.format, FetchFormat::Markdown);
        assert!(o.cache);
        assert_eq!(o.timeout, Duration::from_secs(20));
    }

    #[test]
    fn web_provider_roundtrips_as_lowercase_json() {
        let s = serde_json::to_string(&WebProvider::DuckDuckGo).unwrap();
        assert_eq!(s, "\"duckduckgo\"");
        let back: WebProvider = serde_json::from_str(&s).unwrap();
        assert_eq!(back, WebProvider::DuckDuckGo);
    }

    #[test]
    fn search_result_roundtrips() {
        let r = SearchResult {
            url: "https://example.com/a".into(),
            title: "A".into(),
            snippet: "about a".into(),
            rank: 1,
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: SearchResult = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }
}
