use crate::core::domain::*;
use async_trait::async_trait;
// use core::time::DurationMs; // ADR-2026-05-19-0721
use adapters::primary::http_axum::handlers_listings::SearchListingsParams;

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
    pub start_price: Money,
    pub duration: DurationMs,
}

/// Errors that can occur when interacting with the ListingRepo.
#[derive(Debug)]
pub enum ListingRepoError {
    /// The listing ID provided is invalid.
    InvalidListingId(ListingIdValidationError),
    /// The user ID provided is invalid.
    InvalidUserId(UserIdValidationError),
    /// Other domain-specific errors.
    DomainError(DomainError),
}

impl From<ListingIdValidationError> for ListingRepoError {
    fn from(err: ListingIdValidationError) -> Self {
        ListingRepoError::InvalidListingId(err)
    }
}

impl From<UserIdValidationError> for ListingRepoError {
    fn from(err: UserIdValidationError) -> Self {
        ListingRepoError::InvalidUserId(err)
    }
}

impl From<DomainError> for ListingRepoError {
    fn from(err: DomainError) -> Self {
        ListingRepoError::DomainError(err)
    }
}