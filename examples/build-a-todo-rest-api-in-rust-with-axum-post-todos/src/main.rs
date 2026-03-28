use std::collections::HashMap;
use std::time::Duration;
use anyhow::{Result, anyhow};

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

fn create_server(config: &Config) -> Result<Server> {
    let health = Health {
        status: "OK".to_string(),
        uptime: Duration::ZERO,
    };
    Ok(Server { config, health })
}

struct Server {
    config: Config,
    health: Health,
}

impl Server {
    async fn start(self) -> Result<()> {
        let config = self.config.clone();
        let health = Arc::new(Mutex::new(self.health));

        let app = Router::new()
            .route("/", get(root_handler))
            .route("/health", get(health_handler))
            .route("/config", get(config_handler))
            .layer(
                State(Arc::new(RwLock::new(config))),
            );

        println!("🚀 {} v{} listening on port {}", config.name, config.version, config.port);
        println!("✅ Health check endpoint: http://localhost:{}", config.port);
        println!("✅ Config endpoint: http://localhost:{}/config", config.port);
        Ok(())
    }
}

async fn root_handler(State(state): State<Config>) -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({
        "message": "Welcome to the API",
        "service": state.name,
        "version": state.version
    })))
}

async fn health_handler(State(state): State<Config>) -> impl IntoResponse {
    let start = Duration::from_secs(match get_start_time() {
        Some(t) => t,
        None => 0,
    });
    let uptime = tokio::time::Instant::now() - start;
    (StatusCode::OK, Json(serde_json::json!({
        "status": "OK",
        "uptime": format!("{} seconds", uptime.as_secs()))
    })))
}

async fn config_handler(State(state): State<Config>) -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({
        "name": state.name,
        "port": state.port,
        "version": state.version
    })))
}

fn get_start_time() -> Option<u64> {
    std::env::var("START_TIME").ok().and_then(|s| s.parse().ok())
}

#[derive(Debug, Clone)]
struct StartInfo {
    name: String,
    version: String,
}

impl StartInfo {
    fn new(name: &str, version: &str) -> StartInfo {
        StartInfo {
            name: name.to_string(),
            version: version.to_string(),
        }
    }
}