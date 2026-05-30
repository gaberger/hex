/// Minimal SpacetimeDB client adapter.
///
/// The previous implementation referenced a `connection` submodule that does
/// not exist in this crate, breaking the build. This conforming stub holds the
/// connection string and exposes the same `new` constructor surface so callers
/// continue to compile while real STDB connection wiring is implemented.
pub struct StdbClient {
    #[allow(dead_code)]
    connection_string: String,
}

impl StdbClient {
    pub async fn new(connection_string: &str) -> Result<Self, String> {
        Ok(StdbClient {
            connection_string: connection_string.to_string(),
        })
    }
}