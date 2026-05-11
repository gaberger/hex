//! Substrate operator commands (ADR-2026-04-26-1800 P4,
//! wp-substrate-inference-consumer-rewires P4.1).
//!
//! `hex substrate swap-inference` — propose a candidate inference strategy
//! for shadow-promotion. The substrate mirrors traffic between the live
//! binding and the candidate; the judge evaluates samples against the
//! configured success criteria; the promote orchestrator flips the live
//! binding if the judge greenlights.
//!
//! `hex substrate swap-list` — show in-flight tickets.
//! `hex substrate swap-samples <id>` — show samples for a ticket.

use clap::Subcommand;
use colored::Colorize;
use serde_json::{json, Value};

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum SubstrateAction {
    /// Propose an inference strategy swap. Substrate mirrors traffic to
    /// the candidate during shadow, then judge + orchestrator decide.
    SwapInference {
        /// Adapter id to register the candidate strategy under (e.g.
        /// "fixed-qwen-32b", "noop-killswitch").
        #[arg(long)]
        candidate: String,
        /// JSON spec for the candidate strategy. Examples:
        ///
        ///   --strategy '{"kind":"noop"}'
        ///
        ///   --strategy '{"kind":"fixed","model":"qwen2.5-coder:32b","base_url":"http://localhost:11434"}'
        #[arg(long)]
        strategy: String,
        /// Fraction of traffic to mirror to the candidate during shadow
        /// (0.0..=1.0). Defaults to 0.05.
        #[arg(long, default_value = "0.05")]
        fraction: f32,
        /// Shadow window in seconds before the judge evaluates. Defaults
        /// to 300 (5 min).
        #[arg(long = "window-secs", default_value = "300")]
        window_secs: u64,
        /// Success criterion (repeatable). Format: `kind=value`. Examples:
        ///   --criterion error-rate-below=0.1
        ///   --criterion latency-p99-below-ms=200
        ///   --criterion response-equivalence=0.05
        #[arg(long = "criterion")]
        criteria: Vec<String>,
        /// Dry run — submit to /api/swaps/dry-run instead of /propose.
        /// Runs the L2 adversarial swarm against the proposed swap and
        /// returns the verdict without touching STDB or the router.
        /// Useful before committing a real swap.
        #[arg(long)]
        dry_run: bool,
    },
    /// List in-flight swap tickets (state=shadow).
    SwapList,
    /// Show shadow_sample rows for a ticket.
    SwapSamples {
        /// Ticket id (uuid).
        ticket_id: String,
    },
    /// Layer 6 quarterly ritual — read founding-goals.md, run each goal's
    /// `Test` field, report drift (ADR-2026-04-26-1500 P9 — first L6 ritual).
    L6Ritual {
        /// Path to founding-goals.md. Defaults to `./founding-goals.md`.
        #[arg(long, default_value = "founding-goals.md")]
        goals_path: String,
    },
    /// Substrate health snapshot (substrate_wired, ticket counts, live
    /// bindings, router state). Read-only.
    Status,
    /// Propose a secret-port swap. Today the only strategy is
    /// `EnvSecretAdapter` with an alternate prefix — operator can
    /// shadow-test "what if HEX_SECRET_ prefix was HEX_TEST_SECRET_".
    /// Future ISecretPort impls (vault, OS keychain) will land their own
    /// CLI args here.
    SwapSecret {
        /// Adapter id to register the candidate strategy under (e.g.
        /// "alt-prefix").
        #[arg(long)]
        candidate: String,
        /// Env-var prefix the candidate EnvSecretAdapter will use.
        /// Empty string = direct lookup.
        #[arg(long, default_value = "")]
        base_prefix: String,
        /// Fraction of traffic to mirror (0.0..=1.0). Default 0.05.
        #[arg(long, default_value = "0.05")]
        fraction: f32,
        /// Shadow window in seconds. Default 300.
        #[arg(long = "window-secs", default_value = "300")]
        window_secs: u64,
        /// Success criterion (repeatable). Same format as swap-inference.
        #[arg(long = "criterion")]
        criteria: Vec<String>,
        /// Dry run — submit to /api/swaps/secret/dry-run instead of
        /// /propose. Returns the L2 verdict without touching STDB or
        /// the router.
        #[arg(long)]
        dry_run: bool,
    },
}

