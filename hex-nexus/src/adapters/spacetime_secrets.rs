//! SpacetimeDB-backed secret grant adapter (ADR-026).
//!
//! Implements `ISecretGrantPort` with SpacetimeDB as the sole persistence
//! backend. No in-memory fallback — if SpacetimeDB is down, operations fail
//! with clear errors.
//!
//! Features:
//! - Circuit breaker (3 failures → open for 10s → half-open probe)
//! - Retry on 5xx/timeout (1 retry, no retry on 4xx)
//! - Automatic reconnection with exponential backoff
//! - AES-256-GCM encryption for vault values

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::ports::secret_grant::{
    ISecretGrantPort, SecretBackendHealth, SecretGrant,
};

// ── Circuit Breaker ─────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

struct CircuitBreaker {
    failure_count: AtomicU32,
    threshold: u32,
    last_failure_epoch_ms: AtomicU64,
    cooldown_ms: u64,
}

impl CircuitBreaker {
    fn new(threshold: u32, cooldown: Duration) -> Self {
        Self {
            failure_count: AtomicU32::new(0),
            threshold,
            last_failure_epoch_ms: AtomicU64::new(0),
            cooldown_ms: cooldown.as_millis() as u64,
        }
    }

    fn state(&self) -> CircuitState {
        let failures = self.failure_count.load(Ordering::Relaxed);
        if failures < self.threshold {
            return CircuitState::Closed;
        }
        let last = self.last_failure_epoch_ms.load(Ordering::Relaxed);
        let now = epoch_ms();
        if now - last > self.cooldown_ms {
            CircuitState::HalfOpen
        } else {
            CircuitState::Open
        }
    }

    fn record_success(&self) {
        self.failure_count.store(0, Ordering::Relaxed);
    }

    fn record_failure(&self) {
        self.failure_count.fetch_add(1, Ordering::Relaxed);
        self.last_failure_epoch_ms.store(epoch_ms(), Ordering::Relaxed);
    }

    fn state_name(&self) -> &'static str {
        match self.state() {
            CircuitState::Closed => "closed",
            CircuitState::Open => "open",
            CircuitState::HalfOpen => "half-open",
        }
    }
}

fn epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── Client ──────────────────────────────────────────────────

/// SpacetimeDB-backed implementation of `ISecretGrantPort`.
///
/// All state lives in SpacetimeDB — the local cache is populated from
/// reducer responses and serves as an L1 read cache only.
pub struct SpacetimeSecretClient {
    cache: Arc<RwLock<HashMap<String, SecretGrant>>>,
    host: String,
    database: String,
    hub_id: String,
    connected: Arc<RwLock<bool>>,
    circuit: CircuitBreaker,
    last_error: Arc<RwLock<Option<String>>>,
    http: reqwest::Client,
    vault_key: Option<Vec<u8>>,
}

