use crate::core::domain::*;
use async_trait::async_trait;

/// BidRepoPort defines a read-only interface for accessing bids.
///
/// # Spec References:
/// - docs/specs/ebay-spec-001.md
/// - docs/specs/ebay-spec-004.md
/// - docs/specs/ebay-spec-006.md
/// - docs/specs/ebay-spec-019.md
/// - docs/specs/ebay-spec-023.md
///
/// # Design Rationale:
/// This trait is designed to be used by the core logic of the application, abstracting away
/// any specifics of how bids are stored or retrieved. It ensures that all bid-related operations
/// can be performed asynchronously and do not leak domain types outside of the `core::domain` module.
///
/// ADR-2026-05-19-0721: Use async-trait for repository interfaces to allow asynchronous operations.
#[async_trait]
pub trait BidRepoPort: Send + Sync {
    /// Fetches a bid by its unique identifier.
    ///
    /// # Arguments
    /// * `bid_id` - The unique identifier of the bid.
    ///
    /// # Returns
    /// An `Option<Bid>` containing the bid if found, otherwise `None`.
    async fn get_bid_by_id(&self, bid_id: BidId) -> Option<Bid>;

    /// Fetches all bids placed on a specific listing.
    ///
    /// # Arguments
    /// * `listing_id` - The unique identifier of the listing.
    ///
    /// # Returns
    /// A vector of `Bid` objects associated with the specified listing.
    async fn get_bids_by_listing_id(&self, listing_id: ListingId) -> Vec<Bid>;

    /// Fetches all bids placed by a specific user.
    ///
    /// # Arguments
    /// * `user_id` - The unique identifier of the user.
    ///
    /// # Returns
    /// A vector of `Bid` objects placed by the specified user.
    async fn get_bids_by_user_id(&self, user_id: UserId) -> Vec<Bid>;
}