use hex_core::types::AuctionId; // Assuming AuctionId is defined in hex-core

#[async_trait]
pub trait AuctionRepo {
    async fn get_auction(&self, auction_id: AuctionId) -> Result<Auction, AuctionError>;
    async fn create_auction(&self, auction: NewAuction) -> Result<AuctionId, AuctionError>;
    async fn update_auction(&self, auction_id: AuctionId, update: AuctionUpdate) -> Result<(), AuctionError>;
    async fn delete_auction(&self, auction_id: AuctionId) -> Result<(), AuctionError>;
}

#[derive(Debug)]
pub enum AuctionError {
    NotFound,
    Conflict,
    InternalError,
}