impl SpacetimeSecretClient {
    pub fn new(host: String, database: String, hub_id: String) -> Self {
        let vault_key = std::env::var("HEX_VAULT_KEY")
            .ok()
            .and_then(|k| {
                let bytes = k.as_bytes().to_vec();
                if bytes.len() >= 32 { Some(bytes[..32].to_vec()) } else {
                    tracing::warn!("HEX_VAULT_KEY must be at least 32 bytes — vault encryption disabled");
                    None
                }
            });

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .pool_max_idle_per_host(4)
            .build()
            .expect("failed to build HTTP client");

        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            host,
            database,
            hub_id,
            connected: Arc::new(RwLock::new(false)),
            circuit: CircuitBreaker::new(3, Duration::from_secs(10)),
            last_error: Arc::new(RwLock::new(None)),
            http,
            vault_key,
        }
    }

    /// Attempt initial connection to SpacetimeDB.
    pub async fn connect(&self) -> bool {
        let url = format!("{}{}", self.host, hex_core::SPACETIMEDB_PING_PATH);
        let healthy = self.http
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);

        *self.connected.write().await = healthy;

        if healthy {
            self.circuit.record_success();
            tracing::info!(host = %self.host, db = %self.database, "SpacetimeDB secret client connected");
        } else {
            tracing::warn!(host = %self.host, "SpacetimeDB secret client failed to connect");
        }

        healthy
    }

    pub async fn is_connected(&self) -> bool {
        *self.connected.read().await
    }

    pub fn cache(&self) -> &Arc<RwLock<HashMap<String, SecretGrant>>> {
        &self.cache
    }

    // ── Reducer call with retry + circuit breaker ──

    async fn call_reducer(
        &self,
        reducer_name: &str,
        args: serde_json::Value,
    ) -> Result<(), String> {
        // Check circuit breaker
        match self.circuit.state() {
            CircuitState::Open => {
                return Err(format!(
                    "Circuit breaker open — SpacetimeDB calls suspended (last {} failures)",
                    self.circuit.failure_count.load(Ordering::Relaxed)
                ));
            }
            CircuitState::HalfOpen => {
                tracing::info!(reducer = %reducer_name, "Circuit half-open — probing SpacetimeDB");
            }
            CircuitState::Closed => {}
        }

        let url = format!(
            "{}/v1/database/{}/call/{}",
            self.host, self.database, reducer_name
        );

        // Try with 1 retry on 5xx/timeout
        for attempt in 0..2u8 {
            match self.http.post(&url).json(&args).send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        self.circuit.record_success();
                        *self.connected.write().await = true;
                        *self.last_error.write().await = None;
                        return Ok(());
                    }

                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();

                    // Don't retry 4xx (client errors)
                    if status.is_client_error() {
                        self.circuit.record_success(); // server is reachable
                        let err = format!("Reducer '{}' returned {}: {}", reducer_name, status, body);
                        *self.last_error.write().await = Some(err.clone());
                        return Err(err);
                    }

                    // 5xx: retry once
                    if attempt == 0 {
                        tracing::warn!(
                            reducer = %reducer_name,
                            status = %status,
                            "SpacetimeDB 5xx — retrying once"
                        );
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        continue;
                    }

                    self.circuit.record_failure();
                    let err = format!("Reducer '{}' returned {}: {}", reducer_name, status, body);
                    *self.last_error.write().await = Some(err.clone());
                    *self.connected.write().await = false;
                    return Err(err);
                }
                Err(e) => {
                    if attempt == 0 && e.is_timeout() {
                        tracing::warn!(
                            reducer = %reducer_name,
                            "SpacetimeDB timeout — retrying once"
                        );
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        continue;
                    }

                    self.circuit.record_failure();
                    *self.connected.write().await = false;
                    let err = format!("SpacetimeDB call failed: {}", e);
                    *self.last_error.write().await = Some(err.clone());
                    return Err(err);
                }
            }
        }

        unreachable!()
    }

    // ── Encryption helpers ──

    fn encrypt_value(&self, plaintext: &str) -> (String, u32) {
        match &self.vault_key {
            Some(key) => {
                use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
                use aes_gcm::aead::Aead;
                use base64::Engine;

                let cipher = Aes256Gcm::new_from_slice(key).expect("invalid key length");
                let mut nonce_bytes = [0u8; 12];
                rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce_bytes);
                let nonce = Nonce::from_slice(&nonce_bytes);

                let ciphertext = cipher
                    .encrypt(nonce, plaintext.as_bytes())
                    .expect("encryption failed");

                // Format: nonce (12 bytes) + ciphertext, base64-encoded
                let mut combined = nonce_bytes.to_vec();
                combined.extend_from_slice(&ciphertext);

                let encoded = base64::engine::general_purpose::STANDARD.encode(&combined);
                (format!("AES256GCM:v1:{}", encoded), 1)
            }
            None => {
                // No encryption key — store plaintext (development mode)
                tracing::warn!("Vault encryption disabled — storing plaintext (set HEX_VAULT_KEY for production)");
                (plaintext.to_string(), 0)
            }
        }
    }

    fn decrypt_value(&self, stored: &str) -> Result<String, String> {
        if !stored.starts_with("AES256GCM:v1:") {
            // Plaintext (key_version 0 or legacy)
            return Ok(stored.to_string());
        }

        let key = self.vault_key.as_ref().ok_or("Cannot decrypt: HEX_VAULT_KEY not set")?;

        use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
        use aes_gcm::aead::Aead;
        use base64::Engine;

        let encoded = &stored["AES256GCM:v1:".len()..];
        let combined = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| format!("Base64 decode failed: {}", e))?;

        if combined.len() < 12 {
            return Err("Invalid ciphertext: too short".to_string());
        }

        let (nonce_bytes, ciphertext) = combined.split_at(12);
        let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| format!("Key error: {}", e))?;
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| "Decryption failed — wrong key or corrupted data".to_string())?;

        String::from_utf8(plaintext).map_err(|e| format!("UTF-8 error: {}", e))
    }

    fn audit(&self, action: &str, agent_id: &str, secret_key: &str) {
        let hub_id = self.hub_id.clone();
        let id = uuid::Uuid::new_v4().to_string();
        let timestamp = chrono::Utc::now().to_rfc3339();
        let action = action.to_string();
        let agent_id = agent_id.to_string();
        let secret_key = secret_key.to_string();

        // Clone self's call_reducer context for the spawn
        let host = self.host.clone();
        let database = self.database.clone();
        let http = self.http.clone();

        tokio::spawn(async move {
            let url = format!("{}/v1/database/{}/call/audit_log", host, database);
            let args = serde_json::json!([id, action, agent_id, secret_key, hub_id, timestamp]);
            let _ = http.post(&url).json(&args).send().await;
        });
    }
}

