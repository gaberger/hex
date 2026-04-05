//! SSH tunnel adapter — persistent tunnels with keepalive and reconnect (ADR-040).
//!
//! Unlike `remote::ssh::SshAdapter` which connects/disconnects per command,
//! this adapter maintains long-lived SSH sessions with TCP port forwarding
//! for remote agent communication.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use async_trait::async_trait;
use russh::keys::key::PrivateKeyWithHashAlg;
use tokio::sync::{Mutex, RwLock};

use crate::ports::ssh_tunnel::ISshTunnelPort;
use crate::remote::transport::*;

// ── Internal Types ──────────────────────────────────

/// A session handle shared between the adapter and the keepalive task.
type SharedSession = Arc<Mutex<russh::client::Handle<SshTunnelHandler>>>;

struct ActiveTunnel {
    handle: TunnelHandle,
    config: SshTunnelConfig,
    session: SharedSession,
    keepalive_task: tokio::task::JoinHandle<()>,
    reconnect_count: u32,
}

/// Minimal russh client handler — accepts all host keys.
struct SshTunnelHandler;

impl russh::client::Handler for SshTunnelHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        // TODO: Verify against known_hosts in production
        Ok(true)
    }
}

// ── Adapter ─────────────────────────────────────────

pub struct SshTunnelAdapter {
    tunnels: Arc<RwLock<HashMap<String, ActiveTunnel>>>,
}

impl Default for SshTunnelAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl SshTunnelAdapter {
    pub fn new() -> Self {
        Self {
            tunnels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Establish an authenticated SSH session.
    async fn establish_session(
        config: &SshTunnelConfig,
    ) -> Result<russh::client::Handle<SshTunnelHandler>, TransportError> {
        let ssh_config = russh::client::Config::default();

        let mut session = russh::client::connect(
            Arc::new(ssh_config),
            (config.host.as_str(), config.port),
            SshTunnelHandler,
        )
        .await
        .map_err(|e| {
            TransportError::Tunnel(format!(
                "SSH connect to {}:{} failed: {}",
                config.host, config.port, e
            ))
        })?;

        // Authenticate based on auth method
        match &config.auth {
            SshAuth::Key { path, passphrase } => {
                let key_pair =
                    russh::keys::load_secret_key(path, passphrase.as_deref()).map_err(|e| {
                        TransportError::Auth(format!("Failed to load key {}: {}", path, e))
                    })?;

                let auth_result = session
                    .authenticate_publickey(
                        &config.user,
                        PrivateKeyWithHashAlg::new(Arc::new(key_pair), None),
                    )
                    .await
                    .map_err(|e| TransportError::Auth(format!("Public key auth failed: {}", e)))?;

                if !matches!(auth_result, russh::client::AuthResult::Success) {
                    return Err(TransportError::Auth("Key rejected by server".into()));
                }
            }
            SshAuth::Agent => {
                let mut agent = russh::keys::agent::client::AgentClient::connect_env()
                    .await
                    .map_err(|e| {
                        TransportError::Auth(format!("SSH agent connection failed: {}", e))
                    })?;

                let identities = agent.request_identities().await.map_err(|e| {
                    TransportError::Auth(format!("Failed to list agent identities: {}", e))
                })?;

                if identities.is_empty() {
                    return Err(TransportError::Auth(
                        "SSH agent has no identities loaded".into(),
                    ));
                }

                let mut authenticated = false;
                for identity in &identities {
                    let public_key = match identity {
                        russh::keys::agent::AgentIdentity::PublicKey { key, .. } => key.clone(),
                        russh::keys::agent::AgentIdentity::Certificate { certificate, .. } => {
                            russh::keys::PublicKey::new(certificate.public_key().clone(), "")
                        }
                    };

                    let auth_result = session
                        .authenticate_publickey_with(
                            &config.user,
                            public_key,
                            None,
                            &mut agent,
                        )
                        .await;

                    match auth_result {
                        Ok(russh::client::AuthResult::Success) => {
                            authenticated = true;
                            break;
                        }
                        _ => continue,
                    }
                }

                if !authenticated {
                    return Err(TransportError::Auth(
                        "No agent identity accepted by server".into(),
                    ));
                }
            }
        }

        tracing::info!(host = %config.host, port = config.port, user = %config.user, "SSH session established");
        Ok(session)
    }

    /// Spawn a background task that sends keepalive pings by opening
    /// a session channel. If the channel open fails, the session is dead.
    fn spawn_keepalive(
        session: SharedSession,
        interval_secs: u16,
        tunnel_id: String,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let interval = std::time::Duration::from_secs(interval_secs as u64);
            loop {
                tokio::time::sleep(interval).await;

                let sess = session.lock().await;
                match sess.channel_open_session().await {
                    Ok(_channel) => {
                        tracing::trace!(tunnel_id = %tunnel_id, "keepalive ok");
                        // Channel is dropped immediately — we only needed to probe liveness
                    }
                    Err(e) => {
                        tracing::warn!(
                            tunnel_id = %tunnel_id,
                            error = %e,
                            "keepalive failed — session may be dead"
                        );
                        break;
                    }
                }
            }
        })
    }

