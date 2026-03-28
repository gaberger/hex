use std::sync::{Arc, Mutex, RwLock};
use std::collections::HashMap;
use std::time::Duration;
use std::env;
use anyhow::{Result, anyhow};
use axum::{Router, routing::get, extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;
use tokio::sync::{Arc, Mutex, RwLock};
use tokio::time::Instant;

#[derive(Debug, Clone)]
struct Config {
    name: String,
    port: u16,
    version: String,
}

#[derive(Debug, Clone)]
struct Health {
    status: String,
    uptime: Duration,
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config {
        name: env!("CARGO_PKG_NAME").to_string(),
        port: 3000,
        version: env!("CARGO_PKG_VERSION").to_string(),
    };

    let server = create_server(&config).await?;
    server.start().await?;
    Ok(())
}

async fn create_server(config: &Config) -> Result<Server> {
    let health = Health {
        status: "OK".to_string(),
        uptime: Duration::ZERO,
    };
    Ok(Server { config: config.clone(), health })
}

struct Server {
    config: Config,
    health: Health,
}

impl Server {
    async fn start(self) -> Result<()> {
        let config = Arc::new(Mutex::new(self.config));
        let health = Arc::new(Mutex::new(self.health));

        let app = Router::new()
            .route("/", get(root_handler))
            .route("/health", get(health_handler))
            .route("/config", get(config_handler))
            .layer(
                State(Arc::new(RwLock::new(config))),
            );

        println!("🚀 {} v{} listening on port {}", config.lock().unwrap().name, config.lock().unwrap().version, 3000);
        println!("✅ Health check endpoint: http://localhost:3000");
        println!("✅ Config endpoint: http://localhost:3000/config");
        Ok(())
    }
}

async fn root_handler(State(state): State<Config>) -> impl IntoResponse {
    (StatusCode::OK, Json(json!({
        "message": "Welcome to the API",
        "service": state.name,
        "version": state.version
    })))
}

async fn health_handler(State(_state): State<Config>) -> impl IntoResponse {
    let start = Duration::from_secs(match get_start_time() {
        Some(t) => t,
        None => 0,
    });
    let uptime = Instant::now().duration_since(Instant::now() - start);
    (StatusCode::OK, Json(json!({
        "status": "OK",
        "uptime": format!("{} seconds", uptime.as_secs())
    })))
}

async fn config_handler(State(state): State<Config>) -> impl IntoResponse {
    (StatusCode::OK, Json(json!({
        "name": state.name,
        "port": state.port,
        "version": state.version
    })))
}

fn get_start_time() -> Option<u64> {
    std::env::var("START_TIME").ok().and_then(|s| s.parse().ok())
}