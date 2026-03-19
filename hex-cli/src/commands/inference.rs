//! `hex inference` — Manage inference providers (Ollama, vLLM, etc.)
//!
//! Register, list, and test self-hosted LLM endpoints.
//!
//! Usage:
//!   hex inference add ollama http://bazzite.local:11434 --model qwen3:32b
//!   hex inference add vllm http://gpu-server:8000 --model Qwen/Qwen3-32B
//!   hex inference list
//!   hex inference test <provider-id>
//!   hex inference discover              # Auto-discover Ollama on local network

use clap::Subcommand;
use colored::Colorize;

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum InferenceAction {
    /// Register a new inference provider
    Add {
        /// Provider type: ollama, vllm, openai-compat
        provider_type: String,
        /// Base URL (e.g., http://bazzite.local:11434)
        url: String,
        /// Model name (e.g., qwen3:32b)
        #[arg(long)]
        model: Option<String>,
        /// API key (not needed for Ollama)
        #[arg(long)]
        key: Option<String>,
        /// Provider ID (auto-generated if omitted)
        #[arg(long)]
        id: Option<String>,
    },
    /// List registered inference providers
    List,
    /// Test connectivity to a provider
    Test {
        /// Provider ID or URL
        target: String,
    },
    /// Auto-discover Ollama instances on common addresses
    Discover,
    /// Remove a registered provider
    Remove {
        /// Provider ID
        provider_id: String,
    },
}

pub async fn run(action: InferenceAction) -> anyhow::Result<()> {
    match action {
        InferenceAction::Add { provider_type, url, model, key, id } => {
            add_provider(&provider_type, &url, model.as_deref(), key.as_deref(), id.as_deref()).await
        }
        InferenceAction::List => list_providers().await,
        InferenceAction::Test { target } => test_provider(&target).await,
        InferenceAction::Discover => discover_ollama().await,
        InferenceAction::Remove { provider_id } => remove_provider(&provider_id).await,
    }
}

async fn add_provider(
    provider_type: &str,
    url: &str,
    model: Option<&str>,
    key: Option<&str>,
    id: Option<&str>,
) -> anyhow::Result<()> {
    let provider_id = id.unwrap_or(provider_type);
    let model_name = model.unwrap_or(match provider_type {
        "ollama" => "qwen3:32b",
        "vllm" => "default",
        _ => "default",
    });

    println!("{}", "Registering inference provider...".cyan());

    // First test connectivity
    let test_url = match provider_type {
        "ollama" => format!("{}/api/tags", url.trim_end_matches('/')),
        _ => format!("{}/v1/models", url.trim_end_matches('/')),
    };

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;

    match http.get(&test_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            println!("  {} Connectivity OK ({})", "✓".green(), resp.status());

            // If Ollama, list available models
            if provider_type == "ollama" {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    if let Some(models) = body.get("models").and_then(|m| m.as_array()) {
                        println!("  {} Available models:", "ℹ".cyan());
                        for m in models.iter().take(10) {
                            let name = m.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                            let size = m.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
                            println!("    - {} ({:.1}GB)", name, size as f64 / 1_073_741_824.0);
                        }
                    }
                }
            }
        }
        Ok(resp) => {
            println!("  {} Provider responded with {}", "!".yellow(), resp.status());
        }
        Err(e) => {
            println!("  {} Cannot reach {}: {}", "!".yellow(), url, e);
            println!("  Provider will be registered anyway (may come online later).");
        }
    }

    // Register with nexus if running
    let client = NexusClient::from_env();
    if client.ensure_running().await.is_ok() {
        let body = serde_json::json!({
            "id": provider_id,
            "provider": provider_type,
            "url": url.trim_end_matches('/'),
            "model": model_name,
            "requires_auth": key.is_some(),
            "secret_key": key.unwrap_or(""),
        });

        match client.post("/api/inference/register", &body).await {
            Ok(_) => println!("  {} Registered with hex-nexus", "✓".green()),
            Err(e) => println!("  {} Nexus registration failed: {}", "!".yellow(), e),
        }
    } else {
        println!("  {} hex-nexus not running — provider saved locally only", "!".yellow());
    }

    println!();
    println!("{} Provider registered:", "✓".green());
    println!("  ID:    {}", provider_id);
    println!("  Type:  {}", provider_type);
    println!("  URL:   {}", url);
    println!("  Model: {}", model_name);
    println!();
    println!("Use with hex-agent:");
    println!("  HEX_OLLAMA_HOST={} HEX_OLLAMA_MODEL={} hex-agent --project-dir .", url, model_name);

    Ok(())
}