// ── ISecretGrantPort Implementation ─────────────────────────

#[async_trait]
impl ISecretGrantPort for SpacetimeSecretClient {
    async fn grant(
        &self,
        agent_id: &str,
        secret_key: &str,
        purpose: &str,
        hub_id: &str,
        ttl_secs: u64,
    ) -> Result<SecretGrant, String> {
        let now = chrono::Utc::now();
        let expires = now + chrono::Duration::seconds(ttl_secs as i64);
        let now_str = now.to_rfc3339();
        let expires_str = expires.to_rfc3339();

        // Positional args: agent_id, secret_key, purpose, hub_id, granted_at, expires_at
        self.call_reducer("grant_secret", serde_json::json!([
            agent_id, secret_key, purpose, hub_id, now_str, expires_str
        ]))
        .await?;

        let grant = SecretGrant {
            id: format!("{}:{}", agent_id, secret_key),
            agent_id: agent_id.to_string(),
            secret_key: secret_key.to_string(),
            purpose: purpose.to_string(),
            hub_id: hub_id.to_string(),
            granted_at: now_str,
            expires_at: expires_str,
            claimed: false,
            claimed_at: None,
            claim_hub_id: None,
        };

        self.cache.write().await.insert(grant.id.clone(), grant.clone());
        self.audit("grant", agent_id, secret_key);

        tracing::info!(agent = %agent_id, key = %secret_key, ttl = ttl_secs, "Grant created");
        Ok(grant)
    }

    async fn claim(
        &self,
        agent_id: &str,
        nonce: &str,
        hub_id: &str,
    ) -> Result<Vec<SecretGrant>, String> {
        let cache = self.cache.read().await;
        let agent_grants: Vec<SecretGrant> = cache
            .values()
            .filter(|g| g.agent_id == agent_id && !g.claimed)
            .cloned()
            .collect();
        drop(cache);

        if agent_grants.is_empty() {
            return Err(format!("No unclaimed grants for agent '{}'", agent_id));
        }

        let now_str = chrono::Utc::now().to_rfc3339();
        let mut claimed = Vec::new();

        for grant in &agent_grants {
            // Check expiry
            if grant.expires_at.as_str() <= now_str.as_str() {
                continue;
            }

            // Positional args: agent_id, secret_key, claim_hub_id, claimed_at
            match self.call_reducer("claim_grant", serde_json::json!([
                agent_id, &grant.secret_key, hub_id, &now_str
            ])).await {
                Ok(()) => {
                    let mut updated = grant.clone();
                    updated.claimed = true;
                    updated.claimed_at = Some(now_str.clone());
                    updated.claim_hub_id = Some(hub_id.to_string());

                    self.cache.write().await.insert(updated.id.clone(), updated.clone());
                    self.audit("claim", agent_id, &grant.secret_key);
                    claimed.push(updated);
                }
                Err(e) => {
                    tracing::warn!(key = %grant.secret_key, error = %e, "Claim reducer failed");
                }
            }
        }

        if claimed.is_empty() {
            return Err("Failed to claim any grants".to_string());
        }

        tracing::info!(
            agent = %agent_id,
            count = claimed.len(),
            nonce = %nonce,
            "Grants claimed"
        );

        Ok(claimed)
    }

    async fn revoke(&self, agent_id: &str, secret_key: &str) -> Result<(), String> {
        self.call_reducer("revoke_secret", serde_json::json!([agent_id, secret_key]))
        .await?;

        let id = format!("{}:{}", agent_id, secret_key);
        self.cache.write().await.remove(&id);
        self.audit("revoke", agent_id, secret_key);
        Ok(())
    }

