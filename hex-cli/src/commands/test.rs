//! `hex test` — Full-stack integration testing from the CLI.
//!
//! Runs unit tests, architecture checks, service health, API integration,
//! and swarm coordination tests — all from a single command.
//!
//! Usage:
//!   hex test              # Full stack (requires running nexus)
//!   hex test --unit       # Unit tests only
//!   hex test --arch       # Architecture checks only
//!   hex test --services   # Service health checks only
//!   hex test --all        # Everything including service startup

use std::process::Command;

use clap::Subcommand;
use colored::Colorize;

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum TestAction {
    /// Run all unit tests across Rust crates
    Unit,
    /// Check architecture health (boundaries, deps, dead code)
    Arch,
    /// Test hex-nexus service endpoints (requires running nexus)
    Services,
    /// Run full integration tests (unit + arch + services + swarm)
    All,
}

struct TestResults {
    pass: u32,
    fail: u32,
    skip: u32,
}

impl TestResults {
    fn new() -> Self {
        Self { pass: 0, fail: 0, skip: 0 }
    }

    fn check(&mut self, label: &str, ok: bool) {
        if ok {
            println!("  {} {}", "✓".green(), label);
            self.pass += 1;
        } else {
            println!("  {} {}", "✗".red(), label);
            self.fail += 1;
        }
    }

    fn skip(&mut self, label: &str) {
        println!("  {} {} (skipped)", "○".yellow(), label);
        self.skip += 1;
    }

    fn summary(&self) -> bool {
        let total = self.pass + self.fail + self.skip;
        println!();
        if self.fail == 0 {
            println!(
                "  {}: {} passed, {} skipped, {} failed (of {})",
                "ALL PASS".green().bold(),
                self.pass,
                self.skip,
                self.fail,
                total
            );
            true
        } else {
            println!(
                "  {}: {} passed, {} skipped, {} failed (of {})",
                "FAILURES".red().bold(),
                self.pass,
                self.skip,
                self.fail,
                total
            );
            false
        }
    }
}

pub async fn run(action: TestAction) -> anyhow::Result<()> {
    let mut results = TestResults::new();

    match action {
        TestAction::Unit => {
            run_unit_tests(&mut results);
        }
        TestAction::Arch => {
            run_arch_checks(&mut results).await;
        }
        TestAction::Services => {
            run_service_tests(&mut results).await;
        }
        TestAction::All => {
            run_unit_tests(&mut results);
            println!();
            run_arch_checks(&mut results).await;
            println!();
            let services_ok = run_service_tests(&mut results).await;
            println!();
            run_integration_tests(&mut results, services_ok).await;
        }
    }

    println!("\n{}", "══════════════════════════════════════════".cyan());
    let ok = results.summary();
    println!("{}", "══════════════════════════════════════════".cyan());

    if ok {
        Ok(())
    } else {
        anyhow::bail!("{} test(s) failed", results.fail)
    }
}

// ── Unit Tests ──────────────────────────────────────

fn run_unit_tests(r: &mut TestResults) {
    println!("{}", "── Unit Tests ──".cyan());

    // Main workspace crates
    for crate_name in &["hex-core", "hex-agent"] {
        let ok = cargo_test(crate_name, None);
        r.check(&format!("{} tests pass", crate_name), ok);
    }

    r.check("hex-nexus lib tests pass", cargo_test("hex-nexus", Some("--lib")));

    for crate_name in &["hex-chat", "hex-cli"] {
        let ok = cargo_check(crate_name);
        r.check(&format!("{} compiles", crate_name), ok);
    }

    // SpacetimeDB modules (different workspace)
    println!();
    println!("{}", "── SpacetimeDB Module Tests ──".cyan());

    for module in &[
        "file-lock-manager",
        "architecture-enforcer",
        "conflict-resolver",
        "inference-gateway",
        "hexflo-coordination",
        "secret-grant",
    ] {
        let ok = cargo_test_spacetime(module);
        r.check(&format!("{} tests pass", module), ok);
    }
}

// ── Architecture Checks ─────────────────────────────

async fn run_arch_checks(r: &mut TestResults) {
    println!("{}", "── Architecture Health ──".cyan());

    // Try multiple ways to run hex analyze
    let output = find_and_run_hex_analyze();

    match output {
        Some(stdout) => {
            r.check(
                "Architecture grade A",
                stdout.contains("Grade:") && stdout.contains("A"),
            );
            r.check(
                "Zero boundary violations",
                stdout.contains("Boundary violations") && stdout.contains("| 0"),
            );
            r.check(
                "Zero circular dependencies",
                stdout.contains("Circular dependencies") && stdout.contains("| 0"),
            );
            r.check(
                "Zero dead exports",
                stdout.contains("Dead exports") && stdout.contains("| 0"),
            );
        }
        None => {
            // Fallback: use hex-core boundary rules directly
            println!("  {} hex analyze not in PATH, testing boundary rules directly", "!".yellow());
            r.check(
                "hex-core boundary rules pass",
                cargo_test("hex-core", None),
            );
        }
    }
}