pub async fn run(action: SubstrateAction) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    match action {
        SubstrateAction::SwapInference {
            candidate,
            strategy,
            fraction,
            window_secs,
            criteria,
            dry_run,
        } => {
            swap_inference(
                &nexus, &candidate, &strategy, fraction, window_secs, &criteria, dry_run,
            )
            .await
        }
        SubstrateAction::SwapList => swap_list(&nexus).await,
        SubstrateAction::SwapSamples { ticket_id } => swap_samples(&nexus, &ticket_id).await,
        SubstrateAction::L6Ritual { goals_path } => l6_ritual(&nexus, &goals_path).await,
        SubstrateAction::Status => substrate_status(&nexus).await,
        SubstrateAction::SwapSecret {
            candidate,
            base_prefix,
            fraction,
            window_secs,
            criteria,
            dry_run,
        } => {
            swap_secret(
                &nexus, &candidate, &base_prefix, fraction, window_secs, &criteria, dry_run,
            )
            .await
        }
    }
}

async fn swap_secret(
    nexus: &NexusClient,
    candidate: &str,
    base_prefix: &str,
    fraction: f32,
    window_secs: u64,
    criteria: &[String],
    dry_run: bool,
) -> anyhow::Result<()> {
    let parsed_criteria: Vec<Value> = criteria
        .iter()
        .map(|s| parse_criterion(s))
        .collect::<Result<Vec<_>, _>>()?;
    let body = json!({
        "candidate_adapter_id": candidate,
        "base_prefix": base_prefix,
        "shadow_traffic_fraction": fraction,
        "shadow_window_seconds": window_secs,
        "success_criteria": parsed_criteria,
    });

    if dry_run {
        let resp = nexus.post("/api/swaps/secret/dry-run", &body).await?;
        let approve = resp.get("approve").and_then(|v| v.as_bool()).unwrap_or(false);
        if approve {
            println!("{} L2 swarm would approve this secret-port swap.", "✓".green().bold());
            println!("{}", "  (no STDB writes performed)".dimmed());
        } else {
            println!("{} L2 swarm would REJECT this secret-port swap:", "✗".red().bold());
            if let Some(rejs) = resp.get("rejections").and_then(|v| v.as_array()) {
                for r in rejs {
                    if let Some(arr) = r.as_array() {
                        let name = arr.first().and_then(|v| v.as_str()).unwrap_or("?");
                        let reason = arr.get(1).and_then(|v| v.as_str()).unwrap_or("?");
                        println!("    {} {}", format!("[{}]", name).yellow(), reason);
                    }
                }
            }
            std::process::exit(2);
        }
        return Ok(());
    }

    let resp = nexus.post("/api/swaps/secret/propose", &body).await?;
    let ticket_id = resp.get("ticket_id").and_then(|v| v.as_str()).unwrap_or("?");
    let port_id = resp.get("port_id").and_then(|v| v.as_str()).unwrap_or("?");
    let cand = resp.get("candidate_adapter_id").and_then(|v| v.as_str()).unwrap_or("?");
    let state = resp.get("state").and_then(|v| v.as_str()).unwrap_or("?");
    let started = resp.get("shadow_started_at").and_then(|v| v.as_str()).unwrap_or("?");
    println!("{}", "✓ secret swap proposed".green().bold());
    println!("  ticket:    {}", ticket_id.cyan());
    println!("  port:      {}", port_id);
    println!("  candidate: {}", cand);
    println!("  prefix:    {}", if base_prefix.is_empty() { "(none — direct lookup)".dimmed().to_string() } else { base_prefix.to_string() });
    println!("  state:     {}", state.blue());
    println!("  fraction:  {:.0}%", fraction * 100.0);
    println!("  window:    {}s", window_secs);
    println!("  started:   {}", started);
    Ok(())
}

