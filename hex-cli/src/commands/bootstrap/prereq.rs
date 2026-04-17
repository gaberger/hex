use std::process::Command;

#[derive(Debug, Clone)]
pub struct PrereqStatus {
    pub name: String,
    pub installed: bool,
    pub version: Option<String>,
    pub install_cmd: Option<String>,
}

#[derive(Debug)]
pub struct PrereqReport {
    pub statuses: Vec<PrereqStatus>,
}

impl PrereqReport {
    pub fn all_ok(&self) -> bool {
        self.statuses.iter().all(|s| s.installed)
    }

    pub fn format(&self) -> String {
        let mut output = String::from("⚠ Prerequisites check failed:\n");
        for status in &self.statuses {
            if !status.installed {
                output.push_str(&format!("  ✗ {} (not found)\n", status.name));
                if let Some(cmd) = &status.install_cmd {
                    output.push_str(&format!("    Install: {}\n", cmd));
                }
            }
        }
        output
    }
}

pub struct PrereqChecker;

impl PrereqChecker {
    pub fn new() -> Self {
        Self
    }

    pub async fn check_all(&self) -> anyhow::Result<PrereqReport> {
        let mut statuses = vec![];

        // Check SpacetimeDB
        statuses.push(self.check_spacetimedb());

        // Check Ollama
        statuses.push(self.check_ollama());

        // Check Rust
        statuses.push(self.check_rust());

        // Check Bun
        statuses.push(self.check_bun());

        // Check Cargo
        statuses.push(self.check_cargo());

        Ok(PrereqReport { statuses })
    }

    fn check_spacetimedb(&self) -> PrereqStatus {
        if self.command_exists("spacetime") {
            if let Ok(output) = Command::new("spacetime").arg("--version").output() {
                let version = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .to_string();
                PrereqStatus {
                    name: "SpacetimeDB".to_string(),
                    installed: true,
                    version: Some(version),
                    install_cmd: None,
                }
            } else {
                PrereqStatus {
                    name: "SpacetimeDB".to_string(),
                    installed: true,
                    version: None,
                    install_cmd: None,
                }
            }
        } else {
            let cmd = if cfg!(target_os = "macos") {
                "brew install spacetimedb"
            } else {
                "curl --proto '=https' --tlsv1.2 -sSf https://install.spacetimedb.com | sh"
            };
            PrereqStatus {
                name: "SpacetimeDB".to_string(),
                installed: false,
                version: None,
                install_cmd: Some(cmd.to_string()),
            }
        }
    }

    fn check_ollama(&self) -> PrereqStatus {
        if self.command_exists("ollama") {
            if let Ok(output) = Command::new("ollama").arg("--version").output() {
                let version = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .to_string();
                PrereqStatus {
                    name: "Ollama".to_string(),
                    installed: true,
                    version: Some(version),
                    install_cmd: None,
                }
            } else {
                PrereqStatus {
                    name: "Ollama".to_string(),
                    installed: true,
                    version: None,
                    install_cmd: None,
                }
            }
        } else {
            let cmd = if cfg!(target_os = "macos") {
                "brew install ollama"
            } else {
                "curl https://ollama.ai/install.sh | sh"
            };
            PrereqStatus {
                name: "Ollama".to_string(),
                installed: false,
                version: None,
                install_cmd: Some(cmd.to_string()),
            }
        }
    }

    fn check_rust(&self) -> PrereqStatus {
        if self.command_exists("rustc") {
            if let Ok(output) = Command::new("rustc").arg("--version").output() {
                let version = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .to_string();
                PrereqStatus {
                    name: "Rust".to_string(),
                    installed: true,
                    version: Some(version),
                    install_cmd: None,
                }
            } else {
                PrereqStatus {
                    name: "Rust".to_string(),
                    installed: true,
                    version: None,
                    install_cmd: None,
                }
            }
        } else {
            PrereqStatus {
                name: "Rust".to_string(),
                installed: false,
                version: None,
                install_cmd: Some("curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh".to_string()),
            }
        }
    }

    fn check_bun(&self) -> PrereqStatus {
        if self.command_exists("bun") {
            if let Ok(output) = Command::new("bun").arg("--version").output() {
                let version = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .to_string();
                PrereqStatus {
                    name: "Bun".to_string(),
                    installed: true,
                    version: Some(version),
                    install_cmd: None,
                }
            } else {
                PrereqStatus {
                    name: "Bun".to_string(),
                    installed: true,
                    version: None,
                    install_cmd: None,
                }
            }
        } else {
            PrereqStatus {
                name: "Bun".to_string(),
                installed: false,
                version: None,
                install_cmd: Some("curl -fsSL https://bun.sh/install | bash".to_string()),
            }
        }
    }

    fn check_cargo(&self) -> PrereqStatus {
        if self.command_exists("cargo") {
            if let Ok(output) = Command::new("cargo").arg("--version").output() {
                let version = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .to_string();
                PrereqStatus {
                    name: "Cargo".to_string(),
                    installed: true,
                    version: Some(version),
                    install_cmd: None,
                }
            } else {
                PrereqStatus {
                    name: "Cargo".to_string(),
                    installed: true,
                    version: None,
                    install_cmd: None,
                }
            }
        } else {
            PrereqStatus {
                name: "Cargo".to_string(),
                installed: false,
                version: None,
                install_cmd: Some("curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh".to_string()),
            }
        }
    }

    fn command_exists(&self, cmd: &str) -> bool {
        Command::new("which")
            .arg(cmd)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}
