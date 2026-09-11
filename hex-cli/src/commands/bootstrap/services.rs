use std::process::Command;
use std::time::Duration;
use tokio::time::sleep;

#[derive(Debug, Clone)]
pub struct ServiceStatus {
    pub name: String,
    pub running: bool,
    pub pid: Option<u32>,
}

pub struct ServiceStarter {
    force: bool,
    dry_run: bool,
}

impl ServiceStarter {
    pub fn new(force: bool, dry_run: bool) -> Self {
        Self { force, dry_run }
    }

    pub async fn start_all(&self) -> anyhow::Result<Vec<ServiceStatus>> {
        let mut statuses = vec![];

        // Start Ollama
        statuses.push(self.start_ollama().await);

        Ok(statuses)
    }
    async fn start_ollama(&self) -> ServiceStatus {
        if !self.is_port_open(11434).await && !self.force {
            if self.is_process_running("ollama") {
                return ServiceStatus {
                    name: "Ollama".to_string(),
                    running: true,
                    pid: self.get_pid("ollama"),
                };
            }
        }

        if self.dry_run {
            return ServiceStatus {
                name: "Ollama".to_string(),
                running: false,
                pid: None,
            };
        }

        if self.force {
            let _ = Command::new("pkill").arg("-f").arg("ollama").output();
            sleep(Duration::from_millis(500)).await;
        }

        let output = if cfg!(target_os = "macos") {
            Command::new("brew")
                .arg("services")
                .arg("start")
                .arg("ollama")
                .output()
        } else {
            Command::new("ollama")
                .arg("serve")
                .output()
        };

        match output {
            Ok(_) => {
                sleep(Duration::from_secs(1)).await;
                ServiceStatus {
                    name: "Ollama".to_string(),
                    running: true,
                    pid: self.get_pid("ollama"),
                }
            }
            Err(_) => ServiceStatus {
                name: "Ollama".to_string(),
                running: false,
                pid: None,
            },
        }
    }

    async fn is_port_open(&self, port: u16) -> bool {
        match tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)).await {
            Ok(_) => true,
            Err(_) => false,
        }
    }

    fn is_process_running(&self, process_name: &str) -> bool {
        Command::new("pgrep")
            .arg("-f")
            .arg(process_name)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn get_pid(&self, process_name: &str) -> Option<u32> {
        if let Ok(output) = Command::new("pgrep")
            .arg("-f")
            .arg(process_name)
            .output()
        {
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .split('\n')
                .next()
                .and_then(|pid_str| pid_str.parse::<u32>().ok())
        } else {
            None
        }
    }
}
