// ADR-2026-05-19-0721

use adapters::secondary::stdb_client::{StdBClient, new as stdb_client_new, execute_query, analyze_data};
use core::ports::reducer_call::{ReducerError};

pub struct BiddingUsecase {
    client: StdBClient,
}

impl BiddingUsecase {
    pub fn new() -> Self {
        BiddingUsecase {
            client: stdb_client_new(),
        }
    }

    pub fn fetch_bids(&self, query: &str) -> Result<Vec<String>, ReducerError> {
        let result = execute_query(&self.client, query);
        match result {
            Ok(data) => Ok(analyze_data(&data)),
            Err(e) => Err(ReducerError::new(format!("Failed to execute query: {}", e))),
        }
    }
}