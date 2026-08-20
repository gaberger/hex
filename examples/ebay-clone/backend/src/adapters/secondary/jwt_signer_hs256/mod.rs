use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const TTL_DURATION: Duration = Duration::from_secs(24 * 60 * 60); // 24 hours

#[derive(Debug, Serialize, Deserialize)]
pub struct JwtClaims {
    sub: String,
    exp: usize,
}

pub trait TokenIssuerPort {
    fn issue_token(&self, username: &str) -> Result<String, jwt_signer_error::Error>;
}

pub struct JwtSignerHs256 {
    secret_key: String,
}

impl JwtSignerHs256 {
    pub fn new(secret_key: String) -> Self {
        JwtSignerHs256 { secret_key }
    }
}

mod jwt_signer_error {
    use thiserror::Error;

    #[derive(Error, Debug)]
    pub enum Error {
        #[error("JWT encoding error: {0}")]
        Encoding(#[from] jsonwebtoken::errors::Error),
    }
}

impl TokenIssuerPort for JwtSignerHs256 {
    fn issue_token(&self, username: &str) -> Result<String, jwt_signer_error::Error> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("System time is before UNIX EPOCH!")
            .as_secs() as usize;

        let claims = JwtClaims {
            sub: username.to_string(),
            exp: now + TTL_DURATION.as_secs() as usize,
        };

        let header = Header::new(Algorithm::HS256);
        let token = encode(
            &header,
            &claims,
            &EncodingKey::from_secret(self.secret_key.as_ref()),
        )?;

        Ok(token)
    }
}

// docs/specs/ebay-spec-024