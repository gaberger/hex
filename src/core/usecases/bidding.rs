// ADR-2026-05-19-0721

use adapters::secondary::stdb_client::{StdBClient, new, execute_query, analyze_data};

pub struct BiddingUsecase {
    client: StdBClient,
}

impl BiddingUsecase {
    pub fn new() -> Self {
        BiddingUsecase {
            client: new(),
        }
    }

    pub fn fetch_bids(&self, query: &str) -> Result<Vec<String>, String> {
        let result = execute_query(&self.client, query);
        match result {
            Ok(data) => Ok(analyze_data(&data)),
            Err(e) => Err(format!("Failed to execute query: {}", e)),
        }
    }
}