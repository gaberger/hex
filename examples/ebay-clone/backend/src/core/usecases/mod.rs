// Placeholder for use cases module

// ADR-2023-10-01-1435: Initial structuring of core use cases
pub mod auction_creation;
pub mod auction_listing;
pub mod bidding_process;
pub mod user_authentication;
pub mod user_registration;

// New modules as per the workplan
pub mod search_auctions;
pub mod view_auction_details;
pub mod payment_processing;

// Added for auth use case implementation - docs/workplans/feat-ebay-mvp.json
pub mod auth;