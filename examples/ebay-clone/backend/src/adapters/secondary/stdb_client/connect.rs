use adapters::secondary::stdb_client::connection::{StdbClient, connect};

pub fn establish_connection() -> StdbClient {
    connect()
}