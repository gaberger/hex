//! Web search & fetch ports (ADR-2604142100: hex-native web search).
//!
//! Two driven-adapter contracts: `WebSearchPort` for query-to-results, and
//! `WebFetchPort` for URL-to-page. Domain types live in
//! [`crate::domain::web`] and stay free of HTTP-client dependencies; each
//! adapter is responsible for transport concerns.

use async_trait::async_trait;

use crate::domain::web::{
    FetchOptions, FetchedPage, SearchOptions, SearchResult, Url, WebProvider,
};

/// Driven-adapter contract for a web-search provider (Brave, Tavily,
/// SerpAPI, DuckDuckGo, …).
///
/// Implementations MUST be `Send + Sync` so the composition root can hold
/// them behind `Arc<dyn WebSearchPort>` inside a broker fallback chain.
#[async_trait]
pub trait WebSearchPort: Send + Sync {
    /// Execute a query and return ranked results. `opts.limit` is an upper
    /// bound — providers MAY return fewer results.
    async fn search(
        &self,
        query: &str,
        opts: SearchOptions,
    ) -> Result<Vec<SearchResult>, WebError>;

    /// Which provider this adapter talks to. Used for audit-trail logging
    /// and for provider-override routing at the REST layer.
    fn provider(&self) -> WebProvider;
}

/// Driven-adapter contract for fetching a single URL.
///
/// Implementations MUST be `Send + Sync`.
#[async_trait]
pub trait WebFetchPort: Send + Sync {
    /// Fetch `url` and render the body according to `opts.format`.
    /// Respects `opts.max_bytes`, `opts.timeout`, and `opts.cache`.
    async fn fetch(
        &self,
        url: &Url,
        opts: FetchOptions,
    ) -> Result<FetchedPage, WebError>;
}

/// Errors returned by web search & fetch adapters. Variants are designed to
/// be actionable at the broker layer (e.g. `AuthFailure` → skip to next
/// provider, `RateLimited` → back off).
#[derive(Debug, thiserror::Error)]
pub enum WebError {
    /// Provider returned a non-2xx status or an otherwise invalid response.
    #[error("provider error: {0}")]
    ProviderError(String),

    /// Provider signalled rate-limiting (HTTP 429 or equivalent).
    #[error("rate limited by provider")]
    RateLimited,

    /// API key missing, invalid, or revoked.
    #[error("authentication failed")]
    AuthFailure,

    /// Request did not complete before the configured timeout.
    #[error("request timed out")]
    Timeout,

    /// Response body could not be parsed (malformed JSON, unexpected HTML
    /// shape, invalid UTF-8, etc.).
    #[error("parse error: {0}")]
    Parse(String),
}
