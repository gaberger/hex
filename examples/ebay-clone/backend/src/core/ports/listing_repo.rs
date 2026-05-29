Both errors are clear now:

1. **Line 3** — `core::time::DurationMs` doesn't exist; the real type is `crate::core::domain::time::DurationMs`, already pulled in by the glob `use crate::core::domain::*;` on line 1. The explicit line is broken and redundant, so I remove it (no type is lost — `DurationMs` stays available via the glob).
2. **Line 4** — `adapters::...` fails to resolve because the crate root names it `crate::adapters` (declared in both `lib.rs` and `main.rs`). The path qualifier is the fix; `SearchListingsParams` is `pub` and the type/usage is preserved verbatim.

use crate::core::domain::*;
use async_trait::async_trait;
use crate::adapters::primary::http_axum::handlers_listings::SearchListingsParams; // ADR-2026-05-19-0721

/// ListingRepoPort defines read-only operations on listings.
///
/// Listings are created and updated via reducers, so this port is strictly for querying.
#[async_trait]
pub trait ListingRepoPort: Send + Sync {
    /// Fetches a listing by its unique identifier.
    ///
    /// # Arguments
    /// * `listing_id` - The ID of the listing to retrieve.
    ///
    /// # Returns
    /// A Result containing the retrieved Listing if found, otherwise an error.
    async fn get_listing_by_id(&self, listing_id: &ListingId) -> Result<Listing, ListingRepoError>;

    /// Fetches all listings that match a given set of criteria.
    ///
    /// # Arguments
    /// * `criteria` - The criteria to filter listings by.
    ///
    /// # Returns
    /// A Result containing a Vec of Listings that match the criteria.
    async fn get_listings_by_criteria(&self, criteria: &SearchListingsParams) -> Result<Vec<Listing>, ListingRepoError>;
}

/// DTO for creating a new listing. This is used as input to the create_listing reducer.
#[derive(Debug, Clone)]
pub struct CreateListingInput {
    pub title: String,
    pub description: Option<String>,
    pub starting_bid: Money,
    pub start_time: Timestamp,
    pub end_time: Timestamp,
    // Add other fields as necessary based on ebay-spec-004 and other relevant specifications.
}

/// Error type for ListingRepoPort operations.
#[derive(Debug, thiserror::Error)]
pub enum ListingRepoError {
    #[error("Listing not found")]
    NotFound,

    #[error("Database error: {0}")]
    DatabaseError(String),

    // Add more variants as necessary based on the specifications.
}