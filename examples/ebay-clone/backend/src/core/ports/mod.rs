// Placeholder for core ports definitions

// ADR-2026-05-19-0721: Ports and Adapters Architecture Decision Record
pub mod user_repo;
pub mod listing_repo;
pub mod auction_repo;
pub mod bid_repo;
pub mod watch_repo;
pub mod reducer_call;
pub mod image_store;
pub mod password_hasher;
pub mod token_issuer;
pub mod clock;

// ADR-2026-05-19-0721: Ports and Adapters Architecture Decision Record
use crate::core::domain::*;

#[async_trait::async_trait]
pub trait UserRepoPort: Send + Sync {
    async fn create_user(&self, user: CreateUserInput) -> Result<User>;
    async fn read_user_by_id(&self, id: UserId) -> Result<Option<User>>;
    async fn update_user(&self, user: UpdateUserInput) -> Result<User>;
    async fn delete_user(&self, id: UserId) -> Result<()>;
}

#[async_trait::async_trait]
pub trait ListingRepoPort: Send + Sync {
    async fn get_listing_by_id(&self, listing_id: ListingId) -> Result<Option<Listing>>;
    async fn list_listings(&self, filters: ListingFilters) -> Result<Vec<Listing>>;
}

#[async_trait::async_trait]
pub trait AuctionRepoPort: Send + Sync {
    async fn get_auction_by_id(&self, auction_id: AuctionId) -> Result<Option<Auction>>;
    async fn list_auctions(&self, filters: AuctionFilters) -> Result<Vec<Auction>>;
}

#[async_trait::async_trait]
pub trait BidRepoPort: Send + Sync {
    async fn get_bid_by_id(&self, bid_id: BidId) -> Result<Option<Bid>>;
    async fn list_bids_for_auction(&self, auction_id: AuctionId) -> Result<Vec<Bid>>;
}

#[async_trait::async_trait]
pub trait WatchRepoPort: Send + Sync {
    async fn is_watching(&self, user_id: UserId, listing_id: ListingId) -> Result<bool>;
    async fn list_watchers_for_listing(&self, listing_id: ListingId) -> Result<Vec<UserId>>;
}

#[async_trait::async_trait]
pub trait ReducerCallPort: Send + Sync {
    async fn register_user(&self, input: RegisterUserInput) -> Result<User>;
    async fn create_listing(&self, input: CreateListingInput) -> Result<Listing>;
    async fn place_bid(&self, input: PlaceBidInput) -> Result<Bid>;
    async fn watch_listing(&self, user_id: UserId, listing_id: ListingId) -> Result<()>;
}

#[async_trait::async_trait]
pub trait ImageStorePort: Send + Sync {
    async fn store_image(&self, sha256: Sha256Hash, data: Vec<u8>) -> Result<()>;
    async fn get_image_by_sha256(&self, sha256: Sha256Hash) -> Result<Option<Vec<u8>>>;
}

#[async_trait::async_trait]
pub trait PasswordHasherPort: Send + Sync {
    async fn hash_password(&self, password: String) -> Result<String>;
    async fn verify_password(&self, password: String, hash: String) -> Result<bool>;
}

#[async_trait::async_trait]
pub trait TokenIssuerPort: Send + Sync {
    async fn issue_token(&self, user_id: UserId) -> Result<String>;
    async fn verify_token(&self, token: String) -> Result<UserId>;
}

#[async_trait::async_trait]
pub trait ClockPort: Send + Sync {
    async fn now_unix_millis(&self) -> Result<u128>;
}