    /// Derive a deterministic local port from the tunnel ID (range 10000-60000).
    fn local_port_for(tunnel_id: &str) -> u16 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        tunnel_id.hash(&mut hasher);
        let h = hasher.finish();
        10000 + (h % 50000) as u16
    }
}

#[async_trait]
impl ISshTunnelPort for SshTunnelAdapter {
    async fn connect(&self, config: SshTunnelConfig) -> Result<TunnelHandle, TransportError> {
        let tunnel_id = uuid::Uuid::new_v4().to_string();

        let session = Self::establish_session(&config).await?;

        let local_port = if config.local_forward_port == 0 {
            Self::local_port_for(&tunnel_id)
        } else {
            config.local_forward_port
        };

        // Open a direct-tcpip channel to verify the forwarding path is viable.
        // The channel itself is not held long-term; actual data forwarding happens
        // when WebSocket traffic is proxied through the tunnel later.
        let _channel = session
            .channel_open_direct_tcpip(
                "127.0.0.1",
                config.remote_bind_port as u32,
                "127.0.0.1",
                local_port as u32,
            )
            .await
            .map_err(|e| {
                TransportError::Tunnel(format!(
                    "Failed to open direct-tcpip channel (local:{} -> remote:{}): {}",
                    local_port, config.remote_bind_port, e
                ))
            })?;

        let shared_session = Arc::new(Mutex::new(session));

        let keepalive_task = Self::spawn_keepalive(
            Arc::clone(&shared_session),
            config.keepalive_interval_secs,
            tunnel_id.clone(),
        );

        let handle = TunnelHandle {
            id: tunnel_id.clone(),
            host: config.host.clone(),
            port: config.port,
            user: config.user.clone(),
            local_forward_port: local_port,
            remote_bind_port: config.remote_bind_port,
            established_at: chrono::Utc::now().to_rfc3339(),
        };

        let active = ActiveTunnel {
            handle: handle.clone(),
            config,
            session: shared_session,
            keepalive_task,
            reconnect_count: 0,
        };

        self.tunnels.write().await.insert(tunnel_id.clone(), active);

        tracing::info!(
            tunnel_id = %handle.id,
            local_port = handle.local_forward_port,
            remote_port = handle.remote_bind_port,
            host = %handle.host,
            "tunnel established"
        );

        Ok(handle)
    }

    async fn health(&self, tunnel_id: &str) -> Result<TunnelHealth, TransportError> {
        let tunnels = self.tunnels.read().await;
        let tunnel = tunnels.get(tunnel_id).ok_or_else(|| {
            TransportError::Tunnel(format!("No tunnel with id: {}", tunnel_id))
        })?;

        // If the keepalive task has finished, the session is likely dead
        if tunnel.keepalive_task.is_finished() {
            return Ok(TunnelHealth::Disconnected);
        }

        // Try opening a direct-tcpip channel to verify the session is responsive
        let sess = tunnel.session.lock().await;
        match sess
            .channel_open_direct_tcpip(
                "127.0.0.1",
                tunnel.config.remote_bind_port as u32,
                "127.0.0.1",
                tunnel.handle.local_forward_port as u32,
            )
            .await
        {
            Ok(_) => Ok(TunnelHealth::Connected),
            Err(_) => {
                // Session exists but channel open failed — degraded
                Ok(TunnelHealth::Degraded)
            }
        }
    }

