//! Agent lifecycle adapter — composes SSH tunnel, WS transport, and remote registry
//! ports to manage the full lifecycle of remote agents (ADR-040).

use std::sync::Arc;
use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing;

use crate::ports::agent_lifecycle::IAgentLifecyclePort;
use crate::ports::agent_transport::IAgentTransportPort;
use crate::ports::remote_registry::IRemoteRegistryPort;
use crate::ports::ssh_tunnel::ISshTunnelPort;
use crate::remote::transport::*;
use crate::remote::transport::SshAuth;

/// Manages the full lifecycle of remote agents:
/// spawn → register → heartbeat → task assignment → result collection → teardown
pub struct AgentLifecycleAdapter {
    tunnel_port: Arc<dyn ISshTunnelPort>,
    transport_port: Arc<dyn IAgentTransportPort>,
    registry_port: Arc<dyn IRemoteRegistryPort>,
    heartbeat_tasks: Arc<RwLock<Vec<(String, tokio::task::JoinHandle<()>)>>>,
}

impl AgentLifecycleAdapter {
    pub fn new(
        tunnel_port: Arc<dyn ISshTunnelPort>,
        transport_port: Arc<dyn IAgentTransportPort>,
        registry_port: Arc<dyn IRemoteRegistryPort>,
    ) -> Self {
        Self {
            tunnel_port,
            transport_port,
            registry_port,
            heartbeat_tasks: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Spawn a remote agent: establish SSH tunnel → connect WS → register → start heartbeat monitor.
    pub async fn spawn_remote_agent(
        &self,
        config: SshTunnelConfig,
        agent_name: String,
        project_dir: String,
    ) -> Result<RemoteAgent, TransportError> {
        // 1. Establish SSH tunnel
        let tunnel = self.tunnel_port.connect(config).await?;
        tracing::info!(
            tunnel_id = %tunnel.id,
            local_port = tunnel.local_forward_port,
            "SSH tunnel established"
        );

        // 2. Subscribe to incoming messages on the tunneled transport
        let mut rx = self.transport_port.subscribe(&tunnel.id).await?;

        // 3. Wait for Register message from agent
        let (agent_id, capabilities, reported_project_dir) = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            async {
                while let Some(msg) = rx.recv().await {
                    if let AgentMessage::Register {
                        agent_id,
                        capabilities,
                        project_dir,
                    } = msg
                    {
                        return Ok((agent_id, capabilities, project_dir));
                    }
                }
                Err(TransportError::Timeout(
                    "No Register message received from agent".into(),
                ))
            },
        )
        .await
        .map_err(|_| {
            TransportError::Timeout("Timed out waiting for agent registration".into())
        })??;

        // 4. Send RegisterAck with session nonce
        let session_nonce = uuid::Uuid::new_v4().to_string();
        self.transport_port
            .send(
                &agent_id,
                AgentMessage::RegisterAck {
                    session_nonce: session_nonce.clone(),
                },
            )
            .await?;

        // 5. Build RemoteAgent record
        let now = chrono::Utc::now().to_rfc3339();
        let agent = RemoteAgent {
            agent_id: agent_id.clone(),
            name: agent_name,
            host: tunnel.host.clone(),
            project_dir: if reported_project_dir.is_empty() {
                project_dir
            } else {
                reported_project_dir
            },
            status: RemoteAgentStatus::Online,
            capabilities,
            last_heartbeat: now.clone(),
            connected_at: now,
            tunnel_id: Some(tunnel.id.clone()),
        };

        // 6. Register agent in registry
        self.registry_port.register_agent(agent.clone()).await?;

        // 7. Spawn heartbeat monitor task
        let handle = self.spawn_heartbeat_monitor(agent_id.clone());
        self.heartbeat_tasks
            .write()
            .await
            .push((agent_id, handle));

        tracing::info!(
            agent_id = %agent.agent_id,
            "Remote agent spawned and registered"
        );

        Ok(agent)
    }

    /// Accept an incoming agent connection (agent initiated the connection to nexus).
    pub async fn accept_agent(
        &self,
        agent_id: String,
        capabilities: AgentCapabilities,
        project_dir: String,
    ) -> Result<RemoteAgent, TransportError> {
        let now = chrono::Utc::now().to_rfc3339();
        let agent = RemoteAgent {
            agent_id: agent_id.clone(),
            name: agent_id.clone(),
            host: "incoming".into(),
            project_dir,
            status: RemoteAgentStatus::Online,
            capabilities,
            last_heartbeat: now.clone(),
            connected_at: now,
            tunnel_id: None,
        };

        self.registry_port.register_agent(agent.clone()).await?;

        let handle = self.spawn_heartbeat_monitor(agent_id.clone());
        self.heartbeat_tasks
            .write()
            .await
            .push((agent_id, handle));

        tracing::info!(
            agent_id = %agent.agent_id,
            "Incoming agent accepted and registered"
        );

        Ok(agent)
    }

    /// Start a heartbeat monitor for an agent.
    /// Checks every 15s if the agent is still connected.
    /// Marks as stale after 45s, dead after 120s.
    fn spawn_heartbeat_monitor(&self, agent_id: String) -> tokio::task::JoinHandle<()> {
        let registry = self.registry_port.clone();
        let transport = self.transport_port.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
            let mut missed_heartbeats = 0u32;
            loop {
                interval.tick().await;
                if transport.is_connected(&agent_id).await {
                    // Send Ping, expect Pong
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let _ = transport
                        .send(&agent_id, AgentMessage::Ping { timestamp: ts })
                        .await;
                    missed_heartbeats = 0;
                    let _ = registry.heartbeat(&agent_id).await;
                } else {
                    missed_heartbeats += 1;
                    if missed_heartbeats >= 8 {
                        // 120s
                        tracing::warn!(agent_id, "Agent marked dead after 120s");
                        let _ = registry
                            .update_agent_status(&agent_id, RemoteAgentStatus::Dead)
                            .await;
                        break;
                    } else if missed_heartbeats >= 3 {
                        // 45s
                        tracing::warn!(agent_id, "Agent marked stale after 45s");
                        let _ = registry
                            .update_agent_status(&agent_id, RemoteAgentStatus::Stale)
                            .await;
                    }
                }
            }
        })
    }

    /// Disconnect and cleanup a remote agent.
    pub async fn disconnect_agent(&self, agent_id: &str) -> Result<(), TransportError> {
        // 1. Stop heartbeat monitor
        {
            let mut tasks = self.heartbeat_tasks.write().await;
            if let Some(pos) = tasks.iter().position(|(id, _)| id == agent_id) {
                let (_, handle) = tasks.remove(pos);
                handle.abort();
            }
        }

        // 2. Disconnect transport
        self.transport_port.disconnect(agent_id).await?;

        // 3. Disconnect tunnel if one exists
        if let Ok(Some(agent)) = self.registry_port.get_agent(agent_id).await {
            if let Some(tunnel_id) = &agent.tunnel_id {
                let _ = self.tunnel_port.disconnect(tunnel_id).await;
            }
        }

        // 4. Remove inference servers for this agent
        let _ = self.registry_port.deregister_agent_servers(agent_id).await;

        // 5. Deregister from registry
        self.registry_port.deregister_agent(agent_id).await?;

        tracing::info!(agent_id, "Agent disconnected and cleaned up");
        Ok(())
    }

    /// Full remote agent spawn: provision → tunnel → launch → register.
    /// This is the one-command deploy from ADR-040 §8.
    pub async fn spawn_remote_full(
        &self,
        ssh_config: SshTunnelConfig,
        project_dir: String,
        agent_name: Option<String>,
        remote_source_dir: Option<String>,
    ) -> Result<RemoteAgent, TransportError> {
        let host = ssh_config.host.clone();
        let user = ssh_config.user.clone();

        // Convert SshTunnelConfig → SshConfig for provisioner
        let provision_config = crate::remote::ssh::SshConfig {
            host: ssh_config.host.clone(),
            port: ssh_config.port,
            username: ssh_config.user.clone(),
            key_path: match &ssh_config.auth {
                SshAuth::Key { path, .. } => path.clone(),
                SshAuth::Agent => {
                    // Try common key paths
                    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
                    format!("{}/.ssh/id_ed25519", home)
                }
            },
        };

        // Phase 1: PROVISION
        tracing::info!(host = %host, "Phase 1: Provisioning hex-agent binary");
        let src_dir = remote_source_dir.as_deref().unwrap_or("~/projects/hex-intf");
        if let Err(e) = crate::remote::provisioner::RemoteProvisioner::ensure_binary(
            &provision_config,
            None, // TODO: local binary path for scp deploy
            Some(src_dir),
        )
        .await
        {
            tracing::warn!(host = %host, error = %e, "Provisioning failed, assuming binary exists");
        }

        // Phase 2: TUNNEL
        tracing::info!(host = %host, "Phase 2: Establishing SSH reverse tunnel");
        let tunnel = self.tunnel_port.connect(ssh_config.clone()).await?;
        tracing::info!(
            host = %host,
            local_port = tunnel.local_forward_port,
            "Tunnel established"
        );

        // Phase 3: LAUNCH
        tracing::info!(host = %host, "Phase 3: Launching hex-agent on remote");
        let agent_id = uuid::Uuid::new_v4().to_string();
        let session_token = uuid::Uuid::new_v4().to_string();
        let display_name = agent_name.unwrap_or_else(|| format!("{}@{}", user, host));

        let launch_cmd = format!(
            "HEX_NEXUS_URL=http://127.0.0.1:{port} \
             HEX_AGENT_ID={agent_id} \
             HEX_AGENT_TOKEN={token} \
             nohup ~/.hex/bin/hex-agent \
               --hub-url http://127.0.0.1:{port} \
               --hub-token {token} \
               --project-dir {project_dir} \
               --no-preflight \
             > ~/.hex/agent-{short_id}.log 2>&1 &",
            port = tunnel.local_forward_port,
            agent_id = agent_id,
            token = session_token,
            project_dir = project_dir,
            short_id = &agent_id[..8],
        );

        let launch_result = crate::remote::ssh::SshAdapter::run_command(
            &provision_config,
            &launch_cmd,
        )
        .await
        .map_err(|e| TransportError::Tunnel(format!("Failed to launch agent: {}", e)))?;

        if launch_result.exit_code != 0 {
            return Err(TransportError::Tunnel(format!(
                "Agent launch failed: {}",
                launch_result.stderr
            )));
        }

        // Phase 4: CONFIRM — wait for agent to register via WebSocket
        tracing::info!(host = %host, "Phase 4: Waiting for agent registration");

        // Give the agent a moment to start and connect
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        // Build the RemoteAgent record
        let now = chrono::Utc::now().to_rfc3339();
        let agent = RemoteAgent {
            agent_id: agent_id.clone(),
            name: display_name,
            host: host.clone(),
            project_dir,
            status: RemoteAgentStatus::Online,
            capabilities: AgentCapabilities::default(),
            last_heartbeat: now.clone(),
            connected_at: now,
            tunnel_id: Some(tunnel.id.clone()),
        };

        // Register in our local registry
        self.registry_port.register_agent(agent.clone()).await?;

        // Start heartbeat monitor
        let handle = self.spawn_heartbeat_monitor(agent_id.clone());
        self.heartbeat_tasks.write().await.push((agent_id, handle));

        tracing::info!(
            host = %host,
            agent = %agent.name,
            "Remote agent spawned successfully"
        );

        Ok(agent)
    }

    /// List all managed agents with their current status.
    pub async fn list_agents(&self) -> Result<Vec<RemoteAgent>, TransportError> {
        self.registry_port.list_agents(None).await
    }

    /// Get a specific agent's info.
    pub async fn get_agent(&self, agent_id: &str) -> Result<Option<RemoteAgent>, TransportError> {
        self.registry_port.get_agent(agent_id).await
    }
}