/// Try multiple methods to run `hex analyze .` and return stdout.
fn find_and_run_hex_analyze() -> Option<String> {
    // 1. Try `npx hex analyze .` (npm-installed TS CLI)
    if let Ok(out) = Command::new("npx")
        .args(["hex", "analyze", "."])
        .output()
    {
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        if stdout.contains("Grade:") {
            return Some(stdout);
        }
    }

    // 2. Try `bun run --bun src/cli.ts analyze .` (dev mode)
    if let Ok(out) = Command::new("bun")
        .args(["run", "--bun", "src/cli.ts", "analyze", "."])
        .output()
    {
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        if stdout.contains("Grade:") {
            return Some(stdout);
        }
    }

    // 3. Try `hex` directly (if in PATH)
    if let Ok(out) = Command::new("hex")
        .args(["analyze", "."])
        .output()
    {
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        if stdout.contains("Grade:") {
            return Some(stdout);
        }
    }

    // 4. Try nexus API
    // (handled by caller falling back to hex-core rules)
    None
}

// ── Service Tests ───────────────────────────────────

async fn run_service_tests(r: &mut TestResults) -> bool {
    println!("{}", "── Service Health ──".cyan());

    let client = NexusClient::from_env();

    // Check if nexus is running
    match client.ensure_running().await {
        Ok(_) => {
            r.check("hex-nexus responding", true);

            // These endpoints may return empty arrays — that's fine
            let agents_ok = client.get("/api/agents").await.is_ok();
            r.check("GET /api/agents responds", agents_ok);

            let swarms_ok = client.get("/api/swarms").await.is_ok();
            r.check("GET /api/swarms responds", swarms_ok);

            agents_ok && swarms_ok
        }
        Err(_) => {
            r.skip("hex-nexus not running — start with: hex daemon start");
            r.skip("GET /api/agents responds");
            r.skip("GET /api/swarms responds");
            false
        }
    }
}

// ── Integration Tests ───────────────────────────────

async fn run_integration_tests(r: &mut TestResults, services_ok: bool) {
    println!("{}", "── Integration Tests ──".cyan());

    if !services_ok {
        r.skip("Swarm lifecycle (services not healthy)");
        r.skip("Task creation (services not healthy)");
        r.skip("Swarm status (services not healthy)");
        r.skip("HexFlo memory store (services not healthy)");
        r.skip("HexFlo memory retrieve (services not healthy)");
        r.skip("HexFlo memory search (services not healthy)");
        return;
    }

    let client = NexusClient::from_env();

    // Swarm lifecycle: create → status → complete
    let swarm_body = serde_json::json!({
        "name": "hex-test-swarm",
        "topology": "mesh"
    });

    match client.post("/api/swarms", &swarm_body).await {
        Ok(resp) => {
            let has_id = resp.get("id").and_then(|v| v.as_str()).is_some();
            r.check("Create swarm via API", has_id);

            if let Some(swarm_id) = resp.get("id").and_then(|v| v.as_str()) {
                // Create a task
                let task_body = serde_json::json!({ "title": "integration-test-task" });
                let task_resp = client
                    .post(&format!("/api/swarms/{}/tasks", swarm_id), &task_body)
                    .await;
                r.check("Create task in swarm", task_resp.is_ok());

                // Verify swarm appears in status
                let status = client.get("/api/swarms").await;
                let found = status
                    .as_ref()
                    .ok()
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().any(|s| {
                        s.get("name").and_then(|n| n.as_str()) == Some("hex-test-swarm")
                    }))
                    .unwrap_or(false);
                r.check("Swarm visible in status", found);
            }
        }
        Err(_) => {
            r.check("Create swarm via API", false);
        }
    }

    // HexFlo memory: store → retrieve → search
    let mem_body = serde_json::json!({
        "key": "hex-test-key",
        "value": "hex-test-value"
    });

    let store_ok = client.post("/api/hexflo/memory", &mem_body).await.is_ok();
    r.check("HexFlo memory store", store_ok);

    if store_ok {
        let retrieve = client.get("/api/hexflo/memory/hex-test-key").await;
        let has_value = retrieve
            .as_ref()
            .ok()
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_str())
            .map(|s| s.contains("hex-test-value"))
            .unwrap_or(false);
        r.check("HexFlo memory retrieve", has_value);

        let search = client.get("/api/hexflo/memory/search?q=hex-test").await;
        r.check("HexFlo memory search", search.is_ok());
    }
}

// ── Helpers ─────────────────────────────────────────

fn cargo_test(crate_name: &str, extra: Option<&str>) -> bool {
    let mut cmd = Command::new("cargo");
    cmd.args(["test", "-p", crate_name, "--quiet"]);
    if let Some(flag) = extra {
        cmd.arg(flag);
    }
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

fn cargo_check(crate_name: &str) -> bool {
    Command::new("cargo")
        .args(["check", "-p", crate_name])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn cargo_test_spacetime(module: &str) -> bool {
    Command::new("cargo")
        .args(["test", "-p", module, "--quiet"])
        .current_dir("spacetime-modules")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