async fn substrate_status(nexus: &NexusClient) -> anyhow::Result<()> {
    let resp = nexus.get("/api/substrate/status").await?;
    let wired = resp.get("substrate_wired").and_then(|v| v.as_bool()).unwrap_or(false);
    println!("{}", "Substrate status".bold());
    if wired {
        println!("  wired:           {}", "yes".green());
    } else {
        println!("  wired:           {}", "no (substrate not initialized)".red());
        return Ok(());
    }

    if let Some(t) = resp.get("tickets") {
        let shadow = t.get("shadow").and_then(|v| v.as_u64()).unwrap_or(0);
        let green = t.get("shadow_green").and_then(|v| v.as_u64()).unwrap_or(0);
        println!("  tickets shadow:  {}", shadow);
        println!("  tickets green:   {}", green);
    }
    if let Some(r) = resp.get("router") {
        let handles = r.get("handles_registered").and_then(|v| v.as_u64()).unwrap_or(0);
        let active = r.get("active_shadows").and_then(|v| v.as_u64()).unwrap_or(0);
        println!("  router handles:  {} ({} active shadows)", handles, active);
    }
    if let Some(per_port) = resp.get("router_per_port").and_then(|v| v.as_object()) {
        println!("  router per-port:");
        for (port, stats) in per_port {
            let h = stats.get("handles").and_then(|v| v.as_u64()).unwrap_or(0);
            let a = stats.get("active_shadows").and_then(|v| v.as_u64()).unwrap_or(0);
            println!("    {}: {} handle(s), {} active shadow(s)", port.cyan(), h, a);
        }
    }
    if let Some(b) = resp.get("live_bindings").and_then(|v| v.as_object()) {
        if b.is_empty() {
            println!("  live bindings:   {}", "(none)".dimmed());
        } else {
            println!("  live bindings:");
            for (port, adapter) in b {
                let id = adapter.as_str().unwrap_or("?");
                println!("    {} → {}", port.cyan(), id.cyan());
            }
        }
    }
    Ok(())
}

async fn swap_inference(
    nexus: &NexusClient,
    candidate: &str,
    strategy_json: &str,
    fraction: f32,
    window_secs: u64,
    criteria: &[String],
    dry_run: bool,
) -> anyhow::Result<()> {
    let strategy_spec: Value = serde_json::from_str(strategy_json)
        .map_err(|e| anyhow::anyhow!("--strategy is not valid JSON: {}", e))?;

    let parsed_criteria: Vec<Value> = criteria
        .iter()
        .map(|s| parse_criterion(s))
        .collect::<Result<Vec<_>, _>>()?;

    let body = json!({
        "candidate_adapter_id": candidate,
        "strategy_spec": strategy_spec,
        "shadow_traffic_fraction": fraction,
        "shadow_window_seconds": window_secs,
        "success_criteria": parsed_criteria,
    });

    if dry_run {
        let resp = nexus.post("/api/swaps/dry-run", &body).await?;
        let approve = resp.get("approve").and_then(|v| v.as_bool()).unwrap_or(false);
        if approve {
            println!("{} L2 swarm would approve this swap.", "✓".green().bold());
            println!("{}", "  (no STDB writes performed)".dimmed());
        } else {
            println!("{} L2 swarm would REJECT this swap:", "✗".red().bold());
            if let Some(rejs) = resp.get("rejections").and_then(|v| v.as_array()) {
                for r in rejs {
                    if let Some(arr) = r.as_array() {
                        let name = arr.first().and_then(|v| v.as_str()).unwrap_or("?");
                        let reason = arr.get(1).and_then(|v| v.as_str()).unwrap_or("?");
                        println!("    {} {}", format!("[{}]", name).yellow(), reason);
                    }
                }
            }
            std::process::exit(2);
        }
        return Ok(());
    }

    let resp = nexus.post("/api/swaps/propose", &body).await?;

    let ticket_id = resp.get("ticket_id").and_then(|v| v.as_str()).unwrap_or("?");
    let port_id = resp.get("port_id").and_then(|v| v.as_str()).unwrap_or("?");
    let cand = resp.get("candidate_adapter_id").and_then(|v| v.as_str()).unwrap_or("?");
    let state = resp.get("state").and_then(|v| v.as_str()).unwrap_or("?");
    let started = resp.get("shadow_started_at").and_then(|v| v.as_str()).unwrap_or("?");

    println!("{}", "✓ swap proposed".green().bold());
    println!("  ticket:    {}", ticket_id.cyan());
    println!("  port:      {}", port_id);
    println!("  candidate: {}", cand);
    println!("  state:     {}", state.blue());
    println!("  fraction:  {:.0}%", fraction * 100.0);
    println!("  window:    {}s", window_secs);
    println!("  started:   {}", started);
    println!();
    println!(
        "{}",
        "Watch with: hex substrate swap-list  |  hex substrate swap-samples <ticket>".dimmed()
    );
    Ok(())
}

