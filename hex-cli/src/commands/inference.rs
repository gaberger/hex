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
        /// Provider type: ollama, vllm, openai-compat, openrouter
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
        /// Quantization level: q2, q3, q4, q8, fp16, cloud.
        /// Auto-detected from Ollama model name if omitted (e.g. ':q4_k_m' → q4).
        #[arg(long)]
        quantization: Option<String>,
    },
    /// List registered inference providers
    List,
    /// Test connectivity to a provider
    Test {
        /// Provider ID or URL
        target: String,
    },
    /// Auto-discover inference providers
    Discover {
        /// Provider to discover: ollama (default, LAN scan), openrouter (fetch model catalog)
        #[arg(long, default_value = "ollama")]
        provider: String,
        /// Filter models by name substring
        #[arg(long)]
        filter: Option<String>,
        /// Minimum context window size
        #[arg(long)]
        min_context: Option<u64>,
        /// Remove registered providers that return empty responses
        #[arg(long)]
        prune: bool,
    },
    /// Remove a registered provider
    Remove {
        /// Provider ID
        provider_id: String,
    },
    /// Register and calibrate the key default models (run once after install)
    Setup,
    /// Watch for queued inference tasks and dispatch them autonomously via claude subprocess
    Watch {
        /// Agent ID (auto-resolved from session file if omitted)
        #[arg(long)]
        agent_id: Option<String>,
        /// Run as background daemon (suppress output)
        #[arg(long)]
        daemon: bool,
    },
    /// List pending inference queue tasks
    Queue,
}

pub async fn run(action: InferenceAction) -> anyhow::Result<()> {
    match action {
        InferenceAction::Add { provider_type, url, model, key, id, quantization } => {
            add_provider(&provider_type, &url, model.as_deref(), key.as_deref(), id.as_deref(), quantization.as_deref()).await
        }
        InferenceAction::List => list_providers().await,
        InferenceAction::Test { target } => test_provider(&target).await,
        InferenceAction::Discover { provider, filter, min_context, prune } => {
            match provider.as_str() {
                "openrouter" => discover_openrouter(filter.as_deref(), min_context).await,
                _ => discover_ollama(prune).await,
            }
        }
        InferenceAction::Remove { provider_id } => remove_provider(&provider_id).await,
        InferenceAction::Setup => setup_defaults().await,
        InferenceAction::Watch { agent_id, daemon } => watch(agent_id, daemon).await,
        InferenceAction::Queue => queue_list().await,
    }
}

