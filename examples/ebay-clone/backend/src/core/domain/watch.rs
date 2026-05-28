use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// WatchEntry struct representing a user's watchlist entry for an auction.
///
/// This struct captures the essential information needed to track when a user is watching an auction,
/// including the auction ID and the time at which the watch was added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchEntry {
    /// Unique identifier for the auction being watched.
    pub auction_id: String,

    /// The timestamp of when the auction was added to the user's watchlist.
    pub added_at: SystemTime,
}

// docs/workplans/feat-ebay-mvp.json
// hex-core/src/core/domain/watch.rs
// This file defines the domain model for managing watch entries in an eBay-like application.