use crate::core::domain::{DomainError, User};
use crate::core::ports::{
    PasswordHasherPort, ReducerCallPort, RegisterUserInput, TokenIssuerPort,
};

// ADR-2026-05-19-0721: Password hashing and JWT issuance via ports.
//
// Rewritten to conform to the port contracts in `core::ports`:
//   * `PasswordHasherPort::hash_password(String) -> Result<String, DomainError>` (async)
//   * `ReducerCallPort::register_user(RegisterUserInput) -> Result<User, DomainError>` (async)
//   * `TokenIssuerPort::{issue(&User), verify(&str)} -> Result<_, DomainError>` (async)
//
// The previous body called methods that do not exist on these ports
// (`hash`, `find_by_username`, `issue_token`, `User::new`, ...) and leaked a
// stray `code_patch:` directive as source. Those are removed; the use case now
// drives only real port methods.
pub struct AuthUseCase {
    password_hasher: Box<dyn PasswordHasherPort>,
    token_issuer: Box<dyn TokenIssuerPort>,
    reducer_call: Box<dyn ReducerCallPort>,
}

impl AuthUseCase {
    pub fn new(
        password_hasher: Box<dyn PasswordHasherPort>,
        token_issuer: Box<dyn TokenIssuerPort>,
        reducer_call: Box<dyn ReducerCallPort>,
    ) -> Self {
        AuthUseCase {
            password_hasher,
            token_issuer,
            reducer_call,
        }
    }

    /// Registers a new user: hashes the password via the hasher port, then
    /// persists through the register-user reducer, returning the created `User`.
    pub async fn register_user(
        &self,
        username: String,
        email: String,
        password: String,
    ) -> Result<User, DomainError> {
        validate_username(&username)?;
        validate_password(&password)?;

        let password_hash = self.password_hasher.hash_password(password).await?;
        let input = RegisterUserInput {
            username,
            email,
            password: password_hash,
        };
        self.reducer_call.register_user(input).await
    }

    /// Issues an authentication token for an already-resolved user.
    pub async fn issue_token(&self, user: &User) -> Result<String, DomainError> {
        self.token_issuer.issue(user).await
    }

    /// Verifies a token and returns the associated user.
    pub async fn verify_token(&self, token: &str) -> Result<User, DomainError> {
        self.token_issuer.verify(token).await
    }

    /// Verifies a plaintext password against a stored hash via the hasher port.
    pub async fn verify_password(
        &self,
        password: String,
        hash: String,
    ) -> Result<bool, DomainError> {
        self.password_hasher.verify_password(password, hash).await
    }
}

fn validate_username(username: &str) -> Result<(), DomainError> {
    if username.len() < 3 || username.len() > 20 {
        return Err(DomainError::InvalidUsername(username.to_string()));
    }
    Ok(())
}

fn validate_password(password: &str) -> Result<(), DomainError> {
    if password.len() < 6 {
        return Err(DomainError::Internal(
            "password must be at least 6 characters long".to_string(),
        ));
    }
    Ok(())
}