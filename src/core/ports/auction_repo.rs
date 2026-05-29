// ADR-2026-05-19-0721: Implement AuctionRepo to handle auction data storage and retrieval

use core::ports::listing_repo::{ListingRepo, ListingRepoImpl, Listing};
use core::usecases::bidding::{BiddingUsecase, fetch_bids};
use hex_core::types::AuctionId; // Correct type from the available workspace exports
use async_trait::async_trait; // Required attribute for async trait implementation

pub trait AuctionRepo {
    fn new() -> Self;
    fn create_auction(&self, listing: Listing);
    fn get_auctions_by_user(&self, user_id: String) -> Vec<Listing>;
}

pub struct AuctionRepoImpl {
    // ADR-2026-05-19-0721: Use ListingRepoImpl to manage auction listings
    listing_repo: ListingRepoImpl,
}

impl AuctionRepo for AuctionRepoImpl {
    fn new() -> Self {
        AuctionRepoImpl {
            listing_repo: ListingRepoImpl::new(),
        }
    }

    fn create_auction(&self, listing: Listing) {
        // ADR-2026-05-19-0721: Delegate auction creation to ListingRepo
        self.listing_repo.create_listing(listing);
    }

    fn get_auctions_by_user(&self, user_id: String) -> Vec<Listing> {
        // ADR-2026-05-19-0721: Fetch listings by user from ListingRepo
        self.listing_repo.get_listings_by_user(user_id)
    }
}