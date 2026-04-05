use russh::*;
use russh::keys::key::PrivateKeyWithHashAlg;
use std::sync::Arc;

/// SSH adapter for connecting to remote compute nodes.
///
/// Uses the russh crate for async SSH — key-based auth only (no passwords).
/// Supports command execution via SSH channels and file transfer via stdin piping.
///
/// Security: All remote commands are executed via russh's SSH channel protocol,
/// not via local shell. Key-based authentication only — no password auth.
pub struct SshAdapter;

/// Configuration for connecting to a remote node.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub key_path: String,
}

/// Result of executing a remote command.
#[derive(Debug, Clone)]
pub struct RemoteCommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: u32,
}

/// SSH client handler that accepts host keys.
struct ClientHandler;

impl russh::client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        // TODO: Verify against known_hosts in production
        Ok(true)
    }
}

impl SshAdapter {
    /// Connect to a remote node and run a command over SSH.
    pub async fn run_command(
        config: &SshConfig,
        command: &str,
    ) -> Result<RemoteCommandResult, SshError> {
        let key_pair = russh::keys::load_secret_key(&config.key_path, None)
            .map_err(|e| SshError::KeyError(format!(
                "Failed to load key {}: {}", config.key_path, e
            )))?;

        let ssh_config = russh::client::Config::default();
        let mut session = russh::client::connect(
            Arc::new(ssh_config),
            (config.host.as_str(), config.port),
            ClientHandler,
        )
        .await
        .map_err(|e| SshError::ConnectionFailed(format!(
            "{}:{} — {}", config.host, config.port, e
        )))?;

        let auth_result = session
            .authenticate_publickey(
                &config.username,
                PrivateKeyWithHashAlg::new(Arc::new(key_pair), None),
            )
            .await
            .map_err(|e| SshError::AuthFailed(e.to_string()))?;

        if !matches!(auth_result, russh::client::AuthResult::Success) {
            return Err(SshError::AuthFailed("Key rejected by server".into()));
        }

        // Open SSH channel and execute via the SSH protocol (not local shell)
        let mut channel = session
            .channel_open_session()
            .await
            .map_err(|e| SshError::CommandFailed(e.to_string()))?;

        // russh channel.exec sends the command over the SSH wire protocol
        channel
            .exec(true, command)
            .await
            .map_err(|e| SshError::CommandFailed(e.to_string()))?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_code = 0u32;

        loop {
            match channel.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    stdout.extend_from_slice(&data);
                }
                Some(ChannelMsg::ExtendedData { data, ext }) => {
                    if ext == 1 {
                        stderr.extend_from_slice(&data);
                    }
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    exit_code = exit_status;
                }
                Some(ChannelMsg::Eof) | None => break,
                _ => {}
            }
        }

        session
            .disconnect(Disconnect::ByApplication, "done", "en")
            .await
            .ok();

        Ok(RemoteCommandResult {
            stdout: String::from_utf8_lossy(&stdout).to_string(),
            stderr: String::from_utf8_lossy(&stderr).to_string(),
            exit_code,
        })
    }

    /// Upload a local file to a remote path via SSH stdin piping.
    pub async fn upload_file(
        config: &SshConfig,
        local_path: &str,
        remote_path: &str,
    ) -> Result<(), SshError> {
        let content = tokio::fs::read(local_path)
            .await
            .map_err(|e| SshError::TransferFailed(format!(
                "Cannot read {}: {}", local_path, e
            )))?;

        let key_pair = russh::keys::load_secret_key(&config.key_path, None)
            .map_err(|e| SshError::KeyError(e.to_string()))?;

        let ssh_config = russh::client::Config::default();
        let mut session = russh::client::connect(
            Arc::new(ssh_config),
            (config.host.as_str(), config.port),
            ClientHandler,
        )
        .await
        .map_err(|e| SshError::ConnectionFailed(e.to_string()))?;

        let auth_result = session
            .authenticate_publickey(
                &config.username,
                PrivateKeyWithHashAlg::new(Arc::new(key_pair), None),
            )
            .await
            .map_err(|e| SshError::AuthFailed(e.to_string()))?;

        if !matches!(auth_result, russh::client::AuthResult::Success) {
            return Err(SshError::AuthFailed("Key rejected by server".into()));
        }

        // Create directory and write file via SSH stdin
        let write_cmd = format!(
            "mkdir -p $(dirname '{}') && cat > '{}'",
            remote_path, remote_path
        );

        let mut channel = session
            .channel_open_session()
            .await
            .map_err(|e| SshError::TransferFailed(e.to_string()))?;

        channel
            .exec(true, write_cmd.as_str())
            .await
            .map_err(|e| SshError::TransferFailed(e.to_string()))?;

        // Pipe content through channel stdin
        channel
            .data(&content[..])
            .await
            .map_err(|e| SshError::TransferFailed(e.to_string()))?;

        channel
            .eof()
            .await
            .map_err(|e| SshError::TransferFailed(e.to_string()))?;

        loop {
            match channel.wait().await {
                Some(ChannelMsg::Eof) | None => break,
                _ => {}
            }
        }

        session
            .disconnect(Disconnect::ByApplication, "done", "en")
            .await
            .ok();

        Ok(())
    }

    /// Check if a remote node is reachable via SSH.
    pub async fn health_check(config: &SshConfig) -> Result<bool, SshError> {
        let result = Self::run_command(config, "echo ok").await?;
        Ok(result.exit_code == 0 && result.stdout.trim() == "ok")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("SSH key error: {0}")]
    KeyError(String),
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Authentication failed: {0}")]
    AuthFailed(String),
    #[error("Command execution failed: {0}")]
    CommandFailed(String),
    #[error("File transfer failed: {0}")]
    TransferFailed(String),
}
