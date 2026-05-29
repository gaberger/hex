pub mod ids;
pub mod money;
pub mod username;
pub mod title;
pub mod time;
pub mod error;
pub mod listing;
pub mod auction;
pub mod bid;
pub mod user;
pub mod image;
pub mod watch;

// Flat re-exports so `crate::core::domain::UserId` etc. resolve for the
// usecases + adapters layers, which the persona-authored imports assume.
// Without these every usecase + adapter file hit E0425 "cannot find type
// X in this scope" — 80+ of the 139 compile errors collapse to import
// resolution once these are re-exported at the module root.
pub use ids::{UserId, ListingId, BidId};
pub use money::Money;
pub use username::Username;
pub use title::ListingTitle;
pub use time::{Timestamp, DurationMs};
pub use error::DomainError;
pub use listing::Listing;
pub use auction::{Auction, AuctionStatus};
pub use bid::Bid;
pub use user::User;
pub use image::ImageRef;
pub use watch::WatchEntry;
