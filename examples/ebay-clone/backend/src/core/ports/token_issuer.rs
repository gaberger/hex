use crate::core::domain::*;
use async_trait::async_trait;

#[async_trait]
pub trait TokenIssuerPort: Send + Sync {
    /// Issues a JWT token for a given user.
    ///
    /// # Arguments
    ///
    /// * `user` - A reference to the `User` object for which the token is issued.
    ///
    /// # Returns
    ///
    /// A `Result` containing the issued token as a `String` or an error if the issuance failed.
    async fn issue(&self, user: &User) -> Result<String, DomainError>;

    /// Verifies the validity of a given JWT token.
    ///
    /// # Arguments
    ///
    /// * `token` - The JWT token to verify as a `String`.
    ///
    /// # Returns
    ///
    /// A `Result` containing the `User` object if the token is valid or an error if verification failed.
    async fn verify(&self, token: &str) -> Result<User, DomainError>;
}

// Example DTOs (Data Transfer Objects)
#[derive(Debug, Clone)]
pub struct TokenIssuerInput {
    pub user_id: UserId,
    // Add other necessary fields here as per the spec
}