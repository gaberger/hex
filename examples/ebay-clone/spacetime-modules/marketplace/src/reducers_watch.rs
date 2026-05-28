use spacetime_modules::marketplace::state::{Auction, Watchlist};
use spacetime_sdk::context::Context;
use spacetime_sdk::error::Error;
use spacetime_sdk::storage::Storage;

/// Toggles a watch on a listing for the current user.
///
/// This reducer adds or removes an entry in the Watchlist table to track which auctions
/// a user is watching. It checks if the auction exists before toggling the watch status.
///
/// # Arguments
///
/// * `ctx` - The execution context containing sender information and timestamp.
/// * `auction_id` - The unique identifier for the auction being watched or unwatched.
///
/// # Returns
///
/// A result indicating success or failure of the operation.
pub fn watch_listing(ctx: &Context, auction_id: u64) -> Result<(), Error> {
    // Load the current user's identity from the context
    let user_identity = ctx.sender();

    // Attempt to load the existing watchlist entry for this user and auction
    match Watchlist::load(user_identity, auction_id)? {
        Some(_) => {
            // If the entry exists, remove it (unwatch)
            Watchlist::remove(user_identity, auction_id)?;
        }
        None => {
            // Otherwise, create a new entry to watch the auction
            Watchlist::insert(Watchlist { user: user_identity, auction: auction_id })?;
        }
    }

    Ok(())
}

/// ADR-2026-05-19-0721
/// spacetime-modules/marketplace/src/reducers_watch.rs
```