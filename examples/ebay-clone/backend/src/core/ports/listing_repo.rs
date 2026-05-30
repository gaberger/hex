use crate::core::domain::*;
use async_trait::async_trait;
// use core::time::DurationMs; // ADR-2026-05-19-0721

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

/// Search parameters for listing repository queries.
#[derive(Debug)]
pub struct SearchListingsParams {
    pub title: Option<String>,
    pub min_price: Option<Money>,
    pub max_price: Option<Money>,
    // Add other search criteria fields as needed
}

/// Input structure for creating a new listing.
#[derive(Debug)]
pub struct CreateListingInput {
    pub title: ListingTitle,
    pub description: String,
    pub price: Money,
    // Add other fields as needed
}

/// Errors that can occur in the listing repository.
#[derive(Debug, thiserror::Error)]
pub enum ListingRepoError {
    #[error("Listing not found")]
    NotFound,

    #[error("Invalid input data: {0}")]
    InvalidInput(String),

    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),
}

// Implement other necessary methods and logic for the listing repository.