async fn add_provider(
    provider_type: &str,
    url: &str,
    model: Option<&str>,
    key: Option<&str>,
    id: Option<&str>,
    quantization: Option<&str>,
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

    let mut discovered_models: Vec<String> = vec![model_name.to_string()];

    match http.get(&test_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            println!("  {} Connectivity OK ({})", "✓".green(), resp.status());

            // If Ollama, list available models and capture them
            if provider_type == "ollama" {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    if let Some(models) = body.get("models").and_then(|m| m.as_array()) {
                        discovered_models.clear();
                        println!("  {} Available models:", "ℹ".cyan());
                        for m in models.iter().take(20) {
                            let name = m.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                            let size = m.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
                            println!("    - {} ({:.1}GB)", name, size as f64 / 1_073_741_824.0);
                            discovered_models.push(name.to_string());
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

    let models_json = serde_json::to_string(&discovered_models).unwrap_or_else(|_| format!("[\"{}\"]", model_name));

    // Resolve quantization level (ADR-2603271000):
    // 1. Explicit --quantization flag
    // 2. Auto-detect from model name GGUF tag
    // 3. Default: "cloud" for API providers, "q4" for local
    let resolved_quantization: Option<String> = match quantization {
        Some(q) => {
            // Validate the provided value
            if q.parse::<hex_core::QuantizationLevel>().is_err() {
                println!("  {} Unknown quantization level '{}'. Valid values: q2, q3, q4, q8, fp16, cloud", "!".yellow(), q);
                println!("  Defaulting to q4.");
                Some("q4".to_string())
            } else {
                Some(q.to_string())
            }
        }
        None => {
            match provider_type {
                "ollama" | "vllm" => {
                    match hex_core::QuantizationLevel::detect_from_model_name(model_name) {
                        Some(level) => {
                            println!("  {} Detected quantization: {} (from model name)", "ℹ".cyan(), level);
                            Some(level.to_string())
                        }
                        None => {
                            println!("  {} Could not detect quantization from model name '{}'; defaulting to q4.", "!".yellow(), model_name);
                            println!("  Use --quantization to set explicitly.");
                            Some("q4".to_string())
                        }
                    }
                }
                "openrouter" => {
                    println!("  {} Cloud API provider — quantization: cloud", "ℹ".cyan());
                    Some("cloud".to_string())
                }
                _ => None,
            }
        }
    };

    // Register with nexus if running
    let client = NexusClient::from_env();
    if client.ensure_running().await.is_ok() {
        let mut body = serde_json::json!({
            "id": provider_id,
            "provider": provider_type,
            "url": url.trim_end_matches('/'),
            "model": model_name,
            "models_json": models_json,
            "requires_auth": key.is_some(),
            "secret_key": key.unwrap_or(""),
        });
        if let Some(ref q) = resolved_quantization {
            body["quantization"] = serde_json::Value::String(q.clone());
        }
        let body = body;

        match client.post("/api/inference/register", &body).await {
            Ok(_) => {
                println!("  {} Registered with hex-nexus", "✓".green());
                // Quality gate (ADR-2603311000): verify via nexus test endpoint.
                // Empty or error response means the model ID doesn't exist or is misconfigured.
                match client.get(&format!("/api/inference/test/{}", provider_id)).await {
                    Ok(resp) => {
                        let content = resp.get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        if content.is_empty() {
                            let _ = client.delete(&format!("/api/inference/providers/{}", provider_id)).await;
                            return Err(anyhow::anyhow!(
                                "provider '{}' registered but returned empty response — removed. Check model ID.",
                                provider_id
                            ));
                        }
                        println!("  {} Model validation passed (reply: {:?})", "✓".green(), content);
                    }
                    Err(_) => {
                        let _ = client.delete(&format!("/api/inference/providers/{}", provider_id)).await;
                        return Err(anyhow::anyhow!(
                            "provider '{}' registered but returned empty response — removed. Check model ID.",
                            provider_id
                        ));
                    }
                }
            }
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
    if let Some(ref q) = resolved_quantization {
        println!("  Quant: {}", q);
    }
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
                        let quant = p.get("quantizationLevel").and_then(|v| v.as_str()).unwrap_or("?");
                        let quality = p.get("qualityScore").and_then(|v| v.as_f64())
                            .map(|f| format!(" q={:.2}", f))
                            .unwrap_or_default();
                        let icon = if status == "healthy" || status == "ok" { "●".green() } else { "○".yellow() };
                        println!("  {} {} ({}) — {} [model: {}] [quant: {}{}]", icon, id, provider, url, model, quant, quality);
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

    // Look up full provider record when given an ID (not a raw URL).
    struct ProviderRecord {
        url: String,
        provider_type: String,
        model: String,
    }

    let nexus = crate::nexus_client::NexusClient::from_env();
    let record = if target.starts_with("http") {
        ProviderRecord {
            url: target.to_string(),
            provider_type: String::new(),
            model: String::new(),
        }
    } else if nexus.ensure_running().await.is_ok() {
        nexus.get("/api/inference/endpoints").await
            .ok()
            .and_then(|v| {
                v.get("endpoints")?.as_array()?
                    .iter()
                    .find(|p| p.get("id").and_then(|id| id.as_str()) == Some(target))
                    .map(|p| ProviderRecord {
                        url: p.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string(),
                        provider_type: p.get("provider").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        model: {
                            // models_json is a JSON-encoded array: parse first element
                            let raw = p.get("model").and_then(|v| v.as_str()).unwrap_or("[]");
                            serde_json::from_str::<Vec<String>>(raw)
                                .ok()
                                .and_then(|v| v.into_iter().next())
                                .unwrap_or_default()
                        },
                    })
            })
            .unwrap_or_else(|| ProviderRecord {
                url: format!("http://{}:11434", target),
                provider_type: String::new(),
                model: String::new(),
            })
    } else {
        ProviderRecord {
            url: format!("http://{}:11434", target),
            provider_type: String::new(),
            model: String::new(),
        }
    };

    let url = record.url.clone();

    // Tags probe uses a short timeout; inference probe uses a longer one
    // since large models (27B+) may need time to load from disk on first call.
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let http_infer = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;

    // ── OpenRouter / OpenAI-compatible calibration ────────────────────────
    if record.provider_type == "openrouter" || url.contains("openrouter.ai") {
        let api_key = std::env::var("OPENROUTER_API_KEY").ok()
            .filter(|k| !k.is_empty())
            .or_else(|| {
                // Try vault via nexus (best-effort)
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        nexus.get("/api/secrets/vault/OPENROUTER_API_KEY").await.ok()
                            .and_then(|v| v.get("value").and_then(|s| s.as_str()).map(|s| s.to_string()))
                            .filter(|s| !s.is_empty())
                    })
                })
            });

        let Some(api_key) = api_key else {
            println!("  {} OPENROUTER_API_KEY not set — cannot calibrate", "✗".red());
            println!("  Set it: hex secrets set OPENROUTER_API_KEY sk-or-...");
            return Ok(());
        };

        let model = if record.model.is_empty() { "openai/gpt-4o-mini".to_string() } else { record.model.clone() };
        println!("  {} Sending test inference to {} via {}...", "→".cyan(), model, url);

        let chat_url = format!("{}/chat/completions", url.trim_end_matches('/'));
        let test_body = serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": "Reply with only the word 'ok'."}],
            "max_tokens": 16,
        });

        let start = std::time::Instant::now();
        let result = http_infer
            .post(&chat_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&test_body)
            .send()
            .await;

        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(resp) if resp.status().is_success() => {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                let reply = body
                    .get("choices").and_then(|c| c.get(0))
                    .and_then(|c| c.get("message")).and_then(|m| m.get("content"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_lowercase();
                let reply_ok = !reply.is_empty();

                // Quality score: 0.7 baseline + latency bonus + response sanity bonus
                let latency_bonus: f32 = if latency_ms < 3_000 { 0.15 }
                    else if latency_ms < 8_000 { 0.08 }
                    else if latency_ms < 20_000 { 0.02 }
                    else { -0.05 };
                let sanity_bonus: f32 = if reply_ok { 0.15 } else { 0.0 };
                let quality_score = (0.70_f32 + latency_bonus + sanity_bonus).clamp(0.0, 1.0);

                println!("  {} {} responded in {}ms — reply: {:?}", "✓".green(), model, latency_ms, reply);
                println!("  {} quality_score = {:.2}  (latency bonus: {:+.2}, sanity: {:+.2})",
                    "ℹ".cyan(), quality_score, latency_bonus, sanity_bonus);

                // Write quality_score back to SpacetimeDB via nexus PATCH
                if nexus.ensure_running().await.is_ok() {
                    let patch_body = serde_json::json!({ "quality_score": quality_score });
                    match nexus.patch(&format!("/api/inference/endpoints/{}", target), &patch_body).await {
                        Ok(_) => println!("  {} Calibration saved — provider is now active in model router", "✓".green()),
                        Err(e) => println!("  {} Could not save calibration: {}", "!".yellow(), e),
                    }
                } else {
                    println!("  {} Nexus not running — calibration not saved", "!".yellow());
                }
            }
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                println!("  {} {} returned HTTP {} — {}", "!".yellow(), model, status,
                    body.chars().take(200).collect::<String>());
            }
            Err(e) => {
                println!("  {} Inference failed: {}", "✗".red(), e);
            }
        }
        return Ok(());
    }

    // Test Ollama /api/tags
    let ollama_url = format!("{}/api/tags", url.trim_end_matches('/'));
    println!("  {} GET {}", "→".cyan(), ollama_url);
    match http.get(&ollama_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            println!("  {} Ollama responding at {}", "✓".green(), url);
            // Collect local models sorted smallest-first so the probe uses the
            // quickest-to-load model rather than the largest one.
            let mut local_models: Vec<(u64, String)> = Vec::new();
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                if let Some(models) = body.get("models").and_then(|m| m.as_array()) {
                    println!("  {} {} model(s) available:", "ℹ".cyan(), models.len());
                    for m in models {
                        let name = m.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                        let size = m.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
                        let gb = size as f64 / 1_073_741_824.0;
                        let is_local = m.get("remote_model").is_none() && size > 0;
                        if is_local {
                            local_models.push((size, name.to_string()));
                        }
                        println!("    - {} ({:.1}GB){}", name, gb,
                            if !is_local { " [cloud]" } else { "" });
                    }
                }
            }
            local_models.sort_by_key(|(size, _)| *size);
            let test_model_opt = local_models.into_iter().next().map(|(_, n)| n);

            // Quick inference test using smallest available local model
            if let Some(ref test_model) = test_model_opt {
                println!();
                println!("  {} Running inference test with {}...", "→".cyan(), test_model);
                let chat_url = format!("{}/api/chat", url.trim_end_matches('/'));
                let test_body = serde_json::json!({
                    "model": test_model,
                    "messages": [{"role": "user", "content": "Reply with just the word 'ok'"}],
                    "stream": false,
                });

                let start = std::time::Instant::now();
                match http_infer.post(&chat_url).json(&test_body).send().await {
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

async fn discover_ollama(prune: bool) -> anyhow::Result<()> {
    println!("{}", "── Discovering Inference Providers ──".cyan());
    println!();

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;

    let mut found = 0;

    // ── 1. Query SpacetimeDB via nexus (source of truth) ──────────
    let client = NexusClient::from_env();
    let mut registered_urls: Vec<String> = Vec::new();
    let mut registered_ids: Vec<String> = Vec::new();

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
                        registered_ids.push(id.to_string());
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

        // ── Prune: remove providers that return empty responses ────
        if prune && !registered_ids.is_empty() {
            println!("{}", "── Pruning unhealthy providers ──".cyan());
            for pid in &registered_ids {
                let empty = match client.get(&format!("/api/inference/test/{}", pid)).await {
                    Ok(resp) => resp.get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .is_empty(),
                    Err(_) => true,
                };
                if empty {
                    let _ = client.delete(&format!("/api/inference/providers/{}", pid)).await;
                    println!("  {} Removed {} (empty response)", "✗".red(), pid);
                } else {
                    println!("  {} {} OK", "✓".green(), pid);
                }
            }
            println!();
        }
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

async fn discover_openrouter(filter: Option<&str>, min_context: Option<u64>) -> anyhow::Result<()> {
    println!("{}", "── Discovering OpenRouter Models ──".cyan());
    println!();

    // Check for API key
    let api_key = match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            // Try hex secrets vault
            let client = NexusClient::from_env();
            if let Ok(()) = client.ensure_running().await {
                match client.get("/api/secrets/vault/OPENROUTER_API_KEY").await {
                    Ok(data) => data.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    Err(_) => String::new(),
                }
            } else {
                String::new()
            }
        }
    };

    if api_key.is_empty() {
        println!("  {} OPENROUTER_API_KEY not set.", "✗".red());
        println!("  Set it with: hex secrets set OPENROUTER_API_KEY sk-or-...");
        println!("  Or export:   export OPENROUTER_API_KEY=sk-or-...");
        return Ok(());
    }

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    println!("  {} Fetching models from openrouter.ai...", "→".cyan());

    let resp = http
        .get("https://openrouter.ai/api/v1/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await?;

    if !resp.status().is_success() {
        println!("  {} OpenRouter returned HTTP {}", "✗".red(), resp.status());
        return Ok(());
    }

    let body: serde_json::Value = resp.json().await?;
    let models = body.get("data").and_then(|d| d.as_array());

    let Some(models) = models else {
        println!("  {} No models found in response", "!".yellow());
        return Ok(());
    };

    let min_ctx = min_context.unwrap_or(0);
    let mut count = 0;
    let mut registered = 0;

    let client = NexusClient::from_env();
    let nexus_running = client.ensure_running().await.is_ok();

    for model in models {
        let id = model.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let name = model.get("name").and_then(|v| v.as_str()).unwrap_or(id);
        let context_length = model.get("context_length").and_then(|v| v.as_u64()).unwrap_or(0);

        // Apply filters
        if let Some(f) = filter {
            if !id.to_lowercase().contains(&f.to_lowercase()) && !name.to_lowercase().contains(&f.to_lowercase()) {
                continue;
            }
        }
        if context_length < min_ctx {
            continue;
        }

        // Check if model supports tools (function calling)
        let supported_params = model.get("supported_parameters")
            .and_then(|v| v.as_array());
        let supports_tools = supported_params
            .map(|params| params.iter().any(|p| p.as_str() == Some("tools")))
            .unwrap_or(false);

        // Get pricing
        let pricing = model.get("pricing");
        let prompt_price = pricing
            .and_then(|p| p.get("prompt"))
            .and_then(|v| v.as_str())
            .unwrap_or("0");
        let completion_price = pricing
            .and_then(|p| p.get("completion"))
            .and_then(|v| v.as_str())
            .unwrap_or("0");

        let tool_badge = if supports_tools { " [tools]" } else { "" };
        println!(
            "  {} {} — {}K ctx, ${}/{} per M tok{}",
            "●".green(),
            id,
            context_length / 1000,
            prompt_price,
            completion_price,
            tool_badge,
        );

        // Register with nexus if running
        if nexus_running {
            let reg_body = serde_json::json!({
                "id": format!("openrouter-{}", id.replace('/', "-")),
                "provider": "openrouter",
                "url": "https://openrouter.ai/api/v1",
                "model": id,
                "models_json": serde_json::to_string(&vec![id]).unwrap_or_default(),
                "requires_auth": true,
                "secret_key": "OPENROUTER_API_KEY",
                "context_window": context_length as u32,
            });

            if client.post("/api/inference/register", &reg_body).await.is_ok() {
                registered += 1;
            } // Silent — don't spam on registration failures
        }

        count += 1;
    }

    println!();
    println!("{} {} models found, {} registered with nexus.", "✓".green(), count, registered);

    if !nexus_running {
        println!("  {} hex-nexus not running — models listed but not registered", "!".yellow());
        println!("  Start nexus: hex nexus start");
    }

    Ok(())
}

async fn remove_provider(provider_id: &str) -> anyhow::Result<()> {
    let client = NexusClient::from_env();
    client.ensure_running().await?;

    match client.delete(
        &format!("/api/inference/endpoints/{}", provider_id),
    ).await {
        Ok(_) => println!("{} Removed provider: {}", "✓".green(), provider_id),
        Err(e) => println!("{} Failed to remove: {}", "✗".red(), e),
    }

    Ok(())
}

/// Key default models: one per task type, matching model_selection.rs defaults.
/// ID format mirrors discover_openrouter: "openrouter-" + model.replace('/', "-").
/// Note: only use model IDs confirmed available on OpenRouter (no `:free` suffix
/// unless the model explicitly has a free variant — e.g. qwen3-coder:free exists,
/// but llama-4-maverick:free and deepseek-r1:free do not).
const DEFAULT_MODELS: &[(&str, &str)] = &[
    ("qwen/qwen3-coder:free",      "code generation + editing"),
    ("deepseek/deepseek-r1",       "reasoning + planning"),
    ("openai/gpt-4o-mini",         "structured output"),
    ("meta-llama/llama-4-maverick","general purpose"),
];

async fn setup_defaults() -> anyhow::Result<()> {
    println!("{}", "── Inference Setup ──".cyan());
    println!();

    // Require OpenRouter API key
    let api_key = std::env::var("OPENROUTER_API_KEY").ok()
        .filter(|k| !k.is_empty())
        .or_else(|| {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    NexusClient::from_env().get("/api/secrets/vault/OPENROUTER_API_KEY").await.ok()
                        .and_then(|v| v.get("value").and_then(|s| s.as_str()).map(|s| s.to_string()))
                        .filter(|s| !s.is_empty())
                })
            })
        });

    let Some(api_key) = api_key else {
        println!("  {} OPENROUTER_API_KEY not set — skipping inference setup.", "!".yellow());
        println!("  Set it first:  hex secrets set OPENROUTER_API_KEY sk-or-...");
        println!("  Then re-run:   hex inference setup");
        return Ok(());
    };

    let client = NexusClient::from_env();
    let nexus_running = client.ensure_running().await.is_ok();
    if !nexus_running {
        println!("  {} hex-nexus not running — start it first: hex nexus start", "✗".red());
        return Ok(());
    }

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;

    let or_url = "https://openrouter.ai/api/v1";
    let mut calibrated = 0;

    for (model_id, purpose) in DEFAULT_MODELS {
        let provider_id = format!("openrouter-{}", model_id.replace('/', "-"));
        print!("  {} {} ({})... ", "→".cyan(), model_id, purpose);

        // Register if not already present
        let reg_body = serde_json::json!({
            "id": &provider_id,
            "provider": "openrouter",
            "url": or_url,
            "model": model_id,
            "models_json": serde_json::to_string(&vec![model_id]).unwrap_or_default(),
            "requires_auth": true,
            "secret_key": "OPENROUTER_API_KEY",
            "quantization": "cloud",
        });
        let _ = client.post("/api/inference/register", &reg_body).await;

        // Calibrate via test inference
        let chat_url = format!("{}/chat/completions", or_url);
        let test_body = serde_json::json!({
            "model": model_id,
            "messages": [{"role": "user", "content": "Reply with only the word 'ok'."}],
            "max_tokens": 16,
        });

        let start = std::time::Instant::now();
        let result = http.post(&chat_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&test_body)
            .send()
            .await;

        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(resp) if resp.status().is_success() => {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                let reply_ok = body
                    .get("choices").and_then(|c| c.get(0))
                    .and_then(|c| c.get("message")).and_then(|m| m.get("content"))
                    .and_then(|v| v.as_str())
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false);

                let latency_bonus: f32 = if latency_ms < 3_000 { 0.15 }
                    else if latency_ms < 8_000 { 0.08 }
                    else if latency_ms < 20_000 { 0.02 }
                    else { -0.05 };
                let quality_score = (0.70_f32 + latency_bonus + if reply_ok { 0.15 } else { 0.0 }).clamp(0.0, 1.0);

                let patch = serde_json::json!({ "quality_score": quality_score });
                match client.patch(&format!("/api/inference/endpoints/{}", provider_id), &patch).await {
                    Ok(_) => println!("{} q={:.2} ({}ms)", "✓".green(), quality_score, latency_ms),
                    Err(e) => {
                        println!("{} inference ok but calibration save failed: {}", "!".yellow(), e);
                        continue;
                    }
                }
                calibrated += 1;
            }
            Ok(resp) if resp.status().as_u16() == 429 => {
                // Rate limited — wait 5s and retry once
                print!("rate limited, retrying in 5s... ");
                let _ = std::io::Write::flush(&mut std::io::stdout());
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                let start2 = std::time::Instant::now();
                match http.post(&chat_url)
                    .header("Authorization", format!("Bearer {}", api_key))
                    .header("Content-Type", "application/json")
                    .json(&test_body)
                    .send()
                    .await
                {
                    Ok(r) if r.status().is_success() => {
                        let latency_ms2 = start2.elapsed().as_millis() as u64;
                        let latency_bonus: f32 = if latency_ms2 < 3_000 { 0.15 }
                            else if latency_ms2 < 8_000 { 0.08 }
                            else if latency_ms2 < 20_000 { 0.02 }
                            else { -0.05 };
                        let quality_score = (0.70_f32 + latency_bonus + 0.15).clamp(0.0, 1.0);
                        let patch = serde_json::json!({ "quality_score": quality_score });
                        match client.patch(&format!("/api/inference/endpoints/{}", provider_id), &patch).await {
                            Ok(_) => { println!("{} q={:.2} ({}ms)", "✓".green(), quality_score, latency_ms2); calibrated += 1; }
                            Err(e) => println!("{} save failed: {}", "!".yellow(), e),
                        }
                    }
                    _ => println!("{} still rate limited — run `hex inference test {}` later", "!".yellow(), provider_id),
                }
            }
            Ok(resp) => {
                println!("{} HTTP {}", "!".yellow(), resp.status());
            }
            Err(e) => {
                println!("{} {}", "✗".red(), e);
            }
        }
    }

    println!();
    if calibrated == DEFAULT_MODELS.len() {
        println!("{} All {} models calibrated — run `hex nexus status` to verify.", "✓".green(), calibrated);
    } else {
        println!("{} {}/{} models calibrated.", "!".yellow(), calibrated, DEFAULT_MODELS.len());
    }

    Ok(())
}

