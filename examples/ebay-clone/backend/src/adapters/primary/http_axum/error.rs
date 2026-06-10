use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use std::fmt;

// DomainError is assumed to be defined elsewhere in the project.
pub enum DomainError {
    InvalidInput,
    NotFound,
    Unauthorized,
    // Add other domain-specific errors as needed.
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DomainError::InvalidInput => write!(f, "Invalid input"),
            DomainError::NotFound => write!(f, "Not found"),
            DomainError::Unauthorized => write!(f, "Unauthorized"),
            // Add other variants as needed.
        }
    }
}

impl IntoResponse for DomainError {
    fn into_response(self) -> Response {
        let status_code = match self {
            DomainError::InvalidInput => StatusCode::BAD_REQUEST,
            DomainError::NotFound => StatusCode::NOT_FOUND,
            DomainError::Unauthorized => StatusCode::UNAUTHORIZED,
            // Map other domain errors to appropriate HTTP status codes.
        };
        (status_code, self.to_string()).into_response()
    }
}

// docs/specs/ebay-spec-023
// ADR-2026-05-19-0721