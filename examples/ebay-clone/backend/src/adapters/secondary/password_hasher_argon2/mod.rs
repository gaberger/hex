use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use async_trait::async_trait;
use rand::rngs::OsRng;

use crate::core::domain::DomainError;
use crate::core::ports::password_hasher::PasswordHasherPort;

// Implements PasswordHasherPort with argon2id, a random per-user salt and the
// crate-default (OWASP-recommended) parameters.
//
// This adapter conforms to the `core::ports::password_hasher::PasswordHasherPort`
// contract exactly: async methods taking `String` by value and returning
// `Result<_, DomainError>`. The adapter does NOT define its own port trait —
// the port is the source of truth (hex rule 4). Argon2 0.5 dropped the old
// `Config` / `hash_encoded` API in favour of the `password_hash` trait family.
// docs/specs/ebay-spec-004
pub struct PasswordHasherArgon2 {
    argon2: Argon2<'static>,
}

impl PasswordHasherArgon2 {
    pub fn new() -> Self {
        PasswordHasherArgon2 {
            argon2: Argon2::default(),
        }
    }
}

impl Default for PasswordHasherArgon2 {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PasswordHasherPort for PasswordHasherArgon2 {
    async fn hash_password(&self, password: String) -> Result<String, DomainError> {
        let salt = SaltString::generate(&mut OsRng);
        self.argon2
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|e| DomainError::Internal(format!("failed to hash password: {e}")))
    }

    async fn verify_password(&self, password: String, hash: String) -> Result<bool, DomainError> {
        let parsed_hash = PasswordHash::new(&hash)
            .map_err(|e| DomainError::Internal(format!("invalid password hash: {e}")))?;
        Ok(self
            .argon2
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok())
    }
}

// docs/specs/ebay-spec-004