// ── hex inference watch ────────────────────────────────────────────────────

/// InferenceTaskPush mirrors the server-side struct in hex-nexus/src/state.rs.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct InferenceTaskPush {
    id: String,
    workplan_id: String,
    task_id: String,
    phase: String,
    prompt: String,
    role: String,
}

/// Connect to /ws/inference and dispatch incoming tasks autonomously.
///
/// Each message received is an InferenceTaskPush. We:
///   1. Claim the task via PATCH /api/inference/queue/{id} {"status":"claimed"}
///   2. Spawn a tokio task that calls `claude --dangerously-skip-permissions -p <prompt>`
///   3. Report result/failure back via PATCH /api/inference/queue/{id}
///
/// The loop reconnects on disconnect (5-second backoff).
async fn watch(agent_id_opt: Option<String>, daemon: bool) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let agent_id = agent_id_opt
        .or_else(crate::nexus_client::read_session_agent_id)
        .unwrap_or_else(|| "unknown".to_string());

    if !daemon {
        let short_id = &agent_id[..8.min(agent_id.len())];
        println!("{} inference-watch: connecting (agent {})", "⬡".cyan(), short_id);
    }

    let base_url = nexus.url().to_string();
    let ws_url = base_url
        .replace("http://", "ws://")
        .replace("https://", "wss://");
    let ws_url = format!("{}/ws/inference", ws_url);

    loop {
        match connect_and_watch(&ws_url, &agent_id, &base_url, daemon).await {
            Ok(()) => break,
            Err(e) => {
                if !daemon {
                    eprintln!("{} inference-watch: reconnecting ({})...", "⬡".yellow(), e);
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        }
    }
    Ok(())
}

async fn connect_and_watch(
    ws_url: &str,
    agent_id: &str,
    nexus_base: &str,
    daemon: bool,
) -> anyhow::Result<()> {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;

    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url).await?;

    // Startup reconciliation: fetch any Pending tasks that were enqueued
    // before this watch process connected (missed broadcast events).
    let http = reqwest::Client::new();
    let pending_url = format!("{}/api/inference/queue/pending", nexus_base);
    if let Ok(resp) = http.get(&pending_url).send().await {
        if let Ok(tasks) = resp.json::<Vec<InferenceTaskPush>>().await {
            for task in tasks {
                let push_id = task.id.clone();
                let agent_id_owned = agent_id.to_string();
                let nexus_base_owned = nexus_base.to_string();
                let claim_url = format!("{}/api/inference/queue/{}", nexus_base, push_id);
                let claim_resp = http
                    .patch(&claim_url)
                    .header("X-Hex-Agent-Id", &agent_id_owned)
                    .json(&serde_json::json!({ "status": "claimed" }))
                    .send()
                    .await;
                if claim_resp.and_then(|r| Ok(r.status().is_success())).unwrap_or(false) {
                    if !daemon {
                        println!("{} inference-watch: claimed (startup) {}", "⬡".green(), push_id);
                    }
                    tokio::spawn(async move {
                        dispatch_inference_task(task, agent_id_owned, nexus_base_owned).await;
                    });
                }
            }
        }
    }

    while let Some(msg) = ws.next().await {
        match msg? {
            Message::Text(text) => {
                if let Ok(push) = serde_json::from_str::<InferenceTaskPush>(&text) {
                    if !daemon {
                        println!(
                            "{} inference-watch: dispatching {}/{}",
                            "⬡".cyan(),
                            push.workplan_id,
                            push.task_id
                        );
                    }

                    // Claim the task (CAS — first agent to patch wins).
                    let claim_url = format!("{}/api/inference/queue/{}", nexus_base, push.id);
                    let http = reqwest::Client::new();
                    let claim_resp = http
                        .patch(&claim_url)
                        .header("X-Hex-Agent-Id", agent_id)
                        .json(&serde_json::json!({ "status": "claimed" }))
                        .send()
                        .await;

                    let claimed = claim_resp
                        .and_then(|r| Ok(r.status().is_success()))
                        .unwrap_or(false);

                    if claimed {
                        let push_id = push.id.clone();
                        let agent_id_owned = agent_id.to_string();
                        let nexus_base_owned = nexus_base.to_string();
                        tokio::spawn(async move {
                            dispatch_inference_task(push, agent_id_owned, nexus_base_owned).await;
                        });
                        if !daemon {
                            println!("{} inference-watch: claimed {}", "⬡".green(), push_id);
                        }
                    } else if !daemon {
                        println!("{} inference-watch: claim lost for {} (another agent won)", "⬡".yellow(), push.id);
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    Ok(())
}

async fn dispatch_inference_task(push: InferenceTaskPush, agent_id: String, nexus_base: String) {
    let prompt = format!("HEXFLO_TASK:{}\n\n{}", push.task_id, push.prompt);

    let result = tokio::task::spawn_blocking(move || {
        std::process::Command::new("claude")
            .args(["--dangerously-skip-permissions", "-p", &prompt])
            .output()
    })
    .await;

    let http = reqwest::Client::new();

    let url = format!("{}/api/inference/queue/{}", nexus_base, push.id);
    let body = match result {
        Ok(Ok(out)) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            let snippet = text[..200.min(text.len())].to_string();
            serde_json::json!({ "status": "completed", "result": snippet })
        }
        Ok(Ok(out)) => {
            let text = String::from_utf8_lossy(&out.stderr).to_string();
            let snippet = text[..200.min(text.len())].to_string();
            serde_json::json!({ "status": "failed", "error": snippet })
        }
        Ok(Err(e)) => {
            serde_json::json!({ "status": "failed", "error": e.to_string() })
        }
        Err(e) => {
            serde_json::json!({ "status": "failed", "error": e.to_string() })
        }
    };
    let _ = http
        .patch(&url)
        .header("X-Hex-Agent-Id", &agent_id)
        .json(&body)
        .send()
        .await;
}

/// `hex inference queue` — list pending inference tasks from the nexus.
async fn queue_list() -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let resp = nexus.get("/api/inference/queue/pending").await?;
    let tasks = resp.as_array().cloned().unwrap_or_default();

    if tasks.is_empty() {
        println!("{} No pending inference tasks", "⬡".cyan());
        return Ok(());
    }

    println!("{} Pending inference tasks:", "⬡".cyan());
    for t in &tasks {
        let id = t["id"].as_str().unwrap_or("-");
        let wid = t["workplan_id"].as_str().unwrap_or("-");
        let tid = t["task_id"].as_str().unwrap_or("-");
        let status = t["status"].as_str().unwrap_or("-");
        let role = t["role"].as_str().unwrap_or("-");
        println!("  {} — {}/{} [{}] role={}", id, wid, tid, status, role);
    }
    Ok(())
}
