use axum::{
    extract::Json,
    routing::{post},
    Router,
};
use serde::{Deserialize, Serialize};
use tower_cookies::Cookies;

use crate::core::ports::{
    user_repo_port::UserRepoPort,
    password_hasher_port::PasswordHasherPort,
};

// ADR-2026-05-19-0721

#[derive(Serialize)]
struct AuthResponse {
    token: String,
    username: String,
    expires_at: i64,
}

#[derive(Deserialize)]
struct RegisterRequest {
    username: String,
    password: String,
}

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

async fn register(
    Json(payload): Json<RegisterRequest>,
    user_repo: UserRepoPort,
    password_hasher: PasswordHasherPort,
) -> Result<(), (StatusCode, String)> {
    let hashed_password = password_hasher.hash(&payload.password)?;
    if user_repo.create_user(&payload.username, &hashed_password).await.is_err() {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "Failed to register user".to_string()));
    }
    Ok(())
}

async fn login(
    Json(payload): Json<LoginRequest>,
    cookies: Cookies,
    user_repo: UserRepoPort,
    password_hasher: PasswordHasherPort,
) -> Result<(StatusCode, Json<AuthResponse>), (StatusCode, String)> {
    let stored_password = match user_repo.get_user(&payload.username).await {
        Some(user) => user.password,
        None => return Err((StatusCode::UNAUTHORIZED, "User not found".to_string())),
    };

    if !password_hasher.verify(&payload.password, &stored_password)? {
        return Err((StatusCode::UNAUTHORIZED, "Invalid password".to_string()));
    }

    // Generate JWT token and expiration time
    let token = "generated_jwt_token"; // Replace with actual token generation logic
    let expires_at = 1684934400; // Example expiration timestamp

    Ok((
        StatusCode::OK,
        Json(AuthResponse {
            token: token.to_string(),
            username: payload.username.clone(),
            expires_at,
        }),
    ))
}

pub fn auth_routes(
    user_repo: UserRepoPort,
    password_hasher: PasswordHasherPort,
) -> Router {
    Router::new()
        .route("/api/v1/auth/register", post(register))
        .route("/api/v1/auth/login", post(login))
        .with_state((user_repo, password_hasher))
}

// docs/specs/ebay-spec-001
// docs/specs/ebay-spec-002
// docs/specs/ebay-spec-004
// docs/specs/ebay-spec-005