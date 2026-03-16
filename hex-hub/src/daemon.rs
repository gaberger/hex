use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockFile {
    pub pid: u32,
    pub port: u16,
    pub token: String,
    pub started_at: String,
    pub version: String,
}

pub fn lock_file_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home)
        .join(".hex")
        .join("daemon")
        .join("hub.lock")
}

pub fn write_lock(port: u16, token: &str) -> std::io::Result<()> {
    let path = lock_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let lock = LockFile {
        pid: std::process::id(),
        port,
        token: token.to_string(),
        started_at: chrono::Utc::now().to_rfc3339(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };

    let json = serde_json::to_string_pretty(&lock)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(&path, json)
}

pub fn remove_lock() {
    let path = lock_file_path();
    let _ = std::fs::remove_file(path);
}

pub fn generate_token() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..16).map(|_| rng.gen()).collect();
    hex_encode(&bytes)
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
