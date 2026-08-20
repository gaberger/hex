use async_trait::async_trait;
use crate::core::domain::*;

#[async_trait]
pub trait WatchRepoPort: Send + Sync {
    /// Retrieve all listings being watched by a given user.
    ///
    /// # Parameters
    /// - `user_id`: The ID of the user whose watches are to be retrieved.
    ///
    /// # Returns
    /// A vector of listing IDs that the specified user is watching.
    async fn get_watched_listings(&self, user_id: UserId) -> Result<Vec<ListingId>, DomainError>;

    /// Check if a user is watching a specific listing.
    ///
    /// # Parameters
    /// - `user_id`: The ID of the user.
    /// - `listing_id`: The ID of the listing.
    ///
    /// # Returns
    /// `true` if the user is watching the listing, `false` otherwise.
    async fn is_watching(&self, user_id: UserId, listing_id: ListingId) -> Result<bool, DomainError>;
}

// docs/specs/ebay-spec-019