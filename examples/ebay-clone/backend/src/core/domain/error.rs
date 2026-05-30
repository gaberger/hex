use thiserror::Error;

/// DomainError represents all possible domain layer errors.
#[derive(Debug, Error)]
pub enum DomainError {
    /// The username is already taken.
    #[error("Username '{0}' is already taken.")]
    UsernameTaken(String),

    /// The username provided is invalid.
    #[error("Invalid username: '{0}'.")]
    InvalidUsername(String),

    /// The title length does not meet the requirements.
    #[error("Title length must be between 5 and 100 characters.")]
    InvalidTitleLength,

    /// The starting price for the auction is invalid.
    #[error("Invalid starting price. Must be greater than zero.")]
    InvalidStartingPrice,

    /// The duration provided for the auction is invalid.
    #[error("Invalid auction duration.")]
    InvalidDuration,

    /// A bid was placed that was too low.
    #[error("Bid must be higher than current highest bid.")]
    BidTooLow,

    /// An attempt was made to place a bid on an ended auction.
    #[error("Auction has already ended.")]
    AuctionEnded,

    /// The user tried to bid on their own auction.
    #[error("Self-bidding is forbidden.")]
    SelfBidForbidden,
}

// ADR-2026-05-19-0721