    async fn reconnect(&self, tunnel_id: &str) -> Result<TunnelHandle, TransportError> {
        // Extract config and reconnect count from the existing tunnel
        let (config, prev_reconnect_count) = {
            let tunnels = self.tunnels.read().await;
            let tunnel = tunnels.get(tunnel_id).ok_or_else(|| {
                TransportError::Tunnel(format!("No tunnel with id: {}", tunnel_id))
            })?;
            (tunnel.config.clone(), tunnel.reconnect_count)
        };

        // Disconnect the old tunnel (best-effort)
        let _ = self.disconnect(tunnel_id).await;

        let max_attempts = config.reconnect_max_attempts as u32;
        let mut attempt = 0u32;

        loop {
            attempt += 1;

            // Exponential backoff: 1s, 2s, 4s, 8s, ... capped at 60s
            let backoff_secs = std::cmp::min(1u64 << (attempt - 1), 60);
            tracing::info!(
                tunnel_id = %tunnel_id,
                attempt,
                max_attempts,
                backoff_secs,
                "reconnecting tunnel"
            );
            tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;

            match Self::establish_session(&config).await {
                Ok(session) => {
                    let local_port = if config.local_forward_port == 0 {
                        Self::local_port_for(tunnel_id)
                    } else {
                        config.local_forward_port
                    };

                    // Verify the forwarding path
                    if let Err(e) = session
                        .channel_open_direct_tcpip(
                            "127.0.0.1",
                            config.remote_bind_port as u32,
                            "127.0.0.1",
                            local_port as u32,
                        )
                        .await
                    {
                        tracing::warn!(
                            tunnel_id = %tunnel_id,
                            attempt,
                            error = %e,
                            "session established but channel open failed"
                        );
                        if attempt >= max_attempts {
                            return Err(TransportError::Tunnel(format!(
                                "Reconnect failed after {} attempts: {}",
                                max_attempts, e
                            )));
                        }
                        continue;
                    }

                    let shared_session = Arc::new(Mutex::new(session));

                    let keepalive_task = Self::spawn_keepalive(
                        Arc::clone(&shared_session),
                        config.keepalive_interval_secs,
                        tunnel_id.to_string(),
                    );

                    let handle = TunnelHandle {
                        id: tunnel_id.to_string(),
                        host: config.host.clone(),
                        port: config.port,
                        user: config.user.clone(),
                        local_forward_port: local_port,
                        remote_bind_port: config.remote_bind_port,
                        established_at: chrono::Utc::now().to_rfc3339(),
                    };

                    let active = ActiveTunnel {
                        handle: handle.clone(),
                        config,
                        session: shared_session,
                        keepalive_task,
                        reconnect_count: prev_reconnect_count + attempt,
                    };

                    self.tunnels
                        .write()
                        .await
                        .insert(tunnel_id.to_string(), active);

                    tracing::info!(
                        tunnel_id = %tunnel_id,
                        attempts = attempt,
                        "tunnel reconnected"
                    );

                    return Ok(handle);
                }
                Err(e) => {
                    tracing::warn!(
                        tunnel_id = %tunnel_id,
                        attempt,
                        error = %e,
                        "reconnect attempt failed"
                    );
                    if attempt >= max_attempts {
                        return Err(TransportError::Tunnel(format!(
                            "Reconnect failed after {} attempts: {}",
                            max_attempts, e
                        )));
                    }
                }
            }
        }
    }

    async fn disconnect(&self, tunnel_id: &str) -> Result<(), TransportError> {
        let tunnel = self.tunnels.write().await.remove(tunnel_id).ok_or_else(|| {
            TransportError::Tunnel(format!("No tunnel with id: {}", tunnel_id))
        })?;

        // Cancel keepalive task
        tunnel.keepalive_task.abort();

        // Gracefully disconnect SSH session
        let sess = tunnel.session.lock().await;
        sess.disconnect(russh::Disconnect::ByApplication, "tunnel closed", "en")
            .await
            .map_err(|e| {
                TransportError::Tunnel(format!("Error during SSH disconnect: {}", e))
            })?;

        tracing::info!(tunnel_id = %tunnel_id, "tunnel disconnected");
        Ok(())
    }

    async fn list_tunnels(&self) -> Result<Vec<TunnelInfo>, TransportError> {
        let tunnels = self.tunnels.read().await;
        let mut infos = Vec::with_capacity(tunnels.len());

        for (_, tunnel) in tunnels.iter() {
            let health = if tunnel.keepalive_task.is_finished() {
                TunnelHealth::Disconnected
            } else {
                TunnelHealth::Connected
            };

            infos.push(TunnelInfo {
                handle: tunnel.handle.clone(),
                health,
                bytes_sent: 0,     // TODO: track via channel wrapper
                bytes_received: 0, // TODO: track via channel wrapper
                reconnect_count: tunnel.reconnect_count,
            });
        }

        Ok(infos)
    }
}
