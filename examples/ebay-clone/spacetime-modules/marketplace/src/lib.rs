register_user(ctx, username: String, _password_unused: String), create_listing(ctx, title, description, starting_price_cents: u64, duration_ms: i64, image_sha256s: Vec<String>); all 7 spec failure modes return distinct error strings; no panics — every error path returns Err
mod reducers_bidding;
mod reducers_auction;
mod reducers_watch;

// Example usage of the reducers:
// use self::reducers_bidding::place_bid;
// use self::reducers_auction::close_auction;
// use self::reducers_watch::watch_listing;

// spacetime-modules/marketplace/src/lib.rs
docs/specs/ebay-spec-012, docs/specs/ebay-spec-013, docs/specs/ebay-spec-014, docs/specs/ebay-spec-015, docs/specs/ebay-spec-016, docs/specs/ebay-spec-017, docs/specs/ebay-spec-018