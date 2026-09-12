//! `hex inference` — Manage inference providers (Ollama, vLLM, etc.)
//!
//! Register, list, and test self-hosted LLM endpoints.
//! Supports template-based registration for known free-tier providers (ADR-2026-04-05-2125).
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

/// Known free-tier provider template names (ADR-2026-04-05-2125).
const PROVIDER_TEMPLATES: &[&str] = &["groq", "cerebras", "sambanova", "together", "openrouter", "ollama", "gemini"];

/// Parsed provider template from YAML (ADR-2026-04-05-2125).
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
        /// Provider type or template name: groq, cerebras, sambanova, together, openrouter, ollama, gemini, vllm, openai-compat
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
    /// Benchmark a model: code-gen, reasoning, and identity prompts — quality + speed + tier recommendation (ADR-2026-04-13-1238)
    Bench {
        /// Provider ID, model name, or URL (e.g. "bazzite-ollama", "minimax-m2.7:cloud", "http://bazzite:11434")
        target: String,
        /// Specific model to benchmark (overrides the provider's registered model)
        #[arg(long)]
        model: Option<String>,
        /// Skip the long code-generation prompt (identity + reasoning only)
        #[arg(long)]
        quick: bool,
        /// Run same prompts against a baseline model for side-by-side comparison
        #[arg(long)]
        compare: Option<String>,
        /// Persist quality score and tier recommendation to nexus
        #[arg(long)]
        save: bool,
    },
    /// Verify Ollama GPU inference is working (WP P1-4)
    GpuCheck {
        /// Model to run for the check (default: qwen3:4b — small, fast)
        #[arg(long, default_value = "qwen3:4b")]
        model: String,
        /// Prompt text for the streaming test
        #[arg(long, default_value = "What is hex?")]
        prompt: String,
    },
}

/// LoRA adapter registry subcommands (ADR-2606161300 Phase 1).
#[derive(Subcommand)]
enum AdapterAction {
    /// Register a trained LoRA adapter against a (base, tier, expert) tuple
    Register {
        /// Expert this adapter realizes (e.g. hex-boundaries)
        #[arg(long)]
        expert: String,
        /// Frozen base model the adapter rides on (e.g. qwen2.5-coder:32b)
        #[arg(long)]
        base: String,
        /// Tier served (1, 2, or 25 for T2.5)
        #[arg(long)]
        tier: u8,
        /// Reference to the trained GGUF adapter artifact
        #[arg(long)]
        artifact: String,
        /// Corpus version the adapter was trained on
        #[arg(long = "corpus-version")]
        corpus_version: String,
    },
    /// List registered adapters (flags enabled / promoted / stale)
    List,
    /// Remove an adapter by id (restores the bare base — never weakens a gate)
    Remove {
        /// Adapter id (`<expert>:<base>:t<tier>`, from `adapter list`)
        id: String,
    },
    /// Disable an adapter by id without removing it
    Disable {
        /// Adapter id (`<expert>:<base>:t<tier>`)
        id: String,
    },
    /// Re-enable a previously disabled adapter by id
    Enable {
        /// Adapter id (`<expert>:<base>:t<tier>`)
        id: String,
    },
    /// Bench-gate an adapter: base vs base+adapter acceptance lift (ADR-2606161300 §5)
    Evaluate {
        /// Expert/adapter id to evaluate
        expert: String,
    },
}

/// LoRA idiom-expert corpus subcommands (ADR-2606161300 Phase 0).
#[derive(Subcommand)]
enum CorpusAction {
    /// Build an expert's corpus from hex's own ADRs/specs/exemplars
    Build {
        /// Expert name (e.g. hex-boundaries)
        expert: String,
        /// Compute the manifest without writing any files
        #[arg(long)]
        dry_run: bool,
    },
    /// List known experts and their current corpus manifest
    List,
}

