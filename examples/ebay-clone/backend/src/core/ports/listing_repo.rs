use crate::core::domain::*;
use async_trait::async_trait;

/// Search/filter criteria for listing queries.
///
/// Defined in the port layer so adapters depend inward on the port, never the
/// reverse — a port must not import from `adapters::*` (hex rule).
#[derive(Debug, Clone, Default)]
pub struct SearchListingsParams {
    pub query: Option<String>,
    pub min_price_cents: Option<u64>,
    pub max_price_cents: Option<u64>,
}

/// ListingRepoPort defines read-only operations on listings.
///
/// Listings are created and updated via reducers, so this port is strictly for
/// querying. The create-side DTO lives in `reducer_call` (`CreateListingInput`)
/// — it was previously duplicated here, producing an ambiguous glob re-export
/// from `ports/mod.rs` (two `CreateListingInput`). Removed to resolve it.
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

/// Errors that can occur when interacting with the Listing repository.
#[derive(Debug)]
pub enum ListingRepoError {
    /// An error occurred while trying to fetch a listing.
    FetchError(String),
    /// The requested listing was not found.
    NotFound,
    /// An internal error occurred.
    InternalError(String),
}