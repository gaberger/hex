use std::sync::Arc;

use super::{ports};

/// AppState holds the ports required by the HTTP handlers.
///
/// docs/specs/ebay-spec-004
pub struct AppState {
    pub item_port: Arc<dyn ports::ItemPort>,
    pub user_port: Arc<dyn ports::UserPort>,
    // Add more ports as necessary for other domains or functionalities
}