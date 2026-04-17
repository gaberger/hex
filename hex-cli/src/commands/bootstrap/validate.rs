use std::path::Path;
use std::process::Command;

#[derive(Debug)]
pub struct BootstrapReport {
    pub ready: bool,
    pub service_checks: Vec<(String, bool)>,
    pub model_checks: Vec<(String, bool)>,
    pub config_exists: bool,
}

impl BootstrapReport {
    pub fn format_success(&self) -> String {
        let mut output = String::from("✓ Bootstrap successful — all systems ready\n\n");
        output.push_str("╭─ Bootstrap Status ─────────────────────╮\n");

        for (service, ready) in &self.service_checks {
            let icon = if *ready { "✓" } else { "✗" };
            output.push_str(&format!("│ {} {} (running)\n", icon, service));
        }

        output.push_str("├─ Models ──────────────────────────────┤\n");
        for (model, ready) in &self.model_checks {
            let icon = if *ready { "✓" } else { "✗" };
            output.push_str(&format!("│ {} {} (loaded)\n", icon, model));
        }

        output.push_str("├─ Status ──────────────────────────────┤\n");
        output.push_str("│ Config:  ✓ created (.hex/project.json)\n");
        output.push_str("│ Ready:   ✓ All systems go\n");
        output.push_str("│ Next:    hex plan execute ...\n");
        output.push_str("╰───────────────────────────────────────╯\n");

        output
    }

    pub fn format_warning(&self) -> String {
        let mut output = String::from("⚠ Bootstrap validation detected issues:\n\n");
        output.push_str("╭─ Validation Report ───────────────────╮\n");

        for (service, ready) in &self.service_checks {
            let icon = if *ready { "✓" } else { "✗" };
            output.push_str(&format!("│ {} {} \n", icon, service));
        }

        for (model, ready) in &self.model_checks {
            let icon = if *ready { "✓" } else { "⚠" };
            output.push_str(&format!("│ {} {} (may load on first use)\n", icon, model));
        }

        if !self.config_exists {
            output.push_str("│ ✗ Config (.hex/project.json) missing\n");
        }

        output.push_str("╰───────────────────────────────────────╯\n");

        output
    }
}

pub struct BootstrapValidator;

impl BootstrapValidator {
    pub fn new() -> Self {
        Self
    }

    pub async fn validate_all(&self) -> anyhow::Result<BootstrapReport> {
        let mut service_checks = vec![];
        let mut model_checks = vec![];

        // Check services
        service_checks.push(("SpacetimeDB".to_string(), self.check_service_health(3033).await));
        service_checks.push(("hex-nexus".to_string(), self.check_service_health(5555).await));
        service_checks.push(("Ollama".to_string(), self.check_service_health(11434).await));

        // Check models (quick existence check, don't wait for loading)
        model_checks.push(("qwen3:4b (T1)".to_string(), self.model_exists("qwen3:4b")));
        model_checks.push(("qwen2.5-coder:32b (T2)".to_string(), self.model_exists("qwen2.5-coder:32b")));
        model_checks.push(("devstral-small-2:24b (T2.5)".to_string(), self.model_exists("devstral-small-2:24b")));

        // Check config
        let config_exists = Path::new(".hex/project.json").exists();

        let ready = service_checks.iter().all(|(_, ok)| *ok)
            && config_exists;

        Ok(BootstrapReport {
            ready,
            service_checks,
            model_checks,
            config_exists,
        })
    }

    async fn check_service_health(&self, port: u16) -> bool {
        match tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)).await {
            Ok(_) => true,
            Err(_) => false,
        }
    }

    fn model_exists(&self, model_name: &str) -> bool {
        if let Ok(output) = Command::new("ollama")
            .arg("show")
            .arg(model_name)
            .output()
        {
            output.status.success()
        } else {
            false
        }
    }
}
