use super::{ports::*, domain::*};
use std::error::Error;

// ADR-2026-05-19-0721: Password hashing and JWT issuance via ports
pub struct AuthUseCase {
    password_hasher: Box<dyn PasswordHasherPort>,
    token_issuer: Box<dyn TokenIssuerPort>,
    user_repo: Box<dyn UserRepoPort>,
    reducer_call: Box<dyn ReducerCallPort>,
}

impl AuthUseCase {
    pub fn new(
        password_hasher: Box<dyn PasswordHasherPort>,
        token_issuer: Box<dyn TokenIssuerPort>,
        user_repo: Box<dyn UserRepoPort>,
        reducer_call: Box<dyn ReducerCallPort>,
    ) -> Self {
        AuthUseCase { password_hasher, token_issuer, user_repo, reducer_call }
    }

    pub fn register_user(&self, username: String, password: String) -> Result<User, Box<dyn Error>> {
        validate_username(&username)?;
        validate_password(&password)?;

        let hashed_password = self.password_hasher.hash(password)?;
        let user = User::new(username, hashed_password);
        self.reducer_call.register_user(user.clone())?;
        self.user_repo.save(user.clone())?;

        Ok(user)
    }

    pub fn login(&self, username: String, password: String) -> Result<AuthResult, Box<dyn Error>> {
        if let Some(user) = self.user_repo.find_by_username(&username)? {
            if self.password_hasher.verify(password, &user.hashed_password)? {
                let token = self.token_issuer.issue_token(&user.username)?;
                return Ok(AuthResult::new(token, user.username));
            }
        }

        Err("Invalid username or password".into())
    }
}

fn validate_username(username: &str) -> Result<(), Box<dyn Error>> {
    if username.len() < 3 || username.len() > 20 {
        return Err("Username must be between 3 and 20 characters long".into());
    }
    Ok(())
}

fn validate_password(password: &str) -> Result<(), Box<dyn Error>> {
    if password.len() < 6 {
        return Err("Password must be at least 6 characters long".into());
    }
    Ok(())
}

#[derive(Debug)]
pub struct AuthResult {
    pub token: String,
    pub username: String,
    // expires_at field can be added when JWT expiry logic is implemented
}

impl AuthResult {
    fn new(token: String, username: String) -> Self {
        AuthResult { token, username }
    }
}