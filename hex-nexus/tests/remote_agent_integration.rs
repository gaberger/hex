//! Integration tests for the remote agent transport layer (ADR-040).
//!
//! These tests require a real SSH-accessible host (bazzite.local) with
//! Ollama running. Skip with: cargo test --test remote_agent_integration -- --ignored
//!
//! Prerequisites:
//! - SSH key auth to bazzite.local (ssh-agent or ~/.ssh/id_ed25519)
//! - Ollama running on bazzite.local:11434

use hex_nexus::adapters::ssh_tunnel::SshTunnelAdapter;
use hex_nexus::adapters::remote_registry::RemoteRegistryAdapter;
use hex_nexus::ports::ssh_tunnel::ISshTunnelPort;
use hex_nexus::ports::remote_registry::IRemoteRegistryPort;
use hex_nexus::remote::transport::*;

const BAZZITE_HOST: &str = "bazzite.local";
const BAZZITE_USER: &str = "gary";
const OLLAMA_PORT: u16 = 11434;

fn ssh_config() -> SshTunnelConfig {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/gary".into());
    SshTunnelConfig {
        host: BAZZITE_HOST.into(),
        port: 22,
        user: BAZZITE_USER.into(),
        auth: SshAuth::Key {
            path: format!("{}/.ssh/id_rsa", home),
            passphrase: None,
        },
        remote_bind_port: OLLAMA_PORT,
        local_forward_port: 0, // auto-assign
        keepalive_interval_secs: 15,
        reconnect_max_attempts: 3,
    }
}

// ── SSH Tunnel Tests ─────────────────────────────────

#[tokio::test]
#[ignore] // Requires bazzite.local to be reachable
async fn test_ssh_tunnel_connect_and_health() {
    let adapter = SshTunnelAdapter::new();

    // Connect
    let handle = adapter.connect(ssh_config()).await
        .expect("Failed to establish SSH tunnel to bazzite");

    assert!(!handle.id.is_empty(), "Tunnel should have an ID");
    assert_eq!(handle.host, BAZZITE_HOST);
    println!("Tunnel established: {} → localhost:{}", handle.host, handle.local_forward_port);

    // Health check
    let health = adapter.health(&handle.id).await
        .expect("Health check failed");
    assert_eq!(health, TunnelHealth::Connected, "Tunnel should be connected");

    // List
    let tunnels = adapter.list_tunnels().await.expect("List failed");
    assert!(!tunnels.is_empty(), "Should have at least one tunnel");
    println!("Active tunnels: {}", tunnels.len());

    // Disconnect
    adapter.disconnect(&handle.id).await
        .expect("Disconnect failed");

    // Verify disconnected
    let tunnels_after = adapter.list_tunnels().await.expect("List failed");
    assert!(
        !tunnels_after.iter().any(|t| t.handle.id == handle.id),
        "Tunnel should be removed after disconnect"
    );

    println!("SSH tunnel test passed!");
}

#[tokio::test]
#[ignore]
async fn test_ssh_tunnel_reconnect() {
    let adapter = SshTunnelAdapter::new();

    let handle = adapter.connect(ssh_config()).await
        .expect("Initial connect failed");

    // Reconnect (should work even if tunnel is healthy)
    let new_handle = adapter.reconnect(&handle.id).await
        .expect("Reconnect failed");

    assert_eq!(new_handle.host, BAZZITE_HOST);
    println!("Reconnect successful, new tunnel ID: {}", new_handle.id);

    adapter.disconnect(&new_handle.id).await.expect("Cleanup failed");
}

// ── Remote Registry Tests ────────────────────────────

#[tokio::test]
#[ignore]
async fn test_registry_agent_lifecycle() {
    let registry = RemoteRegistryAdapter::new(None);

    // Register a bazzite agent
    let agent = RemoteAgent {
        agent_id: "bazzite-test-1".into(),
        name: "bazzite-gpu".into(),
        host: BAZZITE_HOST.into(),
        project_dir: "/home/gary/projects/hex-intf".into(),
        status: RemoteAgentStatus::Online,
        capabilities: AgentCapabilities {
            models: vec!["qwen3.5:27b".into(), "qwen3.5:9b".into()],
            tools: vec!["fs".into(), "shell".into()],
            max_concurrent_tasks: 2,
            gpu_vram_mb: None,
        },
        last_heartbeat: chrono::Utc::now().to_rfc3339(),
        connected_at: chrono::Utc::now().to_rfc3339(),
        tunnel_id: Some("test-tunnel-1".into()),
    };

    registry.register_agent(agent.clone()).await.expect("Register failed");

    // List
    let agents = registry.list_agents(None).await.expect("List failed");
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name, "bazzite-gpu");

    // Register inference server
    let server = InferenceServer {
        server_id: "ollama-bazzite-1".into(),
        agent_id: "bazzite-test-1".into(),
        provider: InferenceProvider::Ollama,
        base_url: format!("http://{}:{}", BAZZITE_HOST, OLLAMA_PORT),
        models: vec!["qwen3.5:27b".into(), "qwen3.5:9b".into()],
        gpu_vram_mb: 0,
        status: InferenceServerStatus::Available,
        current_load: 0.0,
    };

    registry.register_inference_server(server).await.expect("Register server failed");

    // Query by model
    let servers = registry.list_inference_servers(Some("qwen3.5:27b")).await.expect("List failed");
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].provider, InferenceProvider::Ollama);

    // Update status
    registry.update_agent_status("bazzite-test-1", RemoteAgentStatus::Busy).await.expect("Update failed");
    let updated = registry.get_agent("bazzite-test-1").await.expect("Get failed").unwrap();
    assert_eq!(updated.status, RemoteAgentStatus::Busy);

    // Heartbeat
    registry.heartbeat("bazzite-test-1").await.expect("Heartbeat failed");

    // Deregister
    registry.deregister_agent("bazzite-test-1").await.expect("Deregister failed");
    let after = registry.list_agents(None).await.expect("List failed");
    assert!(after.is_empty());

    // Servers should be cleaned up too
    let servers_after = registry.list_inference_servers(None).await.expect("List failed");
    assert!(servers_after.is_empty());

    println!("Registry lifecycle test passed!");
}

// ── Full Stack Smoke Test ────────────────────────────

#[tokio::test]
#[ignore]
async fn test_ssh_tunnel_to_ollama() {
    // This test validates the core value prop: SSH tunnel from Mac to bazzite's Ollama
    let adapter = SshTunnelAdapter::new();

    let mut config = ssh_config();
    config.local_forward_port = 19434; // fixed port for this test

    let handle = adapter.connect(config).await
        .expect("SSH tunnel to bazzite failed");

    println!(
        "Tunnel ready: bazzite.local:{} → localhost:{}",
        OLLAMA_PORT, handle.local_forward_port
    );

    // The tunnel is now established. In a full test we'd connect to
    // localhost:{local_forward_port} and hit the Ollama API.
    // For now, verify the tunnel is healthy.
    let health = adapter.health(&handle.id).await.expect("Health check failed");
    assert_eq!(health, TunnelHealth::Connected);

    adapter.disconnect(&handle.id).await.expect("Cleanup failed");
    println!("SSH tunnel to Ollama smoke test passed!");
}
