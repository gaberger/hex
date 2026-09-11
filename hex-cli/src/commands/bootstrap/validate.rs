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
        // SpacetimeDB (3033) and hex-nexus (5555) were checked here too. Both
        // are deleted (ADR-2608241500); an inference backend is the only
        // service hex needs.
        let provider = hex_infer::local_provider();
        service_checks.push((
            provider.display_name.to_string(),
            self.check_service_health(provider.default_port).await,
        ));

        // Check the models this project configures — not a hardcoded list.
        //
        // The list here was three literal ids and had drifted: it checked
        // `gemma4:latest` and `qwen2.5-coder:32b` while `.hex/project.json`
        // declared `gemma4-12b` and `devstral-small-2:24b`. Bootstrap
        // therefore validated three models the project does not use and said
        // nothing about the three it does.
        for (label, _key, model) in hex_infer::configured_tiers() {
            let present = self.model_exists(&model);
            model_checks.push((format!("{model} ({label})"), present));
        }

        // Check config
        let config_exists = Path::new(".hex/project.json").exists();

        // `ready` used to ignore `model_checks` entirely, so an install with
        // the server up, a config file present and not one model pulled
        // reported ready. A readiness claim that cannot be falsified by the
        // thing it is about is not a claim.
        let ready = service_checks.iter().all(|(_, ok)| *ok)
            && !model_checks.is_empty()
            && model_checks.iter().all(|(_, ok)| *ok)
            && config_exists;

        Ok(BootstrapReport {
            ready,
            service_checks,
            model_checks,
            config_exists,
        })
    }

    async fn check_service_health(&self, port: u16) -> bool {
        match tokio::net::TcpStream::connect(hex_infer::local_provider().socket_addr()).await {
            Ok(_) => true,
            Err(_) => false,
        }
    }

    fn model_exists(&self, model_name: &str) -> bool {
        if let Ok(output) = Command::new(hex_infer::local_provider().binary)
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
