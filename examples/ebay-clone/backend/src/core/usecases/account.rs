use crate::core::domain::{DomainError, BidderIdentity, WinnerIdentity};
use crate::core::ports::{
    ReducerCallPort,
    BidRepoPort,
    AuctionRepoPort,
    ListingRepoPort,
};

pub struct AccountUseCase {
    reducer: Box<dyn ReducerCallPort>,
    bid_repo: Box<dyn BidRepoPort>,
    auction_repo: Box<dyn AuctionRepoPort>,
    listing_repo: Box<dyn ListingRepoPort>,
}

impl AccountUseCase {
    pub fn new(
        reducer: Box<dyn ReducerCallPort>,
        bid_repo: Box<dyn BidRepoPort>,
        auction_repo: Box<dyn AuctionRepoPort>,
        listing_repo: Box<dyn ListingRepoPort>,
    ) -> Self {
        Self { reducer, bid_repo, auction_repo, listing_repo }
    }

    pub fn place_bid(&self, bidder_identity: BidderIdentity, amount: u64) -> Result<(), DomainError> {
        self.reducer.place_bid(bidder_identity, amount)
            .map_err(|err| map_reducer_error_to_domain(err))
    }

    pub fn toggle_watch(&self, bidder_identity: BidderIdentity, listing_id: u64) -> Result<(), DomainError> {
        self.reducer.watch_listing(bidder_identity, listing_id)
            .map_err(|err| map_reducer_error_to_domain(err))
    }

    pub fn list_my_bids(&self, bidder_identity: BidderIdentity) -> Result<Vec<Bid>, DomainError> {
        self.bid_repo.list_by_bidder(bidder_identity)
            .map_err(|_| DomainError::RepositoryAccessFailed)
    }

    pub fn list_my_won(&self, winner_identity: WinnerIdentity) -> Result<Vec<Auction>, DomainError> {
        self.auction_repo.list_closed_by_winner(winner_identity)
            .map_err(|_| DomainError::RepositoryAccessFailed)
    }

    pub fn list_my_listings(&self, seller_identity: SellerIdentity) -> Result<Vec<Listing>, DomainError> {
        self.listing_repo.list_by_seller(seller_identity)
            .map_err(|_| DomainError::RepositoryAccessFailed)
    }
}

fn map_reducer_error_to_domain(err: ReducerError) -> DomainError {
    match err {
        ReducerError::BidTooLow => DomainError::BidTooLow,
        ReducerError::AuctionClosed => DomainError::AuctionClosed,
        _ => DomainError::Unexpected,
    }
}

// Dummy types and errors to satisfy the code snippet
type Bid = ();
type Auction = ();
type Listing = ();
type SellerIdentity = ();
enum ReducerError { BidTooLow, AuctionClosed, Unexpected }

// docs/specs/ebay-spec-012, ebay-spec-013, ebay-spec-014, ebay-spec-015, ebay-spec-018, ebay-spec-021