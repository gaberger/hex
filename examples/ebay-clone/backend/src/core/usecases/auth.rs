use super::ports::{PasswordHasherPort, ReducerCallPort, TokenIssuerPort, UserRepoPort};
use crate::domain::models::{AuthResult, NewUser, User};

pub struct AuthService<PWH: PasswordHasherPort, RCP: ReducerCallPort, TIP: TokenIssuerPort, URP: UserRepoPort> {
    password_hasher: PWH,
    reducer_call: RCP,
    token_issuer: TIP,
    user_repo: URP,
}

impl<PWH, RCP, TIP, URP> AuthService<PWH, RCP, TIP, URP>
where
    PWH: PasswordHasherPort,
    RCP: ReducerCallPort,
    TIP: TokenIssuerPort,
    URP: UserRepoPort,
{
    pub fn new(password_hasher: PWH, reducer_call: RCP, token_issuer: TIP, user_repo: URP) -> Self {
        AuthService {
            password_hasher,
            reducer_call,
            token_issuer,
            user_repo,
        }
    }

    pub fn register_user(&self, username: String, password: String) -> Result<User, String> {
        // Validate username and password
        if username.is_empty() || password.len() < 8 {
            return Err(String::from("Invalid username or password"));
        }

        let hashed_password = self.password_hasher.hash(password);
        let new_user = NewUser {
            username,
            password: hashed_password,
        };

        // Register user via ReducerCallPort
        self.reducer_call.register_user(new_user.clone())?;

        // Fetch the created row from UserRepoPort
        self.user_repo.get_by_username(&new_user.username)
    }

    pub fn login(&self, username: String, password: String) -> Result<AuthResult, String> {
        let user = match self.user_repo.get_by_canonical_username(&username) {
            Ok(user) => user,
            Err(_) => return Err(String::from("User not found")),
        };

        // Verify password
        if !self.password_hasher.verify(&password, &user.password) {
            return Err(String::from("Incorrect password"));
        }

        // Issue JWT token
        let token = self.token_issuer.issue_token(&username);
        let expires_at = self.token_issuer.expiry_time();

        Ok(AuthResult {
            token,
            username: user.username.clone(),
            expires_at,
        })
    }
}

// docs/workplans/feat-ebay-mvp.json