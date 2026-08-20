use crate::core::domain::*;
use async_trait::async_trait;
use std::fmt;

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
}

/// DTO describing the query criteria for `get_listings_by_criteria`.
///
/// The use-case layer (`usecases::listings::search_listings`) builds this from
/// the optional query string + pagination params, so the field set must stay in
/// sync with that caller.
#[derive(Debug, Clone, Default)]
pub struct SearchListingsParams {
    /// Free-text query, already normalized (lowercased) by the use case.
    pub query: Option<String>,
    /// Maximum number of results to return.
    pub limit: Option<u32>,
    /// Number of leading results to skip (pagination offset).
    pub offset: Option<u32>,
}

/// Errors that can occur when interacting with the Listing repository.
#[derive(Debug)]
pub enum ListingRepoError {
    /// An error occurred while trying to fetch a listing.
    FetchError,
    /// The requested listing was not found.
    NotFound,
}

impl fmt::Display for ListingRepoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ListingRepoError::FetchError => write!(f, "failed to fetch listing"),
            ListingRepoError::NotFound => write!(f, "listing not found"),
        }
    }
}

impl std::error::Error for ListingRepoError {}