    async fn revoke_all(&self, agent_id: &str) -> Result<usize, String> {
        self.call_reducer("revoke_all_for_agent", serde_json::json!([agent_id]))
        .await?;

        let mut cache = self.cache.write().await;
        let before = cache.len();
        cache.retain(|_, g| g.agent_id != agent_id);
        let removed = before - cache.len();

        self.audit("revoke_all", agent_id, "*");
        Ok(removed)
    }

    async fn list_grants(&self) -> Result<Vec<SecretGrant>, String> {
        Ok(self.cache.read().await.values().cloned().collect())
    }

    async fn list_grants_for_agent(&self, agent_id: &str) -> Result<Vec<SecretGrant>, String> {
        Ok(self.cache.read().await
            .values()
            .filter(|g| g.agent_id == agent_id)
            .cloned()
            .collect())
    }

    async fn prune_expired(&self) -> Result<usize, String> {
        let now = chrono::Utc::now().to_rfc3339();
        self.call_reducer("prune_expired", serde_json::json!([now])).await?;

        let mut cache = self.cache.write().await;
        let before = cache.len();
        cache.retain(|_, g| g.expires_at.as_str() > now.as_str());
        let pruned = before - cache.len();

        if pruned > 0 {
            tracing::info!(pruned, "Pruned expired grants");
        }
        Ok(pruned)
    }

    async fn vault_store(&self, key: &str, value: &str) -> Result<(), String> {
        let (encrypted, key_version) = self.encrypt_value(value);
        let now = chrono::Utc::now().to_rfc3339();

        // Positional args: key, encrypted_value, key_version, stored_at, stored_by_hub
        self.call_reducer("store_secret", serde_json::json!([
            key, encrypted, key_version, now, self.hub_id
        ]))
        .await?;

        tracing::info!(key = %key, encrypted = key_version > 0, "Vault entry stored");
        Ok(())
    }

    async fn vault_get(&self, key: &str) -> Result<Option<String>, String> {
        // Query SpacetimeDB SQL endpoint for the secret value
        let url = format!(
            "{}/v1/database/{}/sql",
            self.host, self.database
        );
        let query = format!("SELECT * FROM secret_vault WHERE key = '{}'", key.replace('\'', "''"));

        let response = self.http
            .post(&url)
            .body(query)
            .header("Content-Type", "text/plain")
            .send()
            .await
            .map_err(|e| format!("SpacetimeDB SQL query failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("SQL query failed ({}): {}", status, body));
        }

        let body = response.text().await.unwrap_or_default();

        // Parse the response — SpacetimeDB returns JSON rows
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body) {
            // Look for encrypted_value in the first row
            let rows = parsed.as_array()
                .and_then(|arr| arr.first())
                .and_then(|table| table.get("rows"))
                .and_then(|r| r.as_array());

            if let Some(rows) = rows {
                if let Some(row) = rows.first() {
                    // Row is an array: [key, encrypted_value, key_version, stored_at, stored_by_hub]
                    if let Some(encrypted) = row.as_array().and_then(|r| r.get(1)).and_then(|v| v.as_str()) {
                        return self.decrypt_value(encrypted).map(Some);
                    }
                }
            }
        }

        Ok(None)
    }

    async fn vault_delete(&self, key: &str) -> Result<(), String> {
        self.call_reducer("delete_secret", serde_json::json!([key])).await
    }

    async fn vault_list(&self) -> Result<HashMap<String, String>, String> {
        let url = format!("{}/v1/database/{}/sql", self.host, self.database);
        let query = "SELECT * FROM secret_vault";

        let response = self.http
            .post(&url)
            .body(query)
            .header("Content-Type", "text/plain")
            .send()
            .await
            .map_err(|e| format!("SpacetimeDB SQL query failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("SQL query failed ({}): {}", status, body));
        }

        let body = response.text().await.unwrap_or_default();
        let mut secrets = HashMap::new();

        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body) {
            let rows = parsed.as_array()
                .and_then(|arr| arr.first())
                .and_then(|table| table.get("rows"))
                .and_then(|r| r.as_array());

            if let Some(rows) = rows {
                for row in rows {
                    if let Some(cols) = row.as_array() {
                        // [key, encrypted_value, key_version, stored_at, stored_by_hub]
                        let key = cols.first().and_then(|v| v.as_str());
                        let encrypted = cols.get(1).and_then(|v| v.as_str());
                        if let (Some(key), Some(encrypted)) = (key, encrypted) {
                            match self.decrypt_value(encrypted) {
                                Ok(plaintext) => { secrets.insert(key.to_string(), plaintext); }
                                Err(e) => { tracing::warn!(key = %key, error = %e, "Failed to decrypt vault entry"); }
                            }
                        }
                    }
                }
            }
        }

        Ok(secrets)
    }

