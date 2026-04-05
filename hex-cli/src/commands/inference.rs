//! `hex inference` — Manage inference providers (Ollama, vLLM, etc.)
//!
//! Register, list, and test self-hosted LLM endpoints.
//! Supports template-based registration for known free-tier providers (ADR-2604052125).
//!
//! Usage:
//!   hex inference add groq --key $GROQ_API_KEY          # Template-based (auto-registers all models)
//!   hex inference add cerebras --key $CEREBRAS_API_KEY   # Template-based
//!   hex inference add ollama http://bazzite.local:11434 --model qwen3:32b  # Manual
//!   hex inference add vllm http://gpu-server:8000 --model Qwen/Qwen3-32B  # Manual
//!   hex inference list
//!   hex inference test <provider-id>
//!   hex inference discover --free                       # Auto-discover all free-tier providers
//!   hex inference stats                                 # Cost attribution dashboard

use clap::Subcommand;
use colored::Colorize;

use crate::assets::Assets;
use crate::nexus_client::NexusClient;

/// Known free-tier provider template names (ADR-2604052125).
const PROVIDER_TEMPLATES: &[&str] = &["groq", "cerebras", "sambanova", "together", "openrouter", "ollama"];

/// Parsed provider template from YAML (ADR-2604052125).
#[derive(Debug, serde::Deserialize)]
struct ProviderTemplate {
    name: String,
    display_name: String,
    base_url: String,
    api_key_env: Option<String>,
    provider_type: String,
    #[serde(default)]
    is_free_tier: bool,
    #[serde(default)]
    rate_limits: ProviderRateLimits,
    #[serde(default)]
    cost: ProviderCost,
    #[serde(default)]
    models: Vec<ProviderModelEntry>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[allow(dead_code)]
struct ProviderRateLimits {
    #[serde(default)]
    rpm: u32,
    #[serde(default)]
    daily_requests: Option<u32>,
    #[serde(default)]
    daily_tokens: Option<u64>,
    #[serde(default)]
    tpm: u64,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ProviderCost {
    #[serde(default)]
    input_per_mtok: f64,
    #[serde(default)]
    output_per_mtok: f64,
}

#[derive(Debug, serde::Deserialize)]
struct ProviderModelEntry {
    id: String,
    #[serde(default = "default_tier")]
    tier: String,
    #[serde(default)]
    context_window: u32,
    #[serde(default)]
    coding_optimized: bool,
}

fn default_tier() -> String { "cloud".to_string() }

/// Load a provider template from embedded assets.
fn load_provider_template(name: &str) -> Option<ProviderTemplate> {
    let path = format!("inference-providers/{}.yml", name);
    let content = Assets::get_str(&path)?;
    serde_yaml::from_str(&content).ok()
}

#[derive(Subcommand)]
pub enum InferenceAction {
    /// Register a new inference provider (template name or manual type+URL)
    Add {
        /// Provider type or template name: groq, cerebras, sambanova, together, openrouter, ollama, vllm, openai-compat
        provider_type: String,
        /// Base URL (e.g., http://bazzite.local:11434). Optional for template providers.
        url: Option<String>,
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
    /// Test connectivity to a provider (or --all uncalibrated)
    Test {
        /// Provider ID, URL, or prefix. Use "openrouter" to test all OpenRouter providers.
        #[arg(required_unless_present = "calibrate_all")]
        target: Option<String>,
        /// Calibrate all uncalibrated providers
        #[arg(long = "all")]
        calibrate_all: bool,
    },
    /// Auto-discover inference providers
    Discover {
        /// Provider to discover: ollama (default, LAN scan), openrouter (fetch model catalog), free (all free-tier)
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
    /// Show inference cost attribution and provider statistics (ADR-2604052125)
    Stats,
}

pub async fn run(action: InferenceAction) -> anyhow::Result<()> {
    match action {
        InferenceAction::Add { provider_type, url, model, key, id, quantization } => {
            // Check if provider_type is a known template name (ADR-2604052125)
            if PROVIDER_TEMPLATES.contains(&provider_type.as_str()) && url.is_none() {
                add_from_template(&provider_type, key.as_deref(), id.as_deref()).await
            } else {
                let url = url.unwrap_or_else(|| {
                    eprintln!("{} URL required for non-template provider type '{}'", "✗".red(), provider_type);
                    std::process::exit(1);
                });
                add_provider(&provider_type, &url, model.as_deref(), key.as_deref(), id.as_deref(), quantization.as_deref()).await
            }
        }
        InferenceAction::List => list_providers().await,
        InferenceAction::Test { target, calibrate_all } => test_provider(target.as_deref(), calibrate_all).await,
        InferenceAction::Discover { provider, filter, min_context, prune } => {
            match provider.as_str() {
                "free" => discover_free_tier().await,
                "openrouter" => discover_openrouter(filter.as_deref(), min_context).await,
                _ => discover_ollama(prune).await,
            }
        }
        InferenceAction::Remove { provider_id } => remove_provider(&provider_id).await,
        InferenceAction::Setup => setup_defaults().await,
        InferenceAction::Watch { agent_id, daemon } => watch(agent_id, daemon).await,
        InferenceAction::Queue => queue_list().await,
        InferenceAction::Stats => inference_stats().await,
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
        "ollama" => "llama3", // placeholder — run `hex inference add ollama <url> --model <name>` with your actual model
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

/// Register a provider from a built-in template (ADR-2604052125).
///
/// Reads the YAML template from embedded assets, resolves the API key from
/// --key flag or environment variable, and registers all models with correct
/// base URL, rate limits, and quantization tier.
async fn add_from_template(
    template_name: &str,
    key: Option<&str>,
    custom_id: Option<&str>,
) -> anyhow::Result<()> {
    let template = match load_provider_template(template_name) {
        Some(t) => t,
        None => {
            println!("{} Unknown provider template '{}'. Available: {}", "✗".red(), template_name,
                PROVIDER_TEMPLATES.join(", "));
            return Ok(());
        }
    };

    println!("{}", format!("── Registering {} ({}) ──", template.display_name, template.name).cyan());
    println!("  Base URL: {}", template.base_url);
    println!("  Free tier: {}", if template.is_free_tier { "yes".green() } else { "no".yellow() });
    if template.rate_limits.rpm > 0 {
        println!("  Rate limits: {} RPM, {} TPM", template.rate_limits.rpm, template.rate_limits.tpm);
    }
    if let Some(daily) = template.rate_limits.daily_tokens {
        println!("  Daily quota: {} tokens", daily);
    }

    // Resolve API key: --key flag > env var > abort
    let api_key = if let Some(k) = key {
        k.to_string()
    } else if let Some(ref env_var) = template.api_key_env {
        match std::env::var(env_var) {
            Ok(k) if !k.is_empty() => {
                println!("  {} API key loaded from {}", "✓".green(), env_var);
                k
            }
            _ => {
                if template.name == "ollama" {
                    String::new() // Ollama doesn't need a key
                } else {
                    println!("  {} No API key provided. Set {} or use --key", "✗".red(),
                        env_var);
                    return Ok(());
                }
            }
        }
    } else {
        String::new()
    };

    let provider_id = custom_id.unwrap_or(template_name);
    let model_ids: Vec<String> = template.models.iter().map(|m| m.id.clone()).collect();
    let models_json = serde_json::to_string(&model_ids).unwrap_or_else(|_| "[]".to_string());
    let quantization = template.models.first()
        .map(|m| m.tier.clone())
        .unwrap_or_else(|| "cloud".to_string());

    println!("  {} model(s):", template.models.len());
    for m in &template.models {
        println!("    - {} [ctx: {}] {}", m.id, m.context_window,
            if m.coding_optimized { "(code-optimized)".green() } else { "".normal() });
    }

    // Register with nexus
    let client = NexusClient::from_env();
    if client.ensure_running().await.is_ok() {
        let body = serde_json::json!({
            "id": provider_id,
            "provider": template.provider_type,
            "url": template.base_url.trim_end_matches('/'),
            "model": model_ids.first().unwrap_or(&"default".to_string()),
            "models_json": models_json,
            "requires_auth": !api_key.is_empty(),
            "secret_key": api_key,
            "quantization": quantization,
            "rate_limit_rpm": template.rate_limits.rpm,
            "rate_limit_tpm": template.rate_limits.tpm,
            "is_free_tier": template.is_free_tier,
            "cost_per_input_mtok": template.cost.input_per_mtok,
            "cost_per_output_mtok": template.cost.output_per_mtok,
        });
        match client.post("/api/inference/register", &body).await {
            Ok(_) => println!("  {} Registered with hex-nexus", "✓".green()),
            Err(e) => println!("  {} Nexus registration failed: {}", "!".yellow(), e),
        }
    } else {
        println!("  {} hex-nexus not running — provider saved locally only", "!".yellow());
    }

    println!();
    println!("{} {} registered with {} model(s)", "✓".green(), template.display_name, template.models.len());
    println!();
    if template.is_free_tier {
        println!("  Cost: {} (free tier)", "$0.00".green());
    }

    Ok(())
}

/// Discover all free-tier providers by checking env vars (ADR-2604052125).
///
/// Probes known free-tier providers (Groq, Cerebras, SambaNova, Together, OpenRouter)
/// for API keys in environment variables and registers all discovered providers.
async fn discover_free_tier() -> anyhow::Result<()> {
    println!("{}", "── Discovering Free-Tier Inference Providers (ADR-2604052125) ──".cyan());
    println!();

    let mut discovered = 0u32;
    let mut total_daily_tokens: u64 = 0;

    for template_name in PROVIDER_TEMPLATES {
        let template = match load_provider_template(template_name) {
            Some(t) => t,
            None => continue,
        };
        if !template.is_free_tier {
            continue;
        }

        // Check for API key (Ollama doesn't need one)
        let has_key = if template.name == "ollama" {
            // Check if Ollama is reachable
            let http = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(3))
                .build()?;
            let url = format!("{}/api/tags", template.base_url.trim_end_matches("/v1").trim_end_matches('/'));
            http.get(&url).send().await.is_ok()
        } else if let Some(ref env_var) = template.api_key_env {
            std::env::var(env_var).map(|v| !v.is_empty()).unwrap_or(false)
        } else {
            false
        };

        let status_icon = if has_key { "✓".green() } else { "○".yellow() };
        let key_source = if template.name == "ollama" {
            if has_key { "reachable" } else { "not running" }
        } else if has_key {
            "API key found"
        } else {
            "no API key"
        };

        println!("  {} {} — {} ({} models)", status_icon, template.display_name,
            key_source, template.models.len());

        if has_key {
            if let Some(daily) = template.rate_limits.daily_tokens {
                total_daily_tokens += daily;
                println!("    Daily quota: {} tokens", daily);
            }
            if template.rate_limits.rpm > 0 {
                println!("    Rate limit: {} RPM", template.rate_limits.rpm);
            }
            // Auto-register
            if let Err(e) = add_from_template(template_name, None, None).await {
                println!("    {} Registration failed: {}", "!".yellow(), e);
            }
            discovered += 1;
        } else if let Some(ref env_var) = template.api_key_env {
            println!("    Set: export {}=<your-key>", env_var);
        }
    }

    println!();
    if discovered > 0 {
        println!("{} Discovered {} free-tier provider(s)", "✓".green(), discovered);
        if total_daily_tokens > 0 {
            println!("  Combined daily quota: ~{}M tokens", total_daily_tokens / 1_000_000);
        }
        println!("  Run 'hex inference test --all' to calibrate quality scores.");
    } else {
        println!("{} No free-tier providers discovered.", "!".yellow());
        println!("  Set API keys for: GROQ_API_KEY, CEREBRAS_API_KEY, SAMBANOVA_API_KEY,");
        println!("  TOGETHER_API_KEY, OPENROUTER_API_KEY");
        println!("  Or start Ollama locally: ollama serve");
    }

    Ok(())
}

/// Show inference cost attribution and provider statistics (ADR-2604052125).
async fn inference_stats() -> anyhow::Result<()> {
    let client = NexusClient::from_env();
    println!("{}", "── Inference Cost Attribution (ADR-2604052125) ──".cyan());
    println!();

    if client.ensure_running().await.is_err() {
        println!("{} hex-nexus not running — cannot fetch stats", "✗".red());
        return Ok(());
    }

    // Fetch provider stats from nexus
    match client.get("/api/inference/stats").await {
        Ok(data) => {
            // Provider distribution
            if let Some(providers) = data.get("providers").and_then(|v| v.as_array()) {
                println!("{}", "  Provider Distribution:".cyan());
                for p in providers {
                    let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let requests = p.get("requests").and_then(|v| v.as_u64()).unwrap_or(0);
                    let tokens = p.get("tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let cost = p.get("cost_usd").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let is_free = p.get("is_free_tier").and_then(|v| v.as_bool()).unwrap_or(false);
                    let cost_str = if is_free {
                        "$0.00 (free)".to_string().green().to_string()
                    } else {
                        format!("${:.4}", cost).to_string()
                    };
                    println!("    {} — {} requests, {}K tokens, {}", name, requests,
                        tokens / 1000, cost_str);
                }
            }
            // Cost summary
            if let Some(summary) = data.get("summary") {
                println!();
                let actual = summary.get("actual_cost_usd").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let counterfactual = summary.get("counterfactual_cost_usd").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let savings_pct = if counterfactual > 0.0 {
                    (1.0 - actual / counterfactual) * 100.0
                } else {
                    0.0
                };
                println!("{}", "  Cost Summary:".cyan());
                println!("    Actual cost:        ${:.4}", actual);
                println!("    Frontier equivalent: ${:.4}", counterfactual);
                println!("    Savings:            {:.1}%", savings_pct);
            }
        }
        Err(_) => {
            println!("  Stats endpoint not available. Ensure hex-nexus is updated.");
            println!("  Stats are collected per-session and reset on nexus restart.");
        }
    }

    // Show free tier utilization
    println!();
    println!("{}", "  Free Tier Utilization:".cyan());
    match client.get("/api/inference/rate-state").await {
        Ok(data) => {
            if let Some(providers) = data.get("providers").and_then(|v| v.as_array()) {
                for p in providers {
                    let name = p.get("provider_id").and_then(|v| v.as_str()).unwrap_or("?");
                    let rpm_used = p.get("requests_this_minute").and_then(|v| v.as_u64()).unwrap_or(0);
                    let rpm_limit = p.get("rpm_limit").and_then(|v| v.as_u64()).unwrap_or(0);
                    let daily_used = p.get("tokens_today").and_then(|v| v.as_u64()).unwrap_or(0);
                    let daily_limit = p.get("daily_token_limit").and_then(|v| v.as_u64());
                    let circuit = p.get("circuit_state").and_then(|v| v.as_str()).unwrap_or("closed");

                    let circuit_icon = match circuit {
                        "open" => "⊘".red(),
                        "half_open" => "◐".yellow(),
                        _ => "●".green(),
                    };

                    print!("    {} {} — {}/{} RPM", circuit_icon, name, rpm_used, rpm_limit);
                    if let Some(limit) = daily_limit {
                        let pct = if limit > 0 { daily_used as f64 / limit as f64 * 100.0 } else { 0.0 };
                        print!(", {}K/{}K daily ({:.0}%)", daily_used / 1000, limit / 1000, pct);
                    }
                    println!();
                }
            } else {
                println!("    No rate state data available.");
            }
        }
        Err(_) => {
            println!("    Rate state not available (nexus may need update).");
        }
    }

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

async fn test_provider(target: Option<&str>, all: bool) -> anyhow::Result<()> {
    let nexus = crate::nexus_client::NexusClient::from_env();

    // ── --all: calibrate every uncalibrated provider ────────────────────────
    if all {
        if nexus.ensure_running().await.is_err() {
            println!("{} hex-nexus not running — cannot list providers", "✗".red());
            return Ok(());
        }
        let endpoints = match nexus.get("/api/inference/endpoints").await {
            Ok(v) => v.get("endpoints").and_then(|e| e.as_array()).cloned(),
            Err(e) => {
                println!("{} Failed to fetch providers: {}", "✗".red(), e);
                return Ok(());
            }
        };
        let Some(endpoints) = endpoints else {
            println!("{} No providers registered", "!".yellow());
            return Ok(());
        };

        let uncalibrated: Vec<_> = endpoints.iter()
            .filter(|p| p.get("qualityScore").is_none() || p.get("qualityScore").map(|v| v.is_number()).unwrap_or(false))
            .collect();

        if uncalibrated.is_empty() {
            println!("{} All providers already calibrated", "✓".green());
            return Ok(());
        }

        println!("{} Found {} uncalibrated provider(s)", "→".cyan(), uncalibrated.len());
        println!();

        for p in &uncalibrated {
            let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            let ptype = p.get("provider").and_then(|v| v.as_str()).unwrap_or("");
            let url_val = p.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let mdl = {
                let raw = p.get("model").and_then(|v| v.as_str()).unwrap_or("[]");
                serde_json::from_str::<Vec<String>>(raw).ok()
                    .and_then(|v| v.into_iter().next())
                    .unwrap_or_default()
            };
            println!("{}", format!("── Calibrating {} ({}) ──", id, ptype).cyan());
            test_single_provider(id, url_val, ptype, &mdl).await?;
            println!();
        }
        return Ok(());
    }

    let Some(target) = target else {
        println!("{} Specify a target or use --all", "!".yellow());
        println!("  hex inference test openrouter   # test all OpenRouter providers");
        println!("  hex inference test ollama       # test Ollama at localhost:11434");
        println!("  hex inference test --all        # calibrate all uncalibrated providers");
        return Ok(());
    };

    println!("{} Testing {}...", "→".cyan(), target);

    // Look up provider record by exact ID, prefix match, or URL.
    struct ProviderRecord {
        id: String,
        url: String,
        provider_type: String,
        model: String,
    }

    let record = if target.starts_with("http") {
        // Direct URL — infer provider type from URL pattern
        let ptype = if target.contains("openrouter.ai") {
            "openrouter"
        } else if target.contains("ollama") || target.contains(":11434") {
            "ollama"
        } else {
            "openai-compat"
        };
        Some(ProviderRecord {
            id: target.to_string(),
            url: target.to_string(),
            provider_type: ptype.to_string(),
            model: String::new(),
        })
    } else if nexus.ensure_running().await.is_ok() {
        let endpoints = nexus.get("/api/inference/endpoints").await
            .ok()
            .and_then(|v| v.get("endpoints").and_then(|e| e.as_array()).cloned());

        let matches: Vec<_> = endpoints
            .into_iter()
            .flatten()
            .filter(|p| {
                let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("");
                // Exact match or prefix match (e.g. "openrouter" matches "openrouter-meta-llama-*")
                id == target || id.starts_with(&format!("{}-", target))
            })
            .collect();

        if matches.len() > 1 {
            println!("{} {} provider(s) match '{}' — calibrating all:", "→".cyan(), matches.len(), target);
            for p in &matches {
                let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let ptype = p.get("provider").and_then(|v| v.as_str()).unwrap_or("");
                let url_val = p.get("url").and_then(|v| v.as_str()).unwrap_or("");
                let mdl = {
                    let raw = p.get("model").and_then(|v| v.as_str()).unwrap_or("[]");
                    serde_json::from_str::<Vec<String>>(raw).ok()
                        .and_then(|v| v.into_iter().next())
                        .unwrap_or_default()
                };
                println!("  • {} ({})", id, ptype);
                test_single_provider(id, url_val, ptype, &mdl).await?;
            }
            return Ok(());
        }

        matches.into_iter().next().map(|p| {
            let raw = p.get("model").and_then(|v| v.as_str()).unwrap_or("[]");
            let model = serde_json::from_str::<Vec<String>>(raw)
                .ok()
                .and_then(|v| v.into_iter().next())
                .unwrap_or_default();
            ProviderRecord {
                id: p.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                url: p.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                provider_type: p.get("provider").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                model,
            }
        })
    } else {
        None
    };

    let Some(record) = record else {
        println!("  {} No provider found for '{}' — trying as direct URL", "!".yellow(), target);
        let ptype = if target.contains("openrouter") { "openrouter" } else { "ollama" };
        test_single_provider(target, target, ptype, "").await?;
        return Ok(());
    };

    test_single_provider(&record.id, &record.url, &record.provider_type, &record.model).await
}

async fn test_single_provider(id: &str, url: &str, provider_type: &str, model_name: &str) -> anyhow::Result<()> {
    let nexus = crate::nexus_client::NexusClient::from_env();
    let http_infer = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    // ── OpenRouter / OpenAI-compatible calibration ────────────────────────
    if provider_type == "openrouter" || (url.contains("openrouter.ai") && !url.contains(":11434")) {
        let api_key = std::env::var("OPENROUTER_API_KEY").ok()
            .filter(|k| !k.is_empty())
            .or_else(|| {
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

        let model = if !model_name.is_empty() {
            model_name.to_string()
        } else {
            "openai/gpt-4o-mini".to_string()
        };
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

                let latency_bonus: f32 = if latency_ms < 3_000 { 0.15 }
                    else if latency_ms < 8_000 { 0.08 }
                    else if latency_ms < 20_000 { 0.02 }
                    else { -0.05 };
                let sanity_bonus: f32 = if reply_ok { 0.15 } else { 0.0 };
                let quality_score = (0.70_f32 + latency_bonus + sanity_bonus).clamp(0.0, 1.0);

                println!("  {} {} responded in {}ms — reply: {:?}", "✓".green(), model, latency_ms, reply);
                println!("  {} quality_score = {:.2}  (latency: {:+.2}, sanity: {:+.2})",
                    "ℹ".cyan(), quality_score, latency_bonus, sanity_bonus);

                if nexus.ensure_running().await.is_ok() {
                    let patch_body = serde_json::json!({ "quality_score": quality_score });
                    match nexus.patch(&format!("/api/inference/endpoints/{}", id), &patch_body).await {
                        Ok(_) => println!("  {} Calibration saved — active in model router", "✓".green()),
                        Err(e) => println!("  {} Could not save calibration: {}", "!".yellow(), e),
                    }
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

    // ── Ollama calibration ─────────────────────────────────────────────────
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let ollama_url = format!("{}/api/tags", url.trim_end_matches('/'));
    println!("  {} GET {}", "→".cyan(), ollama_url);    match http.get(&ollama_url).send().await {
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
        match client.get("/api/inference/endpoints").await {
                Ok(providers) => {
                    if let Some(arr) = providers.get("endpoints").and_then(|e| e.as_array()) {
                        for p in arr {
                            let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                            let ptype = p.get("provider").and_then(|v| v.as_str()).unwrap_or("?");
                            let url = p.get("url").and_then(|v| v.as_str()).unwrap_or("?");

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

        // ── Prune: remove providers that are unreachable ────
        if prune && !registered_ids.is_empty() {
            println!("{}", "── Pruning unhealthy providers ──".cyan());
            match client.get("/api/inference/endpoints").await {
                Ok(providers) => {
                    if let Some(arr) = providers.get("endpoints").and_then(|e| e.as_array()) {
                        for p in arr {
                            let pid = p.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                            let ptype = p.get("provider").and_then(|v| v.as_str()).unwrap_or("");
                            let url_val = p.get("url").and_then(|v| v.as_str()).unwrap_or("?");
                            let reachable = if ptype == "ollama" {
                                http.get(format!("{}/api/tags", url_val.trim_end_matches('/')))
                                    .send().await.map(|r| r.status().is_success()).unwrap_or(false)
                            } else {
                                http.get(format!("{}/v1/models", url_val.trim_end_matches('/')))
                                    .send().await.map(|r| r.status().is_success()).unwrap_or(false)
                            };
                            if !reachable {
                                let _ = client.delete(&format!("/api/inference/endpoints/{}", pid)).await;
                                println!("  {} Removed {} (unreachable)", "✗".red(), pid);
                            } else {
                                println!("  {} {} OK", "✓".green(), pid);
                            }
                        }
                    }
                }
                Err(e) => println!("  {} Could not fetch providers: {}", "!".yellow(), e),
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
                if claim_resp.map(|r| r.status().is_success()).unwrap_or(false) {
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

                    let claimed = claim_resp.map(|r| r.status().is_success())
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
