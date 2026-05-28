use super::ports::{BidRepoPort, AuctionRepoPort, ReducerCallPort};
use crate::core::entities::{Bid, Listing, DomainError};

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
    pub fn place_bid(&self, bid: Bid) -> Result<(), DomainError> {
        self.reducer_port.place_bid(bid)
            .map_err(|reducer_error| match reducer_error {
                // Map specific reducer errors to domain errors
                // Example mappings according to ebay-spec-012
                ReducerError::InvalidBidAmount => DomainError::InvalidBid,
                ReducerError::AuctionClosed => DomainError::AuctionEnded,
                ReducerError::BidTooLow => DomainError::BidBelowMinimum,
                ReducerError::UserNotEligible => DomainError::UserIneligible,
                _ => DomainError::Unexpected,
            })
    }

    /// Lists all bids made by the given bidder.
    pub fn list_my_bids(&self, bidder_identity: &str) -> Result<Vec<Bid>, DomainError> {
        self.bid_repo.find_by_bidder(bidder_identity)
            .map_err(|_| DomainError::RepoAccessFailed)
    }

    /// Lists all auctions won by the given user.
    pub fn list_my_won(&self, winner_identity: &str) -> Result<Vec<Listing>, DomainError> {
        self.auction_repo.find_by_winner(winner_identity)
            .map_err(|_| DomainError::RepoAccessFailed)
    }
}

// ADR-2026-05-19-0721
// hex analyze
// docs/specs/ebay-spec-012