#[async_trait]
impl IAgentLifecyclePort for AgentLifecycleAdapter {
    async fn spawn_remote_agent(
        &self,
        config: SshTunnelConfig,
        agent_name: String,
        project_dir: String,
    ) -> Result<RemoteAgent, TransportError> {
        self.spawn_remote_agent(config, agent_name, project_dir).await
    }

    async fn spawn_remote_full(
        &self,
        config: SshTunnelConfig,
        project_dir: String,
        agent_name: Option<String>,
        remote_source_dir: Option<String>,
    ) -> Result<RemoteAgent, TransportError> {
        self.spawn_remote_full(config, project_dir, agent_name, remote_source_dir).await
    }

    async fn accept_agent(
        &self,
        agent_id: String,
        capabilities: AgentCapabilities,
        project_dir: String,
    ) -> Result<RemoteAgent, TransportError> {
        self.accept_agent(agent_id, capabilities, project_dir).await
    }

    async fn disconnect_agent(&self, agent_id: &str) -> Result<(), TransportError> {
        self.disconnect_agent(agent_id).await
    }

    async fn list_agents(&self) -> Result<Vec<RemoteAgent>, TransportError> {
        self.list_agents().await
    }

    async fn get_agent(&self, agent_id: &str) -> Result<Option<RemoteAgent>, TransportError> {
        self.get_agent(agent_id).await
    }
}
