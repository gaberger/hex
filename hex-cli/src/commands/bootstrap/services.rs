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

        statuses.push(self.start_inference_server().await);

        Ok(statuses)
    }

    /// Start the local inference server.
    ///
    /// Its name, binary, port and start command come from
    /// `hex_infer::local_provider()` — this file named all four, which is
    /// founding goal G1's failure and also meant a change of server was a
    /// six-file edit.
    ///
    /// **The foreground start is spawned, not awaited.** It used to call
    /// `.output()` on the serve subcommand, which blocks until the child
    /// exits — and a server that exits is a server that failed. On Linux
    /// `hex bootstrap` therefore hung until the operator killed it, and the
    /// `Ok(_)` arm reporting "running: true" was unreachable.
    async fn start_inference_server(&self) -> ServiceStatus {
        let provider = hex_infer::local_provider();
        let name = provider.display_name.to_string();

        if self.is_port_open(provider.default_port).await && !self.force {
            return ServiceStatus { name, running: true, pid: self.get_pid(provider.binary) };
        }
        if !self.force && self.is_process_running(provider.binary) {
            return ServiceStatus { name, running: true, pid: self.get_pid(provider.binary) };
        }
        if self.dry_run {
            return ServiceStatus { name, running: false, pid: None };
        }
        if self.force {
            let _ = Command::new("pkill").arg("-f").arg(provider.binary).output();
            sleep(Duration::from_millis(500)).await;
        }

        let started = if cfg!(target_os = "macos") {
            // `brew services start` returns once the job is registered.
            Command::new("brew")
                .args(["services", "start", provider.binary])
                .output()
                .is_ok()
        } else {
            Command::new(provider.binary)
                .arg(provider.serve_arg)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .is_ok()
        };
        if !started {
            return ServiceStatus { name, running: false, pid: None };
        }

        // Report what is true, not what was attempted: wait for the port.
        for _ in 0..20 {
            sleep(Duration::from_millis(250)).await;
            if self.is_port_open(provider.default_port).await {
                return ServiceStatus { name, running: true, pid: self.get_pid(provider.binary) };
            }
        }
        ServiceStatus { name, running: false, pid: None }
    }

    async fn is_port_open(&self, port: u16) -> bool {
        match tokio::net::TcpStream::connect(hex_infer::local_provider().socket_addr()).await {
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