async fn swap_list(nexus: &NexusClient) -> anyhow::Result<()> {
    let resp = nexus.get("/api/swaps").await?;
    if let Some(warning) = resp.get("warning").and_then(|v| v.as_str()) {
        println!("{} {}", "warning:".yellow(), warning);
    }
    let empty = vec![];
    let tickets = resp.get("tickets").and_then(|v| v.as_array()).unwrap_or(&empty);
    if tickets.is_empty() {
        println!("{}", "(no in-flight swap tickets)".dimmed());
        return Ok(());
    }
    for t in tickets {
        let id = t.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let port = t.get("port_id").and_then(|v| v.as_str()).unwrap_or("?");
        let inc = t.get("incumbent_adapter_id").and_then(|v| v.as_str()).unwrap_or("");
        let cand = t.get("candidate_adapter_id").and_then(|v| v.as_str()).unwrap_or("?");
        let state = t.get("state").and_then(|v| v.as_str()).unwrap_or("?");
        let frac = t.get("shadow_traffic_fraction").and_then(|v| v.as_f64()).unwrap_or(0.0);
        println!(
            "  {}  {}  {} → {}  {}  {:.0}%",
            id.cyan(),
            port,
            if inc.is_empty() { "(none)".dimmed().to_string() } else { inc.to_string() },
            cand.cyan(),
            state.blue(),
            frac * 100.0,
        );
    }
    Ok(())
}

async fn swap_samples(nexus: &NexusClient, ticket_id: &str) -> anyhow::Result<()> {
    let path = format!("/api/swaps/{}/samples", ticket_id);
    let resp = nexus.get(&path).await?;
    let empty = vec![];
    let samples = resp.get("samples").and_then(|v| v.as_array()).unwrap_or(&empty);
    if samples.is_empty() {
        println!("{}", "(no samples recorded)".dimmed());
        return Ok(());
    }
    for s in samples {
        let seq = s.get("call_seq").and_then(|v| v.as_u64()).unwrap_or(0);
        let agreed = s.get("agreed").and_then(|v| v.as_bool()).unwrap_or(false);
        let reason = s.get("reason").and_then(|v| v.as_str()).unwrap_or("");
        let badge = if agreed { "✓".green() } else { "✗".red() };
        println!(
            "  #{:<4} {}  {}",
            seq,
            badge,
            if reason.is_empty() { "" } else { reason }
        );
    }
    Ok(())
}

// ── L6 quarterly ritual (ADR-2026-04-26-1500 C6 / P9) ───────────────────────

#[derive(Debug, Clone)]
struct ParsedGoal {
    id: String,
    name: String,
    stated: String,
    test: String,
}

#[derive(Debug)]
enum GoalVerdict {
    Passes(String),
    Drifts(String),
    NoData(String),
    HumanReview(String),
}

async fn l6_ritual(nexus: &NexusClient, goals_path: &str) -> anyhow::Result<()> {
    let raw = std::fs::read_to_string(goals_path)
        .map_err(|e| anyhow::anyhow!("could not read {}: {}", goals_path, e))?;
    let goals = parse_founding_goals(&raw);
    if goals.is_empty() {
        return Err(anyhow::anyhow!(
            "no goals parsed from {} — expected `## G<n> — <name>` headings with **Stated:** and **Test:** lines",
            goals_path
        ));
    }

    println!("{}", "Layer 6 founding-goals ritual".bold());
    println!("  source: {}", goals_path.dimmed());
    println!("  goals:  {}", goals.len());
    println!();

    let mut overall_pass = true;
    for goal in &goals {
        println!("{} — {}", goal.id.cyan().bold(), goal.name);
        println!("  stated: {}", goal.stated.dimmed());
        println!("  test:   {}", goal.test);
        let verdict = evaluate_goal(nexus, goal).await;
        match verdict {
            GoalVerdict::Passes(detail) => {
                println!("  {} {}", "✓ passes".green().bold(), detail);
            }
            GoalVerdict::Drifts(reason) => {
                println!("  {} {}", "✗ drifts".red().bold(), reason);
                overall_pass = false;
            }
            GoalVerdict::NoData(reason) => {
                println!("  {} {}", "○ no data".yellow(), reason);
            }
            GoalVerdict::HumanReview(prompt) => {
                println!("  {} {}", "? human review".blue(), prompt);
            }
        }
        println!();
    }

    if overall_pass {
        println!("{}", "OK — no automated drift detected".green().bold());
    } else {
        println!("{}", "ATTENTION — at least one goal flagged drift".red().bold());
        println!("{}", "Either land a corrective swap (substrate) or open a Retirement-ADR per the goal's Retirement field.".dimmed());
        std::process::exit(1);
    }
    Ok(())
}

