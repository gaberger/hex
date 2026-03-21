//! Remote agent binary provisioner (ADR-040 §8).
//! Checks, deploys, and validates hex-agent binary on remote hosts.

use crate::remote::ssh::{SshAdapter, SshConfig, SshError};
use tracing;

/// Result of provisioning check
#[derive(Debug)]
pub enum ProvisionStatus {
    /// Binary exists and build hash matches
    UpToDate,
    /// Binary exists but build hash differs
    Outdated { remote_hash: String, local_hash: String },
    /// No binary found on remote
    Missing,
}

/// Provision hex-agent binary on a remote host.
pub struct RemoteProvisioner;

impl RemoteProvisioner {
    /// Detect the remote host's architecture via `uname -m`.
    pub async fn detect_arch(config: &SshConfig) -> Result<String, SshError> {
        let result = SshAdapter::run_command(config, "uname -m").await?;
        Ok(result.stdout.trim().to_string())
    }

    /// Check if hex-agent exists on the remote and if it's up to date.
    pub async fn check_binary(config: &SshConfig) -> Result<ProvisionStatus, SshError> {
        // Check if binary exists
        let exists = SshAdapter::run_command(config, "test -f ~/.hex/bin/hex-agent && echo yes || echo no").await?;
        if exists.stdout.trim() != "yes" {
            return Ok(ProvisionStatus::Missing);
        }

        // Check build hash
        let remote = SshAdapter::run_command(config, "~/.hex/bin/hex-agent --build-hash 2>/dev/null || echo unknown").await?;
        let remote_hash = remote.stdout.trim().to_string();
        let local_hash = crate::build_hash().to_string();

        if remote_hash == local_hash {
            Ok(ProvisionStatus::UpToDate)
        } else {
            Ok(ProvisionStatus::Outdated { remote_hash, local_hash })
        }
    }

    /// Deploy hex-agent binary to the remote host.
    /// Uses the local release binary and scps it to ~/.hex/bin/hex-agent.
    pub async fn deploy_binary(config: &SshConfig, local_binary_path: &str) -> Result<(), SshError> {
        tracing::info!(host = %config.host, "Deploying hex-agent binary to remote");

        // Create directory
        SshAdapter::run_command(config, "mkdir -p ~/.hex/bin").await?;

        // Upload binary
        SshAdapter::upload_file(config, local_binary_path, ".hex/bin/hex-agent").await?;

        // Make executable
        SshAdapter::run_command(config, "chmod +x ~/.hex/bin/hex-agent").await?;

        // Verify
        let verify = SshAdapter::run_command(config, "~/.hex/bin/hex-agent --build-hash").await?;
        tracing::info!(
            host = %config.host,
            hash = %verify.stdout.trim(),
            "Binary deployed and verified"
        );

        Ok(())
    }

    /// Build hex-agent on the remote host using cargo (fallback when no pre-built binary).
    pub async fn build_on_remote(config: &SshConfig, source_dir: &str) -> Result<(), SshError> {
        tracing::info!(host = %config.host, "Building hex-agent on remote (this may take a few minutes)");

        let cmd = format!(
            "cd {} && cargo build -p hex-agent --release 2>&1 && \
             mkdir -p ~/.hex/bin && \
             cp target/release/hex-agent ~/.hex/bin/hex-agent",
            source_dir
        );

        let result = SshAdapter::run_command(config, &cmd).await?;
        if result.exit_code != 0 {
            return Err(SshError::CommandFailed(format!(
                "Remote build failed: {}", result.stderr
            )));
        }

        tracing::info!(host = %config.host, "Remote build complete");
        Ok(())
    }

    /// Full provision flow: check → deploy or build → verify.
    pub async fn ensure_binary(
        config: &SshConfig,
        local_binary_path: Option<&str>,
        remote_source_dir: Option<&str>,
    ) -> Result<(), SshError> {
        match Self::check_binary(config).await? {
            ProvisionStatus::UpToDate => {
                tracing::info!(host = %config.host, "hex-agent binary is up to date");
                Ok(())
            }
            ProvisionStatus::Outdated { remote_hash, local_hash } => {
                tracing::info!(
                    host = %config.host,
                    remote = %remote_hash,
                    local = %local_hash,
                    "hex-agent binary outdated, updating"
                );
                if let Some(path) = local_binary_path {
                    Self::deploy_binary(config, path).await
                } else if let Some(src) = remote_source_dir {
                    Self::build_on_remote(config, src).await
                } else {
                    Err(SshError::CommandFailed("No binary path or source dir provided for update".into()))
                }
            }
            ProvisionStatus::Missing => {
                tracing::info!(host = %config.host, "hex-agent binary not found, provisioning");
                if let Some(path) = local_binary_path {
                    Self::deploy_binary(config, path).await
                } else if let Some(src) = remote_source_dir {
                    Self::build_on_remote(config, src).await
                } else {
                    Err(SshError::CommandFailed("No binary path or source dir provided for initial deploy".into()))
                }
            }
        }
    }
}