    async fn health(&self) -> SecretBackendHealth {
        SecretBackendHealth {
            connected: self.is_connected().await,
            backend: "spacetimedb".to_string(),
            host: self.host.clone(),
            circuit_state: self.circuit.state_name().to_string(),
            last_error: self.last_error.read().await.clone(),
        }
    }

    async fn is_healthy(&self) -> bool {
        self.is_connected().await && self.circuit.state() != CircuitState::Open
    }
}

// ── Periodic Pruning Task ────────────────────────────────────

pub fn spawn_prune_task(
    client: Arc<SpacetimeSecretClient>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;
            if !client.is_connected().await { continue; }

            match client.prune_expired().await {
                Ok(n) if n > 0 => tracing::info!(pruned = n, "Periodic grant pruning"),
                Err(e) => tracing::warn!(error = %e, "Periodic grant pruning failed"),
                _ => {}
            }
        }
    })
}

/// Spawn a reconnection task with exponential backoff.
pub fn spawn_reconnect_task(
    client: Arc<SpacetimeSecretClient>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut delay = Duration::from_secs(1);
        let max_delay = Duration::from_secs(30);

        loop {
            tokio::time::sleep(delay).await;

            if client.is_connected().await {
                delay = Duration::from_secs(1); // reset on healthy
                tokio::time::sleep(Duration::from_secs(15)).await;
                continue;
            }

            tracing::info!(delay_secs = delay.as_secs(), "Attempting SpacetimeDB reconnection");
            if client.connect().await {
                delay = Duration::from_secs(1);
                tracing::info!("SpacetimeDB reconnected");
            } else {
                delay = (delay * 2).min(max_delay);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circuit_starts_closed() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(10));
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn circuit_opens_after_threshold() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(10));
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
    }

    #[test]
    fn circuit_resets_on_success() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(10));
        cb.record_failure();
        cb.record_failure();
        cb.record_success();
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let client = SpacetimeSecretClient {
            cache: Arc::new(RwLock::new(HashMap::new())),
            host: "http://localhost:3000".into(),
            database: "hex".into(),
            hub_id: "test-hub".into(),
            connected: Arc::new(RwLock::new(false)),
            circuit: CircuitBreaker::new(3, Duration::from_secs(10)),
            last_error: Arc::new(RwLock::new(None)),
            http: reqwest::Client::new(),
            vault_key: Some(b"01234567890123456789012345678901".to_vec()),
        };

        let (encrypted, version) = client.encrypt_value("super-secret-key");
        assert_eq!(version, 1);
        assert!(encrypted.starts_with("AES256GCM:v1:"));

        let decrypted = client.decrypt_value(&encrypted).unwrap();
        assert_eq!(decrypted, "super-secret-key");
    }

    #[test]
    fn decrypt_plaintext_passthrough() {
        let client = SpacetimeSecretClient {
            cache: Arc::new(RwLock::new(HashMap::new())),
            host: "http://localhost:3000".into(),
            database: "hex".into(),
            hub_id: "test-hub".into(),
            connected: Arc::new(RwLock::new(false)),
            circuit: CircuitBreaker::new(3, Duration::from_secs(10)),
            last_error: Arc::new(RwLock::new(None)),
            http: reqwest::Client::new(),
            vault_key: None,
        };

        let result = client.decrypt_value("plain-text-value").unwrap();
        assert_eq!(result, "plain-text-value");
    }

    #[test]
    fn no_key_stores_plaintext() {
        let client = SpacetimeSecretClient {
            cache: Arc::new(RwLock::new(HashMap::new())),
            host: "http://localhost:3000".into(),
            database: "hex".into(),
            hub_id: "test-hub".into(),
            connected: Arc::new(RwLock::new(false)),
            circuit: CircuitBreaker::new(3, Duration::from_secs(10)),
            last_error: Arc::new(RwLock::new(None)),
            http: reqwest::Client::new(),
            vault_key: None,
        };

        let (stored, version) = client.encrypt_value("my-secret");
        assert_eq!(version, 0);
        assert_eq!(stored, "my-secret");
    }
}