async fn list_providers() -> anyhow::Result<()> {
    let client = NexusClient::from_env();

    println!("{}", "── Inference Providers ──".cyan());
    println!();

    // Check env vars for configured providers
    let env_providers = [
        ("HEX_OLLAMA_HOST", "HEX_OLLAMA_MODEL", "ollama"),
        ("HEX_VLLM_HOST", "HEX_VLLM_MODEL", "vllm"),
        ("HEX_INFERENCE_URL", "HEX_INFERENCE_MODEL", "generic"),
    ];

    let mut found_env = false;
    for (host_var, model_var, ptype) in &env_providers {
        if let Ok(host) = std::env::var(host_var) {
            let model = std::env::var(model_var).unwrap_or_else(|_| "default".to_string());
            println!("  {} {} (env)", "●".green(), ptype);
            println!("    URL:   {}", host);
            println!("    Model: {}", model);
            found_env = true;
        }
    }

    // Check Anthropic
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        println!("  {} anthropic (env)", "●".green());
        println!("    URL:   https://api.anthropic.com");
        found_env = true;
    }

    if !found_env {
        println!("  No providers configured via environment variables.");
    }

    // Query nexus for registered providers
    if client.ensure_running().await.is_ok() {
        println!();
        println!("{}", "── Nexus-Registered Providers ──".cyan());
        match client.get("/api/inference/endpoints").await {
            Ok(data) => {
                let endpoints = data.get("endpoints").and_then(|v| v.as_array());
                if let Some(arr) = endpoints {
                    if arr.is_empty() {
                        println!("  No providers registered in nexus.");
                    }
                    for p in arr {
                        let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                        let provider = p.get("provider").and_then(|v| v.as_str()).unwrap_or("?");
                        let url = p.get("url").and_then(|v| v.as_str()).unwrap_or("?");
                        let model = p.get("model").and_then(|v| v.as_str()).unwrap_or("default");
                        let status = p.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
                        let icon = if status == "healthy" || status == "ok" { "●".green() } else { "○".yellow() };
                        println!("  {} {} ({}) — {} [model: {}]", icon, id, provider, url, model);
                    }
                } else {
                    println!("  No providers registered in nexus.");
                }
            }
            Err(_) => println!("  Could not fetch providers from nexus."),
        }
    }

    println!();
    println!("Register new: hex inference add ollama http://host:11434 --model qwen3:32b");

    Ok(())
}

async fn test_provider(target: &str) -> anyhow::Result<()> {
    println!("{} Testing {}...", "→".cyan(), target);

    let url = if target.starts_with("http") {
        target.to_string()
    } else {
        // Assume it's an Ollama host shorthand
        format!("http://{}:11434", target)
    };

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    // Test Ollama /api/tags
    let ollama_url = format!("{}/api/tags", url.trim_end_matches('/'));
    println!("  {} GET {}", "→".cyan(), ollama_url);
    match http.get(&ollama_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            println!("  {} Ollama responding at {}", "✓".green(), url);
            let mut first_local_model: Option<String> = None;
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                if let Some(models) = body.get("models").and_then(|m| m.as_array()) {
                    println!("  {} {} model(s) available:", "ℹ".cyan(), models.len());
                    for m in models {
                        let name = m.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                        let size = m.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
                        let gb = size as f64 / 1_073_741_824.0;
                        // Skip cloud/remote models for inference test
                        let is_local = m.get("remote_model").is_none() && size > 0;
                        if is_local && first_local_model.is_none() {
                            first_local_model = Some(name.to_string());
                        }
                        println!("    - {} ({:.1}GB){}", name, gb,
                            if !is_local { " [cloud]" } else { "" });
                    }
                }
            }

            // Quick inference test using first available local model
            if let Some(ref test_model) = first_local_model {
                println!();
                println!("  {} Running inference test with {}...", "→".cyan(), test_model);
                let chat_url = format!("{}/v1/chat/completions", url.trim_end_matches('/'));
                let test_body = serde_json::json!({
                    "model": test_model,
                    "messages": [{"role": "user", "content": "Reply with just the word 'ok'"}],
                    "max_tokens": 10,
                });

                let start = std::time::Instant::now();
                match http.post(&chat_url).json(&test_body).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        let latency = start.elapsed().as_millis();
                        println!("  {} Inference OK — {} responded in {}ms", "✓".green(), test_model, latency);
                        println!();
                        println!("  Use with hex-agent:");
                        println!("    HEX_OLLAMA_HOST={} HEX_OLLAMA_MODEL={} hex-agent --project-dir .", url, test_model);
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        println!("  {} Inference returned {} — {}", "!".yellow(), status, body.chars().take(200).collect::<String>());
                    }
                    Err(e) => {
                        println!("  {} Inference failed: {}", "✗".red(), e);
                    }
                }
            } else {
                println!();
                println!("  {} No local models found — pull one with: ollama pull qwen3.5:27b", "!".yellow());
            }
        }
        Ok(resp) => {
            println!("  {} Ollama returned HTTP {} at {}", "!".yellow(), resp.status(), ollama_url);
            // Try OpenAI-compatible /v1/models as fallback
            let oai_url = format!("{}/v1/models", url.trim_end_matches('/'));
            println!("  {} GET {}", "→".cyan(), oai_url);
            match http.get(&oai_url).send().await {
                Ok(r) if r.status().is_success() => {
                    println!("  {} OpenAI-compatible API at {}", "✓".green(), url);
                }
                Ok(r) => {
                    println!("  {} OpenAI endpoint returned HTTP {}", "!".yellow(), r.status());
                }
                Err(e) => {
                    println!("  {} OpenAI endpoint failed: {}", "✗".red(), e);
                }
            }
        }
        Err(e) => {
            println!("  {} Cannot reach {}: {}", "✗".red(), url, e);
            println!();
            println!("  Troubleshooting:");
            if e.is_timeout() {
                println!("    - Connection timed out (10s) — host may be unreachable");
            } else if e.is_connect() {
                println!("    - Connection refused — is Ollama running?");
                println!("    - Ollama may be bound to localhost only. Fix with:");
                println!("      OLLAMA_HOST=0.0.0.0 ollama serve");
            } else {
                println!("    - {}", e);
            }
            println!("    - Verify: curl {}/api/tags", url);
        }
    }

    Ok(())
}

