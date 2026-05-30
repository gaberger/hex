use crate::core::domain::*;
use async_trait::async_trait;

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

/// Query parameters for searching listings. Defined at the port boundary so the
/// trait does not depend on any primary adapter (hex rule 2: ports import domain only).
#[derive(Debug, Clone, Default)]
pub struct SearchListingsParams {
    pub q: Option<String>,
    pub active: Option<bool>,
    pub max_price_cents: Option<u64>,
    pub page: u32,
    pub per_page: u32,
}

/// DTO for creating a new listing. This is used as input to the create_listing reducer.
#[derive(Debug, Clone)]
pub struct CreateListingInput {
    pub title: String,
    pub description: Option<String>,
    pub starting_bid: Money,
    pub start_time: Timestamp,
    pub end_time: Timestamp,
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