fn parse_founding_goals(raw: &str) -> Vec<ParsedGoal> {
    let mut goals = vec![];
    let mut current: Option<ParsedGoal> = None;
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("## ") {
            // Push the previous goal if it had both stated + test.
            if let Some(prev) = current.take() {
                if !prev.stated.is_empty() && !prev.test.is_empty() {
                    goals.push(prev);
                }
            }
            // Heading shape: `G1 — Model tiering and independence`
            let (id, name) = match rest.split_once(" — ") {
                Some((id, name)) => (id.trim().to_string(), name.trim().to_string()),
                None => (rest.to_string(), String::new()),
            };
            if id.starts_with('G') {
                current = Some(ParsedGoal {
                    id,
                    name,
                    stated: String::new(),
                    test: String::new(),
                });
            }
        } else if let Some(goal) = current.as_mut() {
            if let Some(rest) = trimmed.strip_prefix("**Stated:**") {
                goal.stated = rest.trim().to_string();
            } else if let Some(rest) = trimmed.strip_prefix("**Test:**") {
                goal.test = rest.trim().to_string();
            }
        }
    }
    if let Some(prev) = current {
        if !prev.stated.is_empty() && !prev.test.is_empty() {
            goals.push(prev);
        }
    }
    goals
}

async fn evaluate_goal(nexus: &NexusClient, goal: &ParsedGoal) -> GoalVerdict {
    // Goal-test interpretation is per-goal. Three families today:
    //   1. Substrate-internal — needs swap data; reports NoData when the
    //      substrate hasn't shipped any swaps yet.
    //   2. External-tool — runs a checker (e.g. `hex analyze`).
    //   3. Human-review — the test is a stewardship question, not
    //      automatable; report as HumanReview so the operator answers
    //      manually.
    let test_lower = goal.test.to_lowercase();
    if test_lower.contains("hex analyze") {
        // G3-shaped: workspace-level architecture check.
        match std::process::Command::new("hex").args(["analyze", "."]).output() {
            Ok(out) if out.status.success() => {
                let summary = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .find(|l| l.contains("Architecture grade"))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| "hex analyze succeeded".to_string());
                GoalVerdict::Passes(summary)
            }
            Ok(out) => GoalVerdict::Drifts(format!(
                "hex analyze exited {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            )),
            Err(e) => GoalVerdict::Drifts(format!("could not run `hex analyze`: {}", e)),
        }
    } else if test_lower.contains("swap") || test_lower.contains("composition") {
        // G1/G2-shaped: needs substrate swap-ticket data. Today the
        // substrate has not yet shipped a real swap, so report NoData with
        // a pointer to where the data WILL come from once the WASM
        // republish lands and operator-driven swaps begin flowing.
        match nexus.get("/api/swaps").await {
            Ok(body) => {
                let tickets = body.get("tickets").and_then(|v| v.as_array());
                match tickets {
                    Some(arr) if arr.is_empty() => {
                        GoalVerdict::NoData(
                            "no in-flight swap tickets — substrate hasn't shipped a swap in the survey window".to_string()
                        )
                    }
                    Some(arr) => GoalVerdict::Passes(format!(
                        "{} in-flight swap ticket(s) observed",
                        arr.len()
                    )),
                    None => GoalVerdict::NoData("api/swaps response had no tickets array".into()),
                }
            }
            Err(e) => GoalVerdict::NoData(format!("substrate/nexus unreachable: {}", e)),
        }
    } else {
        GoalVerdict::HumanReview(format!(
            "test is not auto-evaluable; you (the human reviewer) answer: {}",
            goal.test
        ))
    }
}

/// Parse one `kind=value` criterion form into the SuccessCriterion JSON
/// shape the judge expects. Three forms supported (matching the variants
/// of `hex_core::ports::adapter_generator::SuccessCriterion`):
///
/// - `error-rate-below=0.1`     → {"ErrorRateBelow": 0.1}
/// - `latency-p99-below-ms=200` → {"LatencyP99BelowMs": 200}
/// - `response-equivalence=0.05` (tolerance) → {"ResponseEquivalence": {"tolerance": 0.05}}
fn parse_criterion(raw: &str) -> anyhow::Result<Value> {
    let (kind, value) = raw
        .split_once('=')
        .ok_or_else(|| anyhow::anyhow!("--criterion must be 'kind=value', got '{}'", raw))?;
    match kind {
        "error-rate-below" => {
            let v: f32 = value
                .parse()
                .map_err(|e| anyhow::anyhow!("error-rate-below value '{}' not a float: {}", value, e))?;
            Ok(json!({ "ErrorRateBelow": v }))
        }
        "latency-p99-below-ms" => {
            let v: u64 = value
                .parse()
                .map_err(|e| anyhow::anyhow!("latency-p99-below-ms value '{}' not an int: {}", value, e))?;
            Ok(json!({ "LatencyP99BelowMs": v }))
        }
        "response-equivalence" => {
            let v: f64 = value
                .parse()
                .map_err(|e| anyhow::anyhow!("response-equivalence value '{}' not a float: {}", value, e))?;
            Ok(json!({ "ResponseEquivalence": { "tolerance": v } }))
        }
        other => Err(anyhow::anyhow!(
            "unknown criterion kind '{}' (expected error-rate-below | latency-p99-below-ms | response-equivalence)",
            other
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_error_rate_below() {
        assert_eq!(
            parse_criterion("error-rate-below=0.1").unwrap(),
            json!({"ErrorRateBelow": 0.1f32})
        );
    }

    #[test]
    fn parses_latency_p99() {
        assert_eq!(
            parse_criterion("latency-p99-below-ms=200").unwrap(),
            json!({"LatencyP99BelowMs": 200u64})
        );
    }

    #[test]
    fn parses_response_equivalence() {
        assert_eq!(
            parse_criterion("response-equivalence=0.05").unwrap(),
            json!({"ResponseEquivalence": {"tolerance": 0.05}})
        );
    }

    #[test]
    fn rejects_unknown_kind() {
        let err = parse_criterion("does-not-exist=1.0").unwrap_err();
        assert!(err.to_string().contains("unknown criterion kind"));
    }

    #[test]
    fn rejects_missing_equals() {
        let err = parse_criterion("error-rate-below").unwrap_err();
        assert!(err.to_string().contains("'kind=value'"));
    }

    #[test]
    fn parses_three_goals_from_real_founding_goals_format() {
        let raw = r#"# Founding Goals

## G1 — Model tiering and independence
**Stated:** 2026-04-26 by gary
**Why:** Hex was started because…
**Test:** Layer 6 surveys the previous quarter's IModelProvider swap tickets.
**Retirement:** Only by a human commit.

## G2 — Multi-host scaleout
**Stated:** 2026-04-26 by gary
**Why:** A self-modifying substrate that runs only on the developer's laptop is a toy.
**Test:** Layer 6 confirms that at least one composition swap was decided by placement policy.
**Retirement:** Only by a human commit.

## G3 — Hexagonal rigor at the workspace level
**Stated:** 2026-04-26 by gary
**Why:** Hexagonal rules were enforced inside src/core/ but every crate at the workspace level pulled tokio.
**Test:** Layer 6 runs `hex analyze --workspace`.
**Retirement:** Only by a human commit.
"#;
        let goals = parse_founding_goals(raw);
        assert_eq!(goals.len(), 3);
        assert_eq!(goals[0].id, "G1");
        assert_eq!(goals[0].name, "Model tiering and independence");
        assert!(goals[0].test.contains("IModelProvider swap tickets"));
        assert_eq!(goals[1].id, "G2");
        assert!(goals[1].test.contains("placement policy"));
        assert_eq!(goals[2].id, "G3");
        assert!(goals[2].test.contains("hex analyze"));
    }

    #[test]
    fn parser_skips_goals_missing_stated_or_test() {
        let raw = r#"## G1 — Has both
**Stated:** 2026-04-26
**Test:** does the thing

## G2 — Missing test
**Stated:** 2026-04-26

## G3 — Missing stated
**Test:** runs analyze
"#;
        let goals = parse_founding_goals(raw);
        assert_eq!(goals.len(), 1);
        assert_eq!(goals[0].id, "G1");
    }
}
