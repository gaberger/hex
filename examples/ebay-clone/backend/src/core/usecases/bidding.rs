use super::ports::{BidRepoPort, AuctionRepoPort, ReducerCallPort};
use crate::core::domain::{Listing, DomainError};

/// Use case for handling bidding actions.
pub struct BiddingUseCase {
    bid_repo: Box<dyn BidRepoPort>,
    auction_repo: Box<dyn AuctionRepoPort>,
    reducer_port: Box<dyn ReducerCallPort>,
}

impl BiddingUseCase {
    pub fn new(bid_repo: Box<dyn BidRepoPort>, auction_repo: Box<dyn AuctionRepoPort>, reducer_port: Box<dyn ReducerCallPort>) -> Self {
        BiddingUseCase { bid_repo, auction_repo, reducer_port }
    }

    /// Places a bid on an auction.
    pub fn place_bid(&self, bid: crate::core::domain::Bid) -> Result<(), DomainError> {
        self.reducer_port.place_bid(bid)
            .map_err(|_| DomainError::Unexpected)
    }

    /// Lists all bids made by the given bidder.
    pub fn list_my_bids(&self, bidder_identity: &str) -> Result<Vec<crate::core::domain::Bid>, DomainError> {
        self.bid_repo.find_by_bidder(bidder_identity)
            .map_err(|_| DomainError::RepoAccessFailed)
    }

    /// Lists all auctions won by the given user.
    pub fn list_my_won(&self, winner_identity: &str) -> Result<Vec<Listing>, DomainError> {
        self.auction_repo.find_by_winner(winner_identity)
            .map_err(|_| DomainError::RepoAccessFailed)
    }

    /// Toggles a listing in the user's watchlist.
    pub fn toggle_watch(&self, listing_id: &str, bidder_identity: &str) -> Result<(), DomainError> {
        self.reducer_port.watch_listing(listing_id, bidder_identity)
            .map_err(|_| DomainError::Unexpected)
    }

    /// Lists all listings created by the given seller.
    pub fn list_my_listings(&self, seller_identity: &str) -> Result<Vec<Listing>, DomainError> {
        self.auction_repo.find_by_seller(seller_identity)
            .map_err(|_| DomainError::RepoAccessFailed)
    }
}

// ADR-2026-05-19-0721