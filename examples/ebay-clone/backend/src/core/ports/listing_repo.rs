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

/// Search criteria for listing queries.
///
/// Owned by the port (not the HTTP adapter) so the contract does not depend
/// on a primary adapter's request DTO — primary adapters map their inbound
/// query parameters into this type before calling the port.
#[derive(Debug, Clone, Default)]
pub struct SearchListingsParams {
    pub query: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// DTO for creating a new listing.
#[derive(Debug)]
pub struct CreateListingInput {
    pub title: ListingTitle,
    pub description: String,
    pub starting_price: Money,
    pub duration: DurationMs,
}

/// Errors that can occur when interacting with the listing repository.
#[derive(Debug, thiserror::Error)]
pub enum ListingRepoError {
    #[error("Database error: {0}")]
    DbError(String),
    #[error("Invalid input: {0}")]
    InvalidInput(String),
}