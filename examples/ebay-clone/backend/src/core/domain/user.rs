use serde::{Deserialize, Serialize};
use chrono::NaiveDateTime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: u64,
    pub canonical_username: String,
    pub created_at: NaiveDateTime,
}

// docs/specs/ebay-spec-012 defines the structure of a user domain object
// ADR-2026-05-19-0721 specifies the serialization requirements for domain objects