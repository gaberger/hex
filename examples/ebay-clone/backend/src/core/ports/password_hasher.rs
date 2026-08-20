use crate::core::domain::*;
use async_trait::async_trait;

/// PasswordHasherPort trait defines the interface for hashing and verifying passwords.
/// This implementation should use argon2id for security reasons.
///
/// docs/specs/ebay-spec-019 specifies that password hashing must be done securely with a strong algorithm like argon2id.
#[async_trait]
pub trait PasswordHasherPort: Send + Sync {
    /// Hashes a given plaintext password and returns the hashed value.
    async fn hash_password(&self, password: String) -> Result<String, DomainError>;

    /// Verifies if a given plaintext password matches the provided hashed password.
    async fn verify_password(&self, password: String, hash: String) -> Result<bool, DomainError>;
}