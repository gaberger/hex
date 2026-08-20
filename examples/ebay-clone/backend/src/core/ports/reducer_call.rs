use crate::core::domain::*;
use async_trait::async_trait;

#[async_trait]
pub trait ReducerCallPort: Send + Sync {
    async fn register_user(&self, input: RegisterUserInput) -> Result<User, DomainError>;
    async fn create_listing(&self, input: CreateListingInput) -> Result<Listing, DomainError>;
    async fn place_bid(&self, input: PlaceBidInput) -> Result<Bid, DomainError>;
    async fn watch_listing(&self, input: WatchListingInput) -> Result<WatchEntry, DomainError>;
}

#[derive(Debug)]
pub struct RegisterUserInput {
    pub username: String,
    pub email: String,
    pub password: String,
}

#[derive(Debug)]
pub struct CreateListingInput {
    pub title: String,
    pub description: String,
    pub starting_price: u64,
    pub end_time: u64, // Unix timestamp
    pub user_id: UserId,
}

#[derive(Debug)]
pub struct PlaceBidInput {
    pub listing_id: ListingId,
    pub amount: u64,
    pub user_id: UserId,
}

#[derive(Debug)]
pub struct WatchListingInput {
    pub listing_id: ListingId,
    pub user_id: UserId,
}