async fn discover_ollama() -> anyhow::Result<()> {
    println!("{}", "── Discovering Inference Providers ──".cyan());
    println!();

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;

    let mut found = 0;

    // ── 1. Query SpacetimeDB via nexus (source of truth) ──────────
    let client = NexusClient::from_env();
    let mut registered_urls: Vec<String> = Vec::new();

    if client.ensure_running().await.is_ok() {
        println!("{}", "── Registered Providers (SpacetimeDB) ──".cyan());
        match client.get("/api/inference/providers").await {
            Ok(providers) => {
                if let Some(arr) = providers.as_array() {
                    for p in arr {
                        let id = p.get("provider_id").and_then(|v| v.as_str()).unwrap_or("?");
                        let ptype = p.get("provider_type").and_then(|v| v.as_str()).unwrap_or("?");
                        let url = p.get("base_url").and_then(|v| v.as_str()).unwrap_or("?");

                        // Verify registered providers are still reachable (live check, not cached healthy flag)
                        let reachable = if ptype == "ollama" {
                            http.get(format!("{}/api/tags", url.trim_end_matches('/')))
                                .send().await
                                .map(|r| r.status().is_success())
                                .unwrap_or(false)
                        } else {
                            http.get(format!("{}/v1/models", url.trim_end_matches('/')))
                                .send().await
                                .map(|r| r.status().is_success())
                                .unwrap_or(false)
                        };

                        let icon = if reachable { "●".green() } else { "○".red() };
                        let status = if reachable { "online" } else { "offline" };
                        println!("  {} {} ({}) — {} [{}]", icon, id, ptype, url, status);
                        registered_urls.push(url.to_string());
                        if reachable { found += 1; }
                    }
                    if arr.is_empty() {
                        println!("  No providers registered yet.");
                    }
                }
            }
            Err(_) => {
                println!("  Provider registry endpoint not available.");
            }
        }
        println!();
    }

    // ── 2. LAN scan for unregistered Ollama instances ─────────────
    println!("{}", "── LAN Scan (unregistered) ──".cyan());

    let candidates = [
        ("localhost", "http://127.0.0.1:11434"),
        ("bazzite", "http://bazzite:11434"),
        ("bazzite.local", "http://bazzite.local:11434"),
        ("Docker host", "http://host.docker.internal:11434"),
    ];

    let mut new_found = 0;
    for (label, url) in &candidates {
        // Skip if already registered in SpacetimeDB
        if registered_urls.iter().any(|r| r.contains(url.trim_start_matches("http://"))) {
            continue;
        }

        let test_url = format!("{}/api/tags", url);
        match http.get(&test_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let model_info = resp
                    .json::<serde_json::Value>()
                    .await
                    .ok()
                    .and_then(|v| {
                        let models = v.get("models")?.as_array()?;
                        let names: Vec<&str> = models.iter()
                            .filter_map(|m| m.get("name")?.as_str())
                            .collect();
                        Some((models.len(), names.join(", ")))
                    });

                if let Some((count, names)) = model_info {
                    println!("  {} {} — {} ({} models: {})", "●".green(), label, url, count, names);
                    println!("    → Register with: hex inference add ollama {} --model <model>", url);
                } else {
                    println!("  {} {} — {} (reachable)", "●".green(), label, url);
                }
                new_found += 1;
                found += 1;
            }
            _ => {} // Don't show unreachable candidates — too noisy
        }
    }

    if new_found == 0 {
        println!("  No unregistered Ollama instances found on LAN.");
    }

    println!();
    if found == 0 {
        println!("No inference providers found.");
        println!("  Start Ollama: ollama serve");
        println!("  Or register:  hex inference add ollama http://<host>:11434 --model <model>");
    } else {
        println!("{} {} provider(s) available.", "✓".green(), found);
    }

    Ok(())
}

async fn remove_provider(provider_id: &str) -> anyhow::Result<()> {
    let client = NexusClient::from_env();
    client.ensure_running().await?;

    match client.post(
        &format!("/api/inference/providers/{}/remove", provider_id),
        &serde_json::json!({}),
    ).await {
        Ok(_) => println!("{} Removed provider: {}", "✓".green(), provider_id),
        Err(e) => println!("{} Failed to remove: {}", "✗".red(), e),
    }

    Ok(())
}