pub async fn run(action: InferenceAction) -> anyhow::Result<()> {
    match action {
        InferenceAction::Add { provider_type, url, model, key, id, quantization } => {
            // Check if provider_type is a known template name (ADR-2026-04-05-2125)
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
        InferenceAction::Bench { target, model, quick, compare, save } => {
            bench_provider(&target, model.as_deref(), quick, compare.as_deref(), save).await
        }
        InferenceAction::GpuCheck { model, prompt } => gpu_check(&model, &prompt).await,
    }
}

/// `hex inference gpu-check` — verify Ollama runs the given model on the GPU (WP P1-4).
///
/// Workplan wp-bazzite-e2e-arch-validation task P1-4 required confirming that
/// `OLLAMA_VULKAN=true ollama run qwen3:4b "..."` streams a response from the GPU
/// (not CPU). This subcommand wraps that check so it can be re-run non-interactively.
async fn gpu_check(model: &str, prompt: &str) -> anyhow::Result<()> {
    use std::process::Command;
    use std::time::Duration;
    use tokio::time::timeout;

    println!("{}", "Ollama GPU inference check (WP P1-4)".bold().underline());
    println!("  Model:  {}", model);
    println!("  Prompt: {:?}", prompt);
    println!();

    // Best-effort GPU detection — absence isn't fatal, Ollama placement is authoritative.
    let gpu_probe = Command::new("rocm-smi")
        .arg("--showproductname")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| ("rocm-smi", String::from_utf8_lossy(&o.stdout).into_owned()))
        .or_else(|| {
            Command::new("nvidia-smi")
                .args(["--query-gpu=name", "--format=csv,noheader"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| ("nvidia-smi", String::from_utf8_lossy(&o.stdout).into_owned()))
        });

    match &gpu_probe {
        Some((tool, out)) => {
            let first = out.lines().find(|l| !l.trim().is_empty()).unwrap_or("(no name)");
            println!("  {} GPU detected ({}): {}", "✓".green(), tool, first.trim());
        }
        None => println!("  {} No rocm-smi / nvidia-smi — relying on Ollama placement", "!".yellow()),
    }

    println!("\n  {} Running inference (60s timeout)...", "→".cyan());
    let model_owned = model.to_string();
    let prompt_owned = prompt.to_string();
    let run = tokio::task::spawn_blocking(move || {
        Command::new("ollama")
            .env("OLLAMA_VULKAN", "true")
            .arg("run")
            .arg(&model_owned)
            .arg(&prompt_owned)
            .output()
    });
    let output = match timeout(Duration::from_secs(60), run).await {
        Ok(Ok(Ok(o))) => o,
        Ok(Ok(Err(e))) => anyhow::bail!("failed to spawn ollama: {}", e),
        Ok(Err(e)) => anyhow::bail!("ollama task join error: {}", e),
        Err(_) => anyhow::bail!("ollama inference timed out after 60s"),
    };
    if !output.status.success() {
        anyhow::bail!("ollama run failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let preview: String = first_line.chars().take(120).collect();
    println!("  {} Response streamed ({} bytes). First line: {}", "✓".green(), stdout.len(), preview);

    println!("\n  {} Checking `ollama ps` for GPU placement...", "→".cyan());
    let ps = Command::new("ollama").arg("ps").output()?;
    if !ps.status.success() {
        anyhow::bail!("`ollama ps` failed: {}", String::from_utf8_lossy(&ps.stderr).trim());
    }
    let ps_out = String::from_utf8_lossy(&ps.stdout);
    println!("{}", ps_out);

    let row = ps_out.lines().find(|l| l.contains(model));
    match row {
        Some(line) if line.contains("100% GPU") => {
            println!("  {} {} loaded at 100% GPU", "✓".green().bold(), model);
            Ok(())
        }
        Some(line) if line.contains("GPU") => {
            println!("  {} {} partially on GPU — not a full offload", "!".yellow().bold(), model);
            println!("    row: {}", line.trim());
            Ok(())
        }
        Some(line) => anyhow::bail!("model {} is NOT on GPU — row: {}", model, line.trim()),
        None => {
            // Model may unload between `ollama run` finishing and `ollama ps` — don't fail.
            println!("  {} {} not currently loaded (may have unloaded after run)", "!".yellow(), model);
            Ok(())
        }
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

            // If Ollama, list available models. Only repopulate discovered_models
            // when no specific --model was given (model_name is the placeholder "llama3").
            let explicit_model = model_name != "llama3" && !model_name.is_empty();
            if provider_type == "ollama" {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    if let Some(models) = body.get("models").and_then(|m| m.as_array()) {
                        println!("  {} Available models:", "ℹ".cyan());
                        if !explicit_model {
                            discovered_models.clear();
                        }
                        for m in models.iter().take(20) {
                            let name = m.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                            let size = m.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
                            let is_local = size > 0;
                            println!("    - {} ({:.1}GB)", name, size as f64 / 1_073_741_824.0);
                            if !explicit_model && is_local {
                                discovered_models.push(name.to_string());
                            }
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

    // Resolve quantization level (ADR-2026-03-27-1000):
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

    // Write the registry (ADR-2608241500 P6.2). This used to POST
    // /api/inference/register, which wrote a SpacetimeDB row, which the daemon
    // then preloaded back out of this same file on its next startup. The file
    // was always the source of truth; the database was a copy.
    let models: Vec<String> = serde_json::from_str(&models_json)
        .unwrap_or_else(|_| vec![model_name.to_string()]);
    let endpoint = hex_infer::Endpoint {
        id: provider_id.to_string(),
        url: url.trim_end_matches('/').to_string(),
        provider: provider_type.to_string(),
        model: model_name.to_string(),
        models,
        status: "unknown".to_string(),
        requires_auth: key.is_some(),
        secret_key: key.unwrap_or("").to_string(),
        health_checked_at: String::new(),
        quality_score: 0.0,
        quantization_level: resolved_quantization.clone().unwrap_or_default(),
    };
    match hex_infer::registry::upsert(endpoint) {
        Ok(()) => println!("  {} Written to {}", "✓".green(),
                           hex_infer::registry::registry_path().display()),
        Err(e) => anyhow::bail!("could not write the inference registry: {e}"),
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

/// Register a provider from a built-in template (ADR-2026-04-05-2125).
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

    // Write the registry. The rate-limit and cost fields the daemon stored
    // alongside the endpoint went with its telemetry; what routing needs is
    // where to send a request and which key opens it.
    let endpoint = hex_infer::Endpoint {
        id: provider_id.to_string(),
        url: template.base_url.trim_end_matches('/').to_string(),
        provider: template.provider_type.to_string(),
        model: model_ids.first().cloned().unwrap_or_else(|| "default".to_string()),
        models: model_ids.clone(),
        status: "unknown".to_string(),
        requires_auth: !api_key.is_empty(),
        secret_key: api_key.clone(),
        health_checked_at: String::new(),
        quality_score: 0.0,
        quantization_level: quantization.clone(),
    };
    match hex_infer::registry::upsert(endpoint) {
        Ok(()) => println!("  {} Written to the registry", "✓".green()),
        Err(e) => println!("  {} Registry write failed: {}", "!".yellow(), e),
    }

    println!();
    println!("{} {} registered with {} model(s)", "✓".green(), template.display_name, template.models.len());
    println!();
    if template.is_free_tier {
        println!("  Cost: {} (free tier)", "$0.00".green());
    }
    Ok(())
}

/// Discover all free-tier providers by checking env vars (ADR-2026-04-05-2125).
///
/// Probes known free-tier providers (Groq, Cerebras, SambaNova, Together, OpenRouter)
/// for API keys in environment variables and registers all discovered providers.
async fn discover_free_tier() -> anyhow::Result<()> {
    println!("{}", "── Discovering Free-Tier Inference Providers (ADR-2026-04-05-2125) ──".cyan());
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

/// The registered backends in the JSON shape `/api/inference/endpoints`
/// returned.
///
/// A shim, deliberately. Half a dozen call sites below read `qualityScore`,
/// `quantizationLevel` and friends off a `serde_json::Value`; handing them the
/// same shape from the registry file keeps the change to where the rows come
/// from, rather than rewriting six blocks of display code that were not wrong.
fn registry_rows() -> Vec<serde_json::Value> {
    hex_infer::registry::load()
        .iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "provider": e.provider,
                "url": e.url,
                "model": e.model,
                "models": e.models,
                "status": e.status,
                "requiresAuth": e.requires_auth,
                "apiKeyRef": e.secret_key,
                "healthCheckedAt": e.health_checked_at,
                "qualityScore": e.quality_score,
                "quantizationLevel": e.quantization_level,
            })
        })
        .collect()
}

async fn list_providers() -> anyhow::Result<()> {
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

    // The registry file (ADR-2608241500 P6.2). The daemon served these from
    // SpacetimeDB, which it had preloaded from this same file on startup.
    println!();
    println!("{}", "── Registered Backends ──".cyan());
    let endpoints = hex_infer::registry::load();
    if endpoints.is_empty() {
        println!("  None registered in {}.", hex_infer::registry::registry_path().display());
    }
    for e in &endpoints {
        let icon = if e.status == "healthy" || e.status == "ok" {
            "●".green()
        } else {
            "○".yellow()
        };
        let quality =
            if e.quality_score > 0.0 { format!(" q={:.2}", e.quality_score) } else { String::new() };
        let quant = if e.quantization_level.is_empty() { "?" } else { &e.quantization_level };
        println!(
            "  {} {} ({}) — {} [model: {}] [quant: {}{}]",
            icon, e.id, e.provider, e.url, e.model, quant, quality
        );
        if e.models.len() > 1 {
            println!("      serves: {}", e.models.join(", ").dimmed());
        }
    }

    println!();
    println!("Register new: hex config inference add ollama http://host:11434 --model <name>");

    Ok(())
}

async fn test_provider(target: Option<&str>, all: bool) -> anyhow::Result<()> {

    // ── --all: calibrate every uncalibrated provider ────────────────────────
    if all {
        let endpoints = registry_rows();
        if endpoints.is_empty() {
            println!("{} No backends registered", "!".yellow());
            return Ok(());
        }

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
            let mdl = extract_primary_model(p.get("model"));
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
    } else {
        let endpoints = registry_rows();
        let matches: Vec<_> = endpoints
            .into_iter()
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
                let mdl = extract_primary_model(p.get("model"));
                println!("  • {} ({})", id, ptype);
                test_single_provider(id, url_val, ptype, &mdl).await?;
            }
            return Ok(());
        }

        matches.into_iter().next().map(|p| {
            let model = extract_primary_model(p.get("model"));
            ProviderRecord {
                id: p.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                url: p.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                provider_type: p.get("provider").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                model,
            }
        })
    };

    let Some(record) = record else {
        println!("  {} No provider found for '{}' — trying as direct URL", "!".yellow(), target);
        let ptype = if target.contains("openrouter") { "openrouter" } else { "ollama" };
        test_single_provider(target, target, ptype, "").await?;
        return Ok(());
    };

    test_single_provider(&record.id, &record.url, &record.provider_type, &record.model).await
}

/// Extract the primary model name from an endpoint's `model` field.
/// The field may be a JSON array value, a JSON array string, or a plain string.
fn extract_primary_model(val: Option<&serde_json::Value>) -> String {
    match val {
        Some(v) if v.is_array() => {
            v.as_array().unwrap().first()
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string()
        }
        Some(v) if v.is_string() => {
            let raw = v.as_str().unwrap_or("");
            serde_json::from_str::<Vec<String>>(raw).ok()
                .and_then(|v| v.into_iter().next())
                .unwrap_or_else(|| raw.to_string())
        }
        _ => String::new(),
    }
}

async fn test_single_provider(id: &str, url: &str, provider_type: &str, model_name: &str) -> anyhow::Result<()> {
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
                        std::env::var("OPENROUTER_API_KEY").ok().map(serde_json::Value::String)
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

                // Persist the score to the registry file (ADR-2608241500 P6.2).
                let mut all = hex_infer::registry::load();
                match all.iter_mut().find(|e| e.id == id) {
                    Some(e) => {
                        e.quality_score = quality_score;
                        e.status = "healthy".to_string();
                        e.health_checked_at = chrono::Utc::now().to_rfc3339();
                        match hex_infer::registry::save(&all) {
                            Ok(()) => println!("  {} Calibration saved", "✓".green()),
                            Err(e) => println!("  {} Could not save calibration: {}", "!".yellow(), e),
                        }
                    }
                    None => println!(
                        "  {} '{}' is not in the registry — score not saved",
                        "!".yellow(),
                        id
                    ),
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
            // Prefer the registered model for this endpoint; fall back to smallest local model
            let test_model_opt = if !model_name.is_empty() {
                Some(model_name.to_string())
            } else {
                local_models.into_iter().next().map(|(_, n)| n)
            };

            // Quick inference test using the endpoint's registered model (or smallest if unset)
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

/// Is this backend answering? A live check, not the stored `status` field.
///
/// Ollama and the OpenAI-compatible family advertise their models at different
/// paths, and that difference is the whole reason this is not a plain GET.
async fn probe(http: &reqwest::Client, provider: &str, url: &str) -> bool {
    let base = url.trim_end_matches('/');
    let path = if provider == "ollama" { "/api/tags" } else { "/v1/models" };
    http.get(format!("{base}{path}"))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

async fn discover_ollama(prune: bool) -> anyhow::Result<()> {
    println!("{}", "── Discovering Inference Providers ──".cyan());
    println!();

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;

    let mut found = 0;

    // ── 1. What is already registered (the file is the source of truth) ──
    //
    // This used to query the daemon, which read SpacetimeDB, which it had
    // preloaded from this same file on startup. Reachability is still a live
    // probe rather than the stored `status` flag — a cached "healthy" tells
    // you what was true once.
    let mut registered_urls: Vec<String> = Vec::new();
    let mut registered_ids: Vec<String> = Vec::new();

    println!("{}", "── Registered Backends ──".cyan());
    let registered = hex_infer::registry::load();
    if registered.is_empty() {
        println!("  None registered yet.");
    }
    for e in &registered {
        let reachable = probe(&http, &e.provider, &e.url).await;
        let icon = if reachable { "●".green() } else { "○".red() };
        let status = if reachable { "online" } else { "offline" };
        println!("  {} {} ({}) — {} [{}]", icon, e.id, e.provider, e.url, status);
        registered_urls.push(e.url.clone());
        registered_ids.push(e.id.clone());
        if reachable {
            found += 1;
        }
    }
    println!();

    // ── Prune: drop backends that no longer answer ────
    if prune && !registered.is_empty() {
        println!("{}", "── Pruning unreachable backends ──".cyan());
        let mut kept: Vec<hex_infer::Endpoint> = Vec::new();
        for e in registered {
            if probe(&http, &e.provider, &e.url).await {
                println!("  {} {} OK", "✓".green(), e.id);
                kept.push(e);
            } else {
                println!("  {} Removed {} (unreachable)", "✗".red(), e.id);
            }
        }
        if let Err(err) = hex_infer::registry::save(&kept) {
            println!("  {} Could not write the registry: {}", "!".yellow(), err);
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

async fn discover_openrouter(filter: Option<&str>, min_context: Option<u64>) -> anyhow::Result<()> {
    println!("{}", "── Discovering OpenRouter Models ──".cyan());
    println!();

    // Check for API key
    let api_key = match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) => key,
        // The daemon's secrets vault is gone — a key is an environment
        // variable, which is how it reached the vault in the first place.
        Err(_) => String::new(),
    };

    if api_key.is_empty() {
        println!("  {} OPENROUTER_API_KEY not set.", "✗".red());
        println!("  Set it with: export OPENROUTER_API_KEY=sk-or-...");
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

        // Write it to the registry. Silent on failure: a discovery listing
        // should not stop because one entry could not be recorded.
        let endpoint = hex_infer::Endpoint {
            id: format!("openrouter-{}", id.replace('/', "-")),
            url: "https://openrouter.ai/api/v1".to_string(),
            provider: "openrouter".to_string(),
            model: id.to_string(),
            models: vec![id.to_string()],
            status: "unknown".to_string(),
            requires_auth: true,
            secret_key: "OPENROUTER_API_KEY".to_string(),
            health_checked_at: String::new(),
            quality_score: 0.0,
            quantization_level: "cloud".to_string(),
        };
        if hex_infer::registry::upsert(endpoint).is_ok() {
            registered += 1;
        }

        count += 1;
    }

    println!();
    println!("{} {} models found, {} registered.", "✓".green(), count, registered);

    Ok(())
}

async fn remove_provider(provider_id: &str) -> anyhow::Result<()> {
    match hex_infer::registry::remove(provider_id) {
        Ok(true) => println!("{} Removed backend: {}", "✓".green(), provider_id),
        Ok(false) => println!("{} No backend with id '{}'", "!".yellow(), provider_id),
        Err(e) => anyhow::bail!("could not write the inference registry: {e}"),
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

/// Record a calibration score against a registered backend.
///
/// Was a PATCH to `/api/inference/endpoints/{id}`, which the daemon turned
/// into a SpacetimeDB row update.
fn save_quality_score(id: &str, score: f32) -> Result<(), String> {
    let mut all = hex_infer::registry::load();
    let Some(e) = all.iter_mut().find(|e| e.id == id) else {
        return Err(format!("'{id}' is not registered"));
    };
    e.quality_score = score;
    e.status = "healthy".to_string();
    e.health_checked_at = chrono::Utc::now().to_rfc3339();
    hex_infer::registry::save(&all)
}

async fn setup_defaults() -> anyhow::Result<()> {
    println!("{}", "── Inference Setup ──".cyan());
    println!();

    // Require OpenRouter API key
    let api_key = std::env::var("OPENROUTER_API_KEY").ok()
        .filter(|k| !k.is_empty())
        .or_else(|| {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    std::env::var("OPENROUTER_API_KEY").ok().map(serde_json::Value::String)
                        .and_then(|v| v.get("value").and_then(|s| s.as_str()).map(|s| s.to_string()))
                        .filter(|s| !s.is_empty())
                })
            })
        });

    let Some(api_key) = api_key else {
        println!("  {} OPENROUTER_API_KEY not set — skipping inference setup.", "!".yellow());
        println!("  Set it first:  export OPENROUTER_API_KEY=sk-or-...");
        println!("  Then re-run:   hex config inference setup");
        return Ok(());
    };

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;

    let or_url = "https://openrouter.ai/api/v1";
    let mut calibrated = 0;

    for (model_id, purpose) in DEFAULT_MODELS {
        let provider_id = format!("openrouter-{}", model_id.replace('/', "-"));
        print!("  {} {} ({})... ", "→".cyan(), model_id, purpose);

        // Register it, then calibrate.
        let _ = hex_infer::registry::upsert(hex_infer::Endpoint {
            id: provider_id.clone(),
            url: or_url.to_string(),
            provider: "openrouter".to_string(),
            model: model_id.to_string(),
            models: vec![model_id.to_string()],
            status: "unknown".to_string(),
            requires_auth: true,
            secret_key: "OPENROUTER_API_KEY".to_string(),
            health_checked_at: String::new(),
            quality_score: 0.0,
            quantization_level: "cloud".to_string(),
        });

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

                match save_quality_score(&provider_id, quality_score) {
                    Ok(()) => println!("{} q={:.2} ({}ms)", "✓".green(), quality_score, latency_ms),
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
                        match save_quality_score(&provider_id, quality_score) {
                            Ok(()) => { println!("{} q={:.2} ({}ms)", "✓".green(), quality_score, latency_ms2); calibrated += 1; }
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

// ── Bench command (ADR-2026-04-13-1238) ──────────────────────────────────────────

/// Result of a single benchmark prompt.
#[allow(dead_code)]
struct BenchResult {
    name: &'static str,
    response: String,
    tokens: u64,
    wall_secs: f64,
    quality_score: f32,
    quality_max: u32,
    quality_details: Vec<(&'static str, bool)>,
}

/// A benchmark's raw quality points: the 0..1 score scaled by its own maximum.
///
/// One named function instead of the same expression in three places. The
/// float-to-int cast saturates in Rust rather than truncating, so this is not
/// the bug class the narrowing-cast rule is named for — but the expression was
/// hard to read three times and is easy to read once.
fn raw_quality(r: &BenchResult) -> u32 {
    let raw = r.quality_score * r.quality_max as f32;
    if !raw.is_finite() || raw <= 0.0 {
        0
    } else if raw >= u32::MAX as f32 {
        u32::MAX
    } else {
        raw as u32
    }
}

impl BenchResult {
    fn tok_per_sec(&self) -> f64 {
        if self.wall_secs > 0.0 { self.tokens as f64 / self.wall_secs } else { 0.0 }
    }
}

/// Send a chat completion to either Ollama or OpenAI-compatible endpoint.
async fn bench_chat(
    http: &reqwest::Client,
    url: &str,
    provider_type: &str,
    model: &str,
    prompt: &str,
) -> anyhow::Result<(String, u64, f64)> {
    let start = std::time::Instant::now();

    if provider_type == "openrouter" || url.contains("openrouter.ai") || url.contains("/v1") {
        let api_key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();
        let chat_url = format!("{}/chat/completions", url.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "temperature": 0.2,
            // Reasoning models (Nemotron, DeepSeek-R1, qwen3) spend output budget on
            // chain-of-thought before emitting the final answer. Without a generous
            // cap they hit finish_reason=length mid-thought and return empty content,
            // scoring 0 despite being correct. 8192 leaves room to finish.
            "max_tokens": 8192,
        });
        let resp = http.post(&chat_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&body)
            .send().await?;
        let d: serde_json::Value = resp.json().await?;
        let msg = &d["choices"][0]["message"];
        let mut content = msg["content"].as_str().unwrap_or("").to_string();
        // Reasoning models stream CoT into a separate `reasoning` field and only
        // fill `content` once they conclude. If content is empty (truncated or the
        // provider splits the fields), fall back to reasoning so a responsive model
        // isn't scored as a non-response.
        if content.trim().is_empty() {
            content = msg["reasoning"].as_str().unwrap_or("").to_string();
        }
        let tokens = d["usage"]["completion_tokens"].as_u64().unwrap_or(content.len() as u64 / 4);
        let wall = start.elapsed().as_secs_f64();
        Ok((content, tokens, wall))
    } else {
        // Ollama /api/chat
        let chat_url = format!("{}/api/chat", url.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "stream": false,
            // Ollama defaults num_predict to 128 — far too short for a reasoning
            // model, which burns that budget on chain-of-thought and returns an
            // empty/truncated answer (scoring 0). Mirror the OpenRouter cap.
            "options": {"temperature": 0.2, "num_predict": 8192},
        });
        let resp = http.post(&chat_url).json(&body).send().await?;
        let d: serde_json::Value = resp.json().await?;
        let msg = &d["message"];
        let mut content = msg["content"].as_str().unwrap_or("").to_string();
        // Reasoning models served via Ollama (qwen3, deepseek-r1) emit CoT into a
        // separate `thinking` field when think-mode is on. If the answer content
        // is empty, fall back to thinking so a responsive model isn't scored empty.
        if content.trim().is_empty() {
            content = msg["thinking"].as_str().unwrap_or("").to_string();
        }
        let tokens = d["eval_count"].as_u64().unwrap_or(content.len() as u64 / 4);
        let wall = start.elapsed().as_secs_f64();
        Ok((content, tokens, wall))
    }
}

// Run the identity prompt — measures latency floor and basic responsiveness.
// ── Persona-task benchmarks (the three shapes hex actually uses) ────────────
//
// These mirror what hex-nexus/src/orchestration/org_responder.rs +
// drafter.rs ask of the model day-to-day. A model that scores well on
// codegen but poorly here is the wrong default for the responder.
// Scoring is the same shape as scripts/bench-persona-prompts.py (which
// remains for ad-hoc / Python-only iteration); production lives here.

/// Detect rambling pre-answer narration ("we are in", "let me think", etc.)
/// Returns true if the response opens with one of the banned patterns.
fn has_meta_reasoning(text: &str) -> bool {
    const BAD: &[&str] = &[
        "we are in", "the user is", "let me recall", "let me think",
        "i need to recall", "i'll respond", "i will respond",
        "first, i note", "key points from", "looking at the",
    ];
    let head = text.chars().take(400).collect::<String>().to_lowercase();
    BAD.iter().any(|b| head.contains(b))
}

/// Count distinct grounded references (ADR ids, repo paths) in the response.
fn count_grounded(text: &str) -> usize {
    let lower = text.to_lowercase();
    let adr_re = regex::Regex::new(r"adr-\d{4}-\d{2}-\d{2}-\d{4}|adr-\d+").unwrap();
    let adrs = adr_re.find_iter(&lower).count();
    const PATHS: &[&str] = &[
        "docs/specs/", "docs/adrs/", "hex-nexus/src/", "hex-cli/src/",
        "spacetime-modules/", "hex-nexus/assets/src/", "scripts/",
    ];
    let paths: usize = PATHS.iter().filter(|p| lower.contains(*p)).count();
    adrs + paths
}

/// chat-mode bench: brief grounded status reply, no meta-reasoning.
async fn bench_persona_chat(
    http: &reqwest::Client, url: &str, ptype: &str, model: &str,
) -> anyhow::Result<BenchResult> {
    let system = "You are CTO. Answer in 2-3 sentences. Cite a real ADR id (ADR-2026-05-08-2500 form) or repo path (docs/specs/X.md). \
                  Do NOT begin with: 'We are', 'The user', 'Let me', 'Looking at'. Just answer.";
    let user = "Status: shipped today, in flight, top concern.";
    let (response, tokens, wall) = bench_chat(http, url, ptype, model, &format!("{}\n\nUser: {}", system, user)).await?;
    let word_count = response.split_whitespace().count();
    let grounded = count_grounded(&response);
    let checks: Vec<(&str, bool)> = vec![
        ("non-empty",       !response.trim().is_empty()),
        ("under 120 words", word_count > 0 && word_count <= 120),
        ("cites artifact",  grounded >= 1),
        ("no meta-prelude", !has_meta_reasoning(&response)),
    ];
    let passed = checks.iter().filter(|(_, v)| *v).count() as f32;
    Ok(BenchResult {
        name: "Persona/chat",
        quality_score: passed / 4.0,
        quality_max: 4,
        quality_details: checks,
        response, tokens, wall_secs: wall,
    })
}

/// commit-mode bench: strict Confirm: format on a single line.
async fn bench_persona_commit(
    http: &reqwest::Client, url: &str, ptype: &str, model: &str,
) -> anyhow::Result<BenchResult> {
    let system = "You are CTO. Reply with EXACTLY ONE line in the form:\n\
                  Confirm: I (cto) will <action> by <deadline> — success: <artifact path>\n\
                  OR the single word: Silent\n\
                  Examples:\n\
                  Confirm: I (cto) will write docs/specs/cost-runbook.md by EOD — success: docs/specs/cost-runbook.md\n\
                  Silent\n\
                  No preamble. Begin with C or S.";
    let user = "Write docs/specs/persona-bench-sample.md by EOD.";
    let (response, tokens, wall) = bench_chat(http, url, ptype, model, &format!("{}\n\nUser: {}", system, user)).await?;
    let stripped = response.trim();
    let first_line = stripped.lines().next().unwrap_or("").trim();
    let is_confirm = first_line.to_lowercase().starts_with("confirm:");
    let is_silent = stripped.to_lowercase() == "silent" || stripped.to_lowercase() == "silent.";
    let checks: Vec<(&str, bool)> = vec![
        ("non-empty",     !stripped.is_empty()),
        ("Confirm/Silent", is_confirm || is_silent),
        ("single line",   stripped.lines().count() <= 1),
        ("cites path",    is_silent || count_grounded(&response) >= 1),
        ("no meta-prelude", !has_meta_reasoning(&response)),
    ];
    let passed = checks.iter().filter(|(_, v)| *v).count() as f32;
    Ok(BenchResult {
        name: "Persona/commit",
        quality_score: passed / 5.0,
        quality_max: 5,
        quality_details: checks,
        response, tokens, wall_secs: wall,
    })
}

/// drafter-mode bench: literal file body, no preamble.
async fn bench_persona_drafter(
    http: &reqwest::Client, url: &str, ptype: &str, model: &str,
) -> anyhow::Result<BenchResult> {
    let system = "Write the body of `docs/specs/persona-bench.md` per the request below. \
                  Output ONLY the file contents. First character of output is the first character of the file. \
                  No 'Okay', no 'Sure', no 'Here is', no code fences.";
    let user = "The file should contain only one line: Hello from the bench.";
    let (response, tokens, wall) = bench_chat(http, url, ptype, model, &format!("{}\n\nUser: {}", system, user)).await?;
    let stripped = response.trim();
    let lower = stripped.to_lowercase();
    let starts_clean = !["okay", "sure", "here", "i'll", "below", "let me", "i will", "of course"]
        .iter().any(|p| lower.starts_with(p));
    let has_exact = lower.contains("hello from the bench");
    let checks: Vec<(&str, bool)> = vec![
        ("non-empty",     !stripped.is_empty()),
        ("no preamble",   starts_clean),
        ("has target",    has_exact),
        ("no meta-prelude", !has_meta_reasoning(&response)),
        ("reasonable size", stripped.len() <= 2048),
    ];
    let passed = checks.iter().filter(|(_, v)| *v).count() as f32;
    Ok(BenchResult {
        name: "Persona/drafter",
        quality_score: passed / 5.0,
        quality_max: 5,
        quality_details: checks,
        response, tokens, wall_secs: wall,
    })
}

async fn bench_identity(
    http: &reqwest::Client, url: &str, ptype: &str, model: &str,
) -> anyhow::Result<BenchResult> {
    let (response, tokens, wall) = bench_chat(
        http, url, ptype, model,
        "What model are you? Respond in one sentence.",
    ).await?;
    let non_empty = !response.trim().is_empty();
    Ok(BenchResult {
        name: "Identity",
        quality_score: if non_empty { 1.0 } else { 0.0 },
        quality_max: 1,
        quality_details: vec![("responsive", non_empty)],
        response, tokens, wall_secs: wall,
    })
}

/// Run the code generation prompt — measures Rust code quality for hex adapter work.
async fn bench_codegen(
    http: &reqwest::Client, url: &str, ptype: &str, model: &str,
) -> anyhow::Result<BenchResult> {
    let prompt = r#"You are a Rust developer. Generate a complete secondary adapter implementing a WeatherPort trait.
Requirements:
1. Define the port trait: WeatherPort with async fn get_forecast(city: &str) -> Result<Forecast, WeatherError>
2. Define domain types: Forecast (city, temp_celsius, humidity, description) and WeatherError enum
3. Implement HttpWeatherAdapter that calls an HTTP API using reqwest
4. Handle timeouts, parse errors, and API errors as distinct WeatherError variants (use thiserror)
5. Include unit tests with a mock (trait object, not mock library)
6. All code must compile. Use proper error handling.
Output complete Rust code."#;

    let (response, tokens, wall) = bench_chat(http, url, ptype, model, prompt).await?;

    let checks: Vec<(&str, bool)> = vec![
        ("async fn", response.contains("async fn")),
        ("thiserror", response.contains("thiserror")),
        ("tests", response.contains("#[cfg(test)]") || response.contains("#[test]")),
        ("reqwest", response.contains("reqwest")),
        ("error variants", response.matches("Error").count() >= 3),
        ("Result<>", response.contains("Result<")),
        ("trait def", response.contains("trait Weather") || response.contains("trait weather")),
        ("mock test", response.to_lowercase().contains("mock")),
        ("derives", response.contains("#[derive")),
        ("timeout", response.to_lowercase().contains("timeout")),
    ];
    let passed = checks.iter().filter(|(_, v)| *v).count() as f32;

    Ok(BenchResult {
        name: "Code-gen",
        quality_score: passed / 10.0,
        quality_max: 10,
        quality_details: checks,
        response, tokens, wall_secs: wall,
    })
}

/// Run the reasoning prompt — measures architectural analysis capability.
async fn bench_reasoning(
    http: &reqwest::Client, url: &str, ptype: &str, model: &str,
) -> anyhow::Result<BenchResult> {
    let prompt = r#"In a hexagonal architecture (ports and adapters) Rust project, a developer wrote this in adapters/http/handler.rs:
```rust
use crate::adapters::database::PostgresRepo;
```
Identify the architectural violation, explain which rule it breaks, and describe how to fix it. Be specific about dependency inversion."#;

    let (response, tokens, wall) = bench_chat(http, url, ptype, model, prompt).await?;
    let lower = response.to_lowercase();

    let checks: Vec<(&str, bool)> = vec![
        ("cross-adapter", lower.contains("adapter") && (lower.contains("import") || lower.contains("depend") || lower.contains("coupl"))),
        ("names rule", lower.contains("port") || lower.contains("boundary") || lower.contains("hexagonal")),
        ("port extraction", lower.contains("trait") || lower.contains("interface") || lower.contains("port")),
        ("dep inversion", lower.contains("inversion") || lower.contains("abstraction") || lower.contains("inject")),
        ("code example", response.contains("trait ") || response.contains("fn ") || response.contains("impl ")),
    ];
    let passed = checks.iter().filter(|(_, v)| *v).count() as f32;

    Ok(BenchResult {
        name: "Reasoning",
        quality_score: passed / 5.0,
        quality_max: 5,
        quality_details: checks,
        response, tokens, wall_secs: wall,
    })
}

/// Compute overall score and recommend tier.
fn compute_tier(results: &[&BenchResult]) -> (f32, u8, &'static str) {
    let codegen = results.iter().find(|r| r.name == "Code-gen");
    let reasoning = results.iter().find(|r| r.name == "Reasoning");

    let code_score = codegen.map(|r| r.quality_score).unwrap_or(0.0);
    let reason_score = reasoning.map(|r| r.quality_score).unwrap_or(0.0);

    // Average tok/s across all prompts (excluding identity for fairness)
    let heavy: Vec<_> = results.iter().filter(|r| r.name != "Identity").collect();
    let avg_tps = if heavy.is_empty() { 0.0 } else {
        heavy.iter().map(|r| r.tok_per_sec()).sum::<f64>() / heavy.len() as f64
    };
    let latency_score: f32 = if avg_tps > 100.0 { 1.0 }
        else if avg_tps > 50.0 { 0.8 }
        else if avg_tps > 20.0 { 0.6 }
        else if avg_tps > 5.0 { 0.3 }
        else { 0.1 };

    let overall = code_score * 0.5 + reason_score * 0.3 + latency_score * 0.2;

    let code_raw = codegen.map(|r| raw_quality(&r)).unwrap_or(0);
    let reason_raw = reasoning.map(|r| raw_quality(&r)).unwrap_or(0);

    let (tier, label) = if overall >= 0.85 && reason_raw >= 4 {
        (3, "Tier 3 (Opus-equivalent: planning, specs, validation)")
    } else if overall >= 0.65 && code_raw >= 7 {
        (2, "Tier 2 (Sonnet-equivalent: code generation, review)")
    } else if overall >= 0.40 {
        (1, "Tier 1 (Haiku-equivalent: simple edits, summarization)")
    } else {
        (0, "Not recommended for hex agent work")
    };

    (overall, tier, label)
}

/// Print benchmark results for one model.
fn print_bench_results(model: &str, results: &[BenchResult], label: Option<&str>) {
    if let Some(lbl) = label {
        println!("  {}", format!("── {} ──", lbl).cyan());
    }
    println!();
    for r in results {
        let status = if r.quality_score >= 0.6 { "✓".green() } else if r.quality_score >= 0.3 { "~".yellow() } else { "✗".red() };
        let q = raw_quality(r);
        println!("  {}  {:<12} {:>5.1}s  ({}/{} quality, {:.0} tok/s)",
            status, r.name, r.wall_secs, q, r.quality_max, r.tok_per_sec());
        for (name, passed) in &r.quality_details {
            let mark = if *passed { "✓".green() } else { "✗".red() };
            print!("     {} {}", mark, name);
        }
        println!();
    }

    let refs: Vec<&BenchResult> = results.iter().collect();
    let (overall, tier, tier_label) = compute_tier(&refs);

    let heavy: Vec<_> = results.iter().filter(|r| r.name != "Identity").collect();
    let avg_tps = if heavy.is_empty() { 0.0 } else {
        heavy.iter().map(|r| r.tok_per_sec()).sum::<f64>() / heavy.len() as f64
    };

    println!();
    println!("  {}", "── Summary ──────────────────────────────────".dimmed());
    println!("  Model:            {}", model);
    println!("  Overall score:    {:.2}", overall);
    println!("  Avg tok/s:        {:.0}", avg_tps);
    println!("  Recommended:      {}", tier_label);
    if tier >= 2 {
        println!("  Best for:         code_generation, code_edit");
    } else if tier == 1 {
        println!("  Best for:         general, structured_output");
    }
    println!();

    // score is computed inline
}

/// `hex inference bench` — benchmark a model with hex-specific prompts (ADR-2026-04-13-1238).
async fn bench_provider(
    target: &str,
    model_override: Option<&str>,
    quick: bool,
    compare: Option<&str>,
    save: bool,
) -> anyhow::Result<()> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    // ── Resolve target ──────────────────────────────────────────────────────
    struct Resolved { id: String, url: String, ptype: String, model: String }

    let resolve = |target_str: &str, model_ov: Option<&str>| -> Option<Resolved> {
        if target_str.starts_with("http") {
            let ptype = if target_str.contains("openrouter.ai") { "openrouter" }
                else if target_str.contains(":11434") { "ollama" }
                else { "openai-compat" };
            return Some(Resolved {
                id: target_str.to_string(), url: target_str.to_string(),
                ptype: ptype.to_string(),
                model: model_ov.unwrap_or("").to_string(),
            });
        }
        // Check if target looks like a model name (contains : or /)
        if target_str.contains(':') || target_str.contains('/') {
            // It's a model name — need to find a provider that has it
            return Some(Resolved {
                id: target_str.to_string(), url: String::new(),
                ptype: String::new(),
                model: target_str.to_string(),
            });
        }
        None
    };

    let mut resolved = resolve(target, model_override);

    // Registry lookup, if the target is not already a direct URL or model.
    if resolved.is_none() || resolved.as_ref().map(|r| r.url.is_empty()).unwrap_or(false) {
            {
                {
                    let endpoints = &registry_rows();
                    // Exact ID match or prefix match
                    let found = endpoints.iter().find(|p| {
                        let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        id == target || id.starts_with(&format!("{}-", target))
                    });
                    if let Some(p) = found {
                        let model = model_override
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| extract_primary_model(p.get("model")));
                        resolved = Some(Resolved {
                            id: p["id"].as_str().unwrap_or(target).to_string(),
                            url: p["url"].as_str().unwrap_or("").to_string(),
                            ptype: p["provider"].as_str().unwrap_or("ollama").to_string(),
                            model,
                        });
                    }
                    // If target looks like a model name, find provider that serves it
                    if resolved.is_none() || resolved.as_ref().map(|r| r.url.is_empty()).unwrap_or(false) {
                        let model_target = model_override.unwrap_or(target);
                        let host = endpoints.iter().find(|p| {
                            p.get("models")
                                .and_then(|v| v.as_array())
                                .map(|a| a.iter().any(|m| m.as_str() == Some(model_target)))
                                .unwrap_or(false)
                        });
                        if let Some(p) = host {
                            resolved = Some(Resolved {
                                id: p["id"].as_str().unwrap_or(target).to_string(),
                                url: p["url"].as_str().unwrap_or("").to_string(),
                                ptype: p["provider"].as_str().unwrap_or("ollama").to_string(),
                                model: model_target.to_string(),
                            });
                        }
                    }
                }
            }
    }

    let Some(mut r) = resolved.filter(|r| !r.url.is_empty()) else {
        println!("{} Could not resolve target '{}' — register it first with `hex inference add`", "✗".red(), target);
        return Ok(());
    };

    // Cloud openai-compat backends used to be benched through the daemon's
    // `/v1` proxy, because their API key was a vault reference only the daemon
    // could resolve. A key is an environment variable now, so the direct path
    // works for every backend — and it measures true latency rather than the
    // latency of a hop through a proxy.

    // An OpenRouter bench with no key in the environment used to be rescued by
    // reading the daemon's vault. Without a key the Bearer header is empty and
    // every request fails auth, which scores the model 0 — so say so rather
    // than letting a configuration gap look like a bad model.
    if (r.ptype == "openrouter" || r.url.contains("openrouter.ai"))
        && std::env::var("OPENROUTER_API_KEY").map(|v| v.trim().is_empty()).unwrap_or(true)
    {
        println!(
            "{} OPENROUTER_API_KEY is not set — the bench would fail auth and score 0, \
             which measures configuration, not the model. Set it with: \
             export OPENROUTER_API_KEY=sk-or-...",
            "✗".red()
        );
        return Ok(());
    }

    println!("{}", format!("── hex inference bench: {} via {} ──", r.model, r.id).cyan());
    println!();

    // ── Run benchmarks ──────────────────────────────────────────────────────
    let run_suite = |http: &reqwest::Client, url: &str, ptype: &str, model: &str, quick: bool| {
        let http = http.clone();
        let url = url.to_string();
        let ptype = ptype.to_string();
        let model = model.to_string();
        async move {
            let mut results: Vec<BenchResult> = Vec::new();

            // Identity
            print!("  {} Running identity probe...", "→".cyan());
            match bench_identity(&http, &url, &ptype, &model).await {
                Ok(br) => { println!(" {:.1}s", br.wall_secs); results.push(br); }
                Err(e) => { println!(" {} {}", "✗".red(), e); }
            }

            // Code generation (skip in --quick mode)
            if !quick {
                print!("  {} Running code generation benchmark...", "→".cyan());
                match bench_codegen(&http, &url, &ptype, &model).await {
                    Ok(br) => { println!(" {:.1}s", br.wall_secs); results.push(br); }
                    Err(e) => { println!(" {} {}", "✗".red(), e); }
                }
            }

            // Reasoning
            print!("  {} Running reasoning benchmark...", "→".cyan());
            match bench_reasoning(&http, &url, &ptype, &model).await {
                Ok(br) => { println!(" {:.1}s", br.wall_secs); results.push(br); }
                Err(e) => { println!(" {} {}", "✗".red(), e); }
            }

            // Persona-task benchmarks — the three shapes the responder/drafter
            // actually use. A model can ace codegen and still ramble on these.
            print!("  {} Running persona/chat benchmark...", "→".cyan());
            match bench_persona_chat(&http, &url, &ptype, &model).await {
                Ok(br) => { println!(" {:.1}s", br.wall_secs); results.push(br); }
                Err(e) => { println!(" {} {}", "✗".red(), e); }
            }
            print!("  {} Running persona/commit benchmark...", "→".cyan());
            match bench_persona_commit(&http, &url, &ptype, &model).await {
                Ok(br) => { println!(" {:.1}s", br.wall_secs); results.push(br); }
                Err(e) => { println!(" {} {}", "✗".red(), e); }
            }
            print!("  {} Running persona/drafter benchmark...", "→".cyan());
            match bench_persona_drafter(&http, &url, &ptype, &model).await {
                Ok(br) => { println!(" {:.1}s", br.wall_secs); results.push(br); }
                Err(e) => { println!(" {} {}", "✗".red(), e); }
            }

            results
        }
    };

    let results = run_suite(&http, &r.url, &r.ptype, &r.model, quick).await;

    if results.is_empty() {
        println!("{} All prompts failed — model may be unreachable", "✗".red());
        return Ok(());
    }

    println!();
    print_bench_results(&r.model, &results, None);

    // ── Compare mode ────────────────────────────────────────────────────────
    if let Some(baseline_target) = compare {
        // Resolve baseline the same way
        let baseline_resolved = hex_infer::registry::load()
            .into_iter()
            .find(|e| {
                e.id == baseline_target || e.id.starts_with(&format!("{}-", baseline_target))
            })
            .map(|e| Resolved {
                id: e.id,
                url: e.url,
                ptype: e.provider,
                model: e.model,
            });

        if let Some(bl) = baseline_resolved.filter(|b| !b.url.is_empty()) {
            println!("{}", format!("── Baseline: {} via {} ──", bl.model, bl.id).cyan());
            println!();
            let bl_results = run_suite(&http, &bl.url, &bl.ptype, &bl.model, quick).await;
            if !bl_results.is_empty() {
                print_bench_results(&bl.model, &bl_results, Some("Baseline"));
            }
        } else {
            println!("{} Could not resolve baseline '{}'", "!".yellow(), baseline_target);
        }
    }

    // ── Save calibration ────────────────────────────────────────────────────
    if save {
        let refs: Vec<&BenchResult> = results.iter().collect();
        let (overall, tier, _) = compute_tier(&refs);
        match save_quality_score(&r.id, overall) {
            Ok(()) => println!(
                "{} Calibration saved (score={:.2}, tier={})",
                "✓".green(),
                overall,
                tier
            ),
            Err(e) => println!("{} Could not save calibration: {}", "!".yellow(), e),
        }
    }

    Ok(())
}

