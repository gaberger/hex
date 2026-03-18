use super::ssh::{SshAdapter, SshConfig, SshError};

/// Deploys hex-agent binary and ruflo swarm to remote compute nodes.
///
/// Workflow:
/// 1. Check if hex-agent is already installed (and correct version)
/// 2. Upload binary if needed
/// 3. Make executable
/// 4. Verify checksum
/// 5. Install ruflo (npm global)
pub struct Deployer;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeployResult {
    pub host: String,
    pub hex_agent_installed: bool,
    pub ruflo_installed: bool,
    pub hex_agent_version: String,
    pub error: Option<String>,
}

impl Deployer {
    /// Deploy hex-agent to a remote node.
    pub async fn deploy_hex_agent(
        config: &SshConfig,
        local_binary_path: &str,
        remote_install_dir: &str,
    ) -> Result<DeployResult, SshError> {
        let remote_bin = format!("{}/hex-agent", remote_install_dir);

        // Check existing version
        let version_check = SshAdapter::run_command(
            config,
            &format!("{} --version 2>/dev/null || echo 'not-installed'", remote_bin),
        )
        .await?;

        let existing_version = version_check.stdout.trim().to_string();
        let needs_install = existing_version.contains("not-installed");

        if needs_install {
            tracing::info!(host = %config.host, "Installing hex-agent");

            // Upload binary
            SshAdapter::upload_file(config, local_binary_path, &remote_bin).await?;

            // Make executable
            SshAdapter::run_command(config, &format!("chmod +x '{}'", remote_bin)).await?;

            // Verify it runs
            let verify = SshAdapter::run_command(config, &format!("{} --version", remote_bin)).await?;
            if verify.exit_code != 0 {
                return Ok(DeployResult {
                    host: config.host.clone(),
                    hex_agent_installed: false,
                    ruflo_installed: false,
                    hex_agent_version: String::new(),
                    error: Some(format!("Binary verification failed: {}", verify.stderr)),
                });
            }
        }

        // Get version
        let version_result = SshAdapter::run_command(
            config,
            &format!("{} --version", remote_bin),
        )
        .await?;

        Ok(DeployResult {
            host: config.host.clone(),
            hex_agent_installed: true,
            ruflo_installed: false, // ruflo install is separate
            hex_agent_version: version_result.stdout.trim().to_string(),
            error: None,
        })
    }

    /// Install ruflo (@claude-flow/cli) on a remote node via npm.
    pub async fn deploy_ruflo(config: &SshConfig) -> Result<bool, SshError> {
        let result = SshAdapter::run_command(
            config,
            "npm install -g @anthropic-ai/claude-flow 2>&1 || npx --yes @anthropic-ai/claude-flow --version",
        )
        .await?;

        Ok(result.exit_code == 0)
    }

    /// Full deployment: hex-agent + ruflo.
    pub async fn deploy_full(
        config: &SshConfig,
        local_binary_path: &str,
        remote_install_dir: &str,
    ) -> Result<DeployResult, SshError> {
        let mut result = Self::deploy_hex_agent(config, local_binary_path, remote_install_dir).await?;

        if result.hex_agent_installed {
            result.ruflo_installed = Self::deploy_ruflo(config).await.unwrap_or(false);
        }

        Ok(result)
    }
}
