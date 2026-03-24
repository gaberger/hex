//! Neural Lab — SpacetimeDB WASM module for neural network architecture search.
//!
//! Tracks network configurations, experiment lifecycle, research frontiers,
//! and mutation strategy learning for evolutionary neural architecture search.
//!
//! Tables:
//!   - `network_config` — architecture hyperparameters
//!   - `layer_spec` — per-layer attention/residual configuration
//!   - `experiment` — training runs with metrics
//!   - `research_frontier` — best-known configs per lineage
//!   - `mutation_strategy` — UCB1-weighted mutation selection

#![allow(clippy::too_many_arguments, clippy::needless_borrows_for_generic_args)]

use spacetimedb::{reducer, table, ReducerContext, ScheduleAt, Table};

// ============================================================
//  Tables
// ============================================================

#[table(name = network_config, public)]
#[derive(Clone, Debug)]
pub struct NetworkConfig {
    #[unique]
    pub id: String,
    pub name: String,
    pub parent_id: String,
    pub n_layer: u32,
    pub n_head: u32,
    pub n_kv_head: u32,
    pub n_embd: u32,
    pub vocab_size: u32,
    pub sequence_len: u32,
    pub window_pattern: String,
    pub activation: String,
    /// JSON-encoded optimizer configuration
    pub optimizer_config: String,
    pub total_batch_size: u32,
    pub time_budget_secs: u32,
    pub created_at: String,
    pub created_by: String,
    /// "candidate", "active", "archived"
    pub status: String,
}

#[table(name = layer_spec, public)]
#[derive(Clone, Debug)]
pub struct LayerSpec {
    #[unique]
    pub id: String,
    pub config_id: String,
    pub layer_index: u32,
    /// "sliding_window", "global", "local", "linear"
    pub attention_type: String,
    pub window_size: u32,
    /// "true" or "false"
    pub use_value_embeddings: String,
    /// Float stored as string
    pub resid_lambda: String,
    /// Float stored as string
    pub x0_lambda: String,
}

#[table(name = experiment, public)]
#[derive(Clone, Debug)]
pub struct Experiment {
    #[unique]
    pub id: String,
    pub config_id: String,
    pub swarm_id: String,
    pub hypothesis: String,
    /// JSON-encoded diff describing the mutation
    pub mutation_diff: String,
    /// "queued", "training", "kept", "discarded", "failed"
    pub status: String,
    /// Float stored as string
    pub val_bpb: String,
    /// Float stored as string
    pub baseline_bpb: String,
    /// Float stored as string (val_bpb - baseline_bpb, negative = improvement)
    pub improvement_bpb: String,
    /// Float stored as string
    pub train_loss_final: String,
    pub tokens_processed: u64,
    pub wall_time_secs: u32,
    pub gpu_node_id: String,
    pub git_branch: String,
    pub git_commit: String,
    pub started_at: String,
    pub completed_at: String,
    pub error_message: String,
    pub lineage_name: String,
}

#[table(name = research_frontier, public)]
#[derive(Clone, Debug)]
pub struct ResearchFrontier {
    #[unique]
    pub id: String,
    pub lineage_name: String,
    pub best_config_id: String,
    pub best_experiment_id: String,
    /// Float stored as string
    pub best_val_bpb: String,
    pub total_experiments: u32,
    pub total_kept: u32,
    pub total_discarded: u32,
    pub updated_at: String,
}

#[table(name = mutation_strategy, public)]
#[derive(Clone, Debug)]
pub struct MutationStrategy {
    #[unique]
    pub id: String,
    pub strategy_name: String,
    /// Float stored as string (selection probability weight)
    pub selection_weight: String,
    pub total_tried: u32,
    pub total_kept: u32,
    /// Float stored as string
    pub success_rate: String,
    pub last_used_at: String,
}

/// Scheduler anchor for periodic research loop tick.
#[table(name = neural_lab_schedule, public)]
pub struct NeuralLabSchedule {
    #[primary_key]
    pub id: u64,
    pub scheduled_id: ScheduleAt,
    pub interval_secs: u64,
}

// ============================================================
//  Constants
// ============================================================

/// Minimum queue depth per lineage before auto-generating mutations.
const MIN_QUEUE_DEPTH: usize = 3;

/// Exploration bonus for UCB1-like strategy selection.
const EXPLORATION_BONUS: f64 = 1.0;

/// Default mutation strategies seeded on init.
const DEFAULT_STRATEGIES: &[&str] = &[
    "widen",
    "deepen",
    "attention",
    "optimizer",
    "activation",
    "random",
];

// ============================================================
//  Helpers
// ============================================================

fn generate_id(ctx: &ReducerContext) -> String {
    // Use the debug representation of timestamp as a unique seed
    let ts = format!("{:?}", ctx.timestamp);
    let hash: u64 = ts.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    format!("{:016x}", hash)
}

fn now_str(ctx: &ReducerContext) -> String {
    ctx.timestamp.to_string()
}

// ============================================================
//  Step 2: Config CRUD Reducers
// ============================================================

/// Create a new network configuration.
///
/// Validates: n_layer > 0, n_embd > 0, vocab_size >= 256, time_budget_secs >= 60.
#[reducer]
pub fn config_create(
    ctx: &ReducerContext,
    name: String,
    parent_id: String,
    n_layer: u32,
    n_head: u32,
    n_kv_head: u32,
    n_embd: u32,
    vocab_size: u32,
    sequence_len: u32,
    window_pattern: String,
    activation: String,
    optimizer_config: String,
    total_batch_size: u32,
    time_budget_secs: u32,
    created_by: String,
) -> Result<(), String> {
    // Validation
    if n_layer == 0 {
        return Err("n_layer must be > 0".to_string());
    }
    if n_embd == 0 {
        return Err("n_embd must be > 0".to_string());
    }
    if vocab_size < 256 {
        return Err("vocab_size must be >= 256".to_string());
    }
    if time_budget_secs < 60 {
        return Err("time_budget_secs must be >= 60".to_string());
    }

    let id = generate_id(ctx);
    let created_at = now_str(ctx);

    ctx.db.network_config().insert(NetworkConfig {
        id: id.clone(),
        name: name.clone(),
        parent_id,
        n_layer,
        n_head,
        n_kv_head,
        n_embd,
        vocab_size,
        sequence_len,
        window_pattern,
        activation,
        optimizer_config,
        total_batch_size,
        time_budget_secs,
        created_at,
        created_by,
        status: "candidate".to_string(),
    });

    log::info!("Config created: {} ({})", name, id);
    Ok(())
}

/// Activate a config — set status to "active".
#[reducer]
pub fn config_activate(ctx: &ReducerContext, config_id: String) -> Result<(), String> {
    let config = ctx
        .db
        .network_config()
        .id()
        .find(&config_id)
        .ok_or_else(|| format!("Config '{}' not found", config_id))?;

    ctx.db.network_config().id().update(NetworkConfig {
        status: "active".to_string(),
        ..config
    });

    log::info!("Config activated: {}", config_id);
    Ok(())
}

/// Archive a config — set status to "archived".
#[reducer]
pub fn config_archive(ctx: &ReducerContext, config_id: String) -> Result<(), String> {
    let config = ctx
        .db
        .network_config()
        .id()
        .find(&config_id)
        .ok_or_else(|| format!("Config '{}' not found", config_id))?;

    ctx.db.network_config().id().update(NetworkConfig {
        status: "archived".to_string(),
        ..config
    });

    log::info!("Config archived: {}", config_id);
    Ok(())
}

/// Create a layer specification for a config.
///
/// Validates that the referenced config_id exists.
#[reducer]
pub fn layer_spec_create(
    ctx: &ReducerContext,
    config_id: String,
    layer_index: u32,
    attention_type: String,
    window_size: u32,
    use_value_embeddings: String,
    resid_lambda: String,
    x0_lambda: String,
) -> Result<(), String> {
    // Validate config exists
    if ctx.db.network_config().id().find(&config_id).is_none() {
        return Err(format!("Config '{}' not found", config_id));
    }

    let id = generate_id(ctx);

    ctx.db.layer_spec().insert(LayerSpec {
        id: id.clone(),
        config_id: config_id.clone(),
        layer_index,
        attention_type,
        window_size,
        use_value_embeddings,
        resid_lambda,
        x0_lambda,
    });

    log::info!(
        "LayerSpec created: {} (config={}, layer={})",
        id,
        config_id,
        layer_index
    );
    Ok(())
}

// ============================================================
//  Step 3: Experiment Lifecycle Reducers
// ============================================================

/// Create a new experiment (status = "queued").
///
/// Looks up ResearchFrontier for baseline_bpb (empty string if first experiment in lineage).
#[reducer]
pub fn experiment_create(
    ctx: &ReducerContext,
    config_id: String,
    hypothesis: String,
    mutation_diff: String,
    lineage_name: String,
) -> Result<(), String> {
    // Validate config exists
    if ctx.db.network_config().id().find(&config_id).is_none() {
        return Err(format!("Config '{}' not found", config_id));
    }

    // Look up baseline from research frontier
    let baseline_bpb = ctx
        .db
        .research_frontier()
        .iter()
        .find(|f| f.lineage_name == lineage_name)
        .map(|f| f.best_val_bpb.clone())
        .unwrap_or_default();

    let id = generate_id(ctx);

    ctx.db.experiment().insert(Experiment {
        id: id.clone(),
        config_id,
        swarm_id: String::new(),
        hypothesis,
        mutation_diff,
        status: "queued".to_string(),
        val_bpb: String::new(),
        baseline_bpb,
        improvement_bpb: String::new(),
        train_loss_final: String::new(),
        tokens_processed: 0,
        wall_time_secs: 0,
        gpu_node_id: String::new(),
        git_branch: String::new(),
        git_commit: String::new(),
        started_at: String::new(),
        completed_at: String::new(),
        error_message: String::new(),
        lineage_name: lineage_name.clone(),
    });

    log::info!(
        "Experiment created: {} (lineage={})",
        id,
        lineage_name
    );
    Ok(())
}

/// Start an experiment — transition from "queued" to "training".
#[reducer]
pub fn experiment_start(
    ctx: &ReducerContext,
    experiment_id: String,
    gpu_node_id: String,
) -> Result<(), String> {
    let exp = ctx
        .db
        .experiment()
        .id()
        .find(&experiment_id)
        .ok_or_else(|| format!("Experiment '{}' not found", experiment_id))?;

    if exp.status != "queued" {
        return Err(format!(
            "Experiment '{}' status is '{}', expected 'queued'",
            experiment_id, exp.status
        ));
    }

    ctx.db.experiment().id().update(Experiment {
        status: "training".to_string(),
        started_at: now_str(ctx),
        gpu_node_id,
        ..exp
    });

    log::info!("Experiment started: {}", experiment_id);
    Ok(())
}

/// Complete an experiment — compute improvement and update frontier if better.
///
/// If baseline is empty OR val_bpb < baseline_bpb, status = "kept" and frontier is updated.
/// Otherwise status = "discarded".
#[reducer]
pub fn experiment_complete(
    ctx: &ReducerContext,
    experiment_id: String,
    val_bpb: String,
    train_loss_final: String,
    tokens_processed: u64,
    wall_time_secs: u32,
    git_commit: String,
) -> Result<(), String> {
    let exp = ctx
        .db
        .experiment()
        .id()
        .find(&experiment_id)
        .ok_or_else(|| format!("Experiment '{}' not found", experiment_id))?;

    if exp.status != "training" {
        return Err(format!(
            "Experiment '{}' status is '{}', expected 'training'",
            experiment_id, exp.status
        ));
    }

    let val: f64 = val_bpb
        .parse()
        .map_err(|_| format!("Invalid val_bpb: '{}'", val_bpb))?;

    let baseline_empty = exp.baseline_bpb.is_empty();
    let baseline: f64 = if baseline_empty {
        f64::MAX
    } else {
        exp.baseline_bpb
            .parse()
            .map_err(|_| format!("Invalid baseline_bpb: '{}'", exp.baseline_bpb))?
    };

    let is_improvement = baseline_empty || val < baseline;
    let status = if is_improvement { "kept" } else { "discarded" };
    let improvement = if baseline_empty {
        String::new()
    } else {
        format!("{:.6}", val - baseline)
    };

    let now = now_str(ctx);

    ctx.db.experiment().id().update(Experiment {
        status: status.to_string(),
        val_bpb: val_bpb.clone(),
        train_loss_final,
        tokens_processed,
        wall_time_secs,
        git_commit,
        completed_at: now.clone(),
        improvement_bpb: improvement,
        ..exp.clone()
    });

    // Update frontier if kept
    if is_improvement {
        update_frontier(ctx, &exp.lineage_name, &exp.config_id, &experiment_id, &val_bpb, &now);
    }

    // Update mutation strategy stats
    update_strategy_from_experiment(ctx, &exp.mutation_diff, is_improvement);

    log::info!(
        "Experiment completed: {} status={} val_bpb={}",
        experiment_id,
        status,
        val_bpb
    );
    Ok(())
}

/// Mark an experiment as failed. Does not touch the frontier.
#[reducer]
pub fn experiment_fail(
    ctx: &ReducerContext,
    experiment_id: String,
    error_message: String,
) -> Result<(), String> {
    let exp = ctx
        .db
        .experiment()
        .id()
        .find(&experiment_id)
        .ok_or_else(|| format!("Experiment '{}' not found", experiment_id))?;

    ctx.db.experiment().id().update(Experiment {
        status: "failed".to_string(),
        error_message,
        completed_at: now_str(ctx),
        ..exp
    });

    log::info!("Experiment failed: {}", experiment_id);
    Ok(())
}

/// Internal: update or create a research frontier entry for a lineage.
fn update_frontier(
    ctx: &ReducerContext,
    lineage_name: &str,
    config_id: &str,
    experiment_id: &str,
    val_bpb: &str,
    now: &str,
) {
    let existing: Option<ResearchFrontier> = ctx
        .db
        .research_frontier()
        .iter()
        .find(|f| f.lineage_name == lineage_name);

    match existing {
        Some(frontier) => {
            ctx.db.research_frontier().id().update(ResearchFrontier {
                best_config_id: config_id.to_string(),
                best_experiment_id: experiment_id.to_string(),
                best_val_bpb: val_bpb.to_string(),
                total_experiments: frontier.total_experiments + 1,
                total_kept: frontier.total_kept + 1,
                updated_at: now.to_string(),
                ..frontier
            });
        }
        None => {
            let id = format!("frontier-{}", lineage_name);
            ctx.db.research_frontier().insert(ResearchFrontier {
                id,
                lineage_name: lineage_name.to_string(),
                best_config_id: config_id.to_string(),
                best_experiment_id: experiment_id.to_string(),
                best_val_bpb: val_bpb.to_string(),
                total_experiments: 1,
                total_kept: 1,
                total_discarded: 0,
                updated_at: now.to_string(),
            });
        }
    }
}

/// Internal: try to attribute the experiment to a mutation strategy and update stats.
fn update_strategy_from_experiment(ctx: &ReducerContext, mutation_diff: &str, kept: bool) {
    // Try to extract strategy name from mutation_diff JSON (look for "strategy" key)
    // Simple heuristic: check if mutation_diff contains any known strategy name
    let strategies: Vec<MutationStrategy> = ctx.db.mutation_strategy().iter().collect();
    for strategy in strategies {
        if mutation_diff.contains(&strategy.strategy_name) {
            let new_kept = if kept {
                strategy.total_kept + 1
            } else {
                strategy.total_kept
            };
            let new_tried = strategy.total_tried + 1;
            let new_rate = if new_tried > 0 {
                new_kept as f64 / new_tried as f64
            } else {
                0.0
            };
            ctx.db.mutation_strategy().id().update(MutationStrategy {
                total_tried: new_tried,
                total_kept: new_kept,
                success_rate: format!("{:.6}", new_rate),
                ..strategy
            });
            return;
        }
    }
}

// ============================================================
//  Step 4: Scheduled Reducers
// ============================================================

/// Initialize the neural-lab schedule. Called once on module publish.
#[reducer(init)]
pub fn init(ctx: &ReducerContext) {
    // Schedule research_loop_tick every 60 seconds
    let schedule_at = ScheduleAt::Interval(std::time::Duration::from_secs(60).into());
    ctx.db.neural_lab_schedule().insert(NeuralLabSchedule {
        id: 1,
        scheduled_id: schedule_at,
        interval_secs: 60,
    });

    // Seed default mutation strategies
    seed_strategies(ctx);

    log::info!("neural-lab: initialized, scheduled every 60s, strategies seeded");
}

/// Scheduled reducer: research loop tick.
///
/// 1. Check queue depth per lineage, log if below MIN_QUEUE_DEPTH.
/// 2. Check for timed-out experiments (training > 2x time_budget).
#[reducer]
pub fn research_loop_tick(ctx: &ReducerContext) {
    // 1. Check queue depth per lineage
    let frontiers: Vec<ResearchFrontier> = ctx.db.research_frontier().iter().collect();
    for frontier in &frontiers {
        let queued_count = ctx
            .db
            .experiment()
            .iter()
            .filter(|e| e.lineage_name == frontier.lineage_name && e.status == "queued")
            .count();

        if queued_count < MIN_QUEUE_DEPTH {
            // Auto-generate mutations from the best config
            if let Some(best_config) = ctx.db.network_config().id().find(&frontier.best_config_id) {
                let strategies = ["widen", "deepen", "attention", "optimizer", "activation"];
                let needed = MIN_QUEUE_DEPTH - queued_count;
                let ts_hash = format!("{:?}", ctx.timestamp).bytes().fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize));

                for i in 0..needed {
                    let strategy = strategies[(ts_hash + i) % strategies.len()];
                    let (hypothesis, mutation_diff) = match strategy {
                        "widen" => (
                            format!("widen: n_embd {} → {}", best_config.n_embd, best_config.n_embd + 128),
                            format!("{{\"strategy\":\"widen\",\"n_embd\":[{},{}]}}", best_config.n_embd, best_config.n_embd + 128),
                        ),
                        "deepen" => (
                            format!("deepen: n_layer {} → {}", best_config.n_layer, best_config.n_layer + 2),
                            format!("{{\"strategy\":\"deepen\",\"n_layer\":[{},{}]}}", best_config.n_layer, best_config.n_layer + 2),
                        ),
                        "attention" => {
                            let new_pattern = if best_config.window_pattern == "SSSL" { "SSLL" } else { "SSSL" };
                            (
                                format!("attention: window {} → {}", best_config.window_pattern, new_pattern),
                                format!("{{\"strategy\":\"attention\",\"window_pattern\":[\"{}\",\"{}\"]}}", best_config.window_pattern, new_pattern),
                            )
                        }
                        "optimizer" => (
                            "optimizer: adjust lr by -10%".to_string(),
                            "{\"strategy\":\"optimizer\",\"lr_adjust\":-0.1}".to_string(),
                        ),
                        _ => (
                            format!("activation: try swiglu (was {})", best_config.activation),
                            format!("{{\"strategy\":\"activation\",\"activation\":[\"{}\",\"swiglu\"]}}", best_config.activation),
                        ),
                    };

                    let exp_id = generate_id(ctx);
                    let now = now_str(ctx);
                    ctx.db.experiment().insert(Experiment {
                        id: format!("{}-{}", exp_id, i),
                        config_id: best_config.id.clone(),
                        swarm_id: String::new(),
                        hypothesis,
                        mutation_diff,
                        status: "queued".to_string(),
                        val_bpb: String::new(),
                        baseline_bpb: frontier.best_val_bpb.clone(),
                        improvement_bpb: String::new(),
                        train_loss_final: String::new(),
                        tokens_processed: 0,
                        wall_time_secs: 0,
                        gpu_node_id: String::new(),
                        git_branch: String::new(),
                        git_commit: String::new(),
                        started_at: String::new(),
                        completed_at: String::new(),
                        error_message: String::new(),
                        lineage_name: frontier.lineage_name.clone(),
                    });

                    log::info!("Auto-generated experiment for lineage '{}': {}", frontier.lineage_name, strategy);
                }
            }
        }
    }

    // 2. Check for timed-out experiments
    stale_experiment_cleanup(ctx);

    // 3. Recompute frontiers
    frontier_consolidate(ctx);
}

/// Find training experiments exceeding 2x time_budget and mark them failed.
#[reducer]
pub fn stale_experiment_cleanup(ctx: &ReducerContext) {
    let now = now_str(ctx);
    let training: Vec<Experiment> = ctx
        .db
        .experiment()
        .iter()
        .filter(|e| e.status == "training")
        .collect();

    for exp in training {
        // Look up the config to get time_budget_secs
        if let Some(config) = ctx.db.network_config().id().find(&exp.config_id) {
            // Use wall_time_secs field if available (updated by external runner),
            // otherwise skip — timeout is enforced by the external coordinator.
            if exp.wall_time_secs > config.time_budget_secs * 2 {
                    let exp_id = exp.id.clone();
                    let budget = config.time_budget_secs;
                    ctx.db.experiment().id().update(Experiment {
                        status: "failed".to_string(),
                        error_message: format!(
                            "Timed out: exceeded 2x time_budget ({}s)",
                            budget * 2
                        ),
                        completed_at: now.clone(),
                        ..exp
                    });
                    log::info!("Experiment {} timed out (budget={}s)", exp_id, budget);
                }
        }
    }
}

/// Recompute ResearchFrontier from all "kept" experiments.
#[reducer]
pub fn frontier_consolidate(ctx: &ReducerContext) {
    let frontiers: Vec<ResearchFrontier> = ctx.db.research_frontier().iter().collect();

    for frontier in frontiers {
        let lineage = &frontier.lineage_name;

        let kept: Vec<Experiment> = ctx
            .db
            .experiment()
            .iter()
            .filter(|e| e.lineage_name == *lineage && e.status == "kept")
            .collect();

        let discarded_count = ctx
            .db
            .experiment()
            .iter()
            .filter(|e| e.lineage_name == *lineage && e.status == "discarded")
            .count() as u32;

        let total_count = ctx
            .db
            .experiment()
            .iter()
            .filter(|e| e.lineage_name == *lineage)
            .count() as u32;

        // Find the best kept experiment (lowest val_bpb)
        let best = kept.iter().min_by(|a, b| {
            let a_val: f64 = a.val_bpb.parse().unwrap_or(f64::MAX);
            let b_val: f64 = b.val_bpb.parse().unwrap_or(f64::MAX);
            a_val.partial_cmp(&b_val).unwrap_or(std::cmp::Ordering::Equal)
        });

        if let Some(best_exp) = best {
            ctx.db.research_frontier().id().update(ResearchFrontier {
                best_config_id: best_exp.config_id.clone(),
                best_experiment_id: best_exp.id.clone(),
                best_val_bpb: best_exp.val_bpb.clone(),
                total_experiments: total_count,
                total_kept: kept.len() as u32,
                total_discarded: discarded_count,
                updated_at: now_str(ctx),
                ..frontier
            });
        }
    }
}

// ============================================================
//  Step 5: Mutation Strategy Learning
// ============================================================

/// Seed the 6 default mutation strategies with uniform weights.
fn seed_strategies(ctx: &ReducerContext) {
    let uniform_weight = format!("{:.6}", 1.0 / DEFAULT_STRATEGIES.len() as f64);
    let now = now_str(ctx);

    for name in DEFAULT_STRATEGIES {
        let id = format!("strategy-{}", name);
        // Only insert if not already present
        if ctx.db.mutation_strategy().id().find(&id).is_none() {
            ctx.db.mutation_strategy().insert(MutationStrategy {
                id,
                strategy_name: name.to_string(),
                selection_weight: uniform_weight.clone(),
                total_tried: 0,
                total_kept: 0,
                success_rate: "0.000000".to_string(),
                last_used_at: now.clone(),
            });
        }
    }
}

/// Manually trigger strategy seeding (idempotent).
#[reducer]
pub fn mutation_strategy_init(ctx: &ReducerContext) -> Result<(), String> {
    seed_strategies(ctx);
    log::info!("Mutation strategies seeded/verified");
    Ok(())
}

/// Recompute strategy weights using UCB1-like formula.
///
/// weight = success_rate + EXPLORATION_BONUS / (1 + total_tried)
/// Then normalize all weights to sum to 1.0.
#[reducer]
pub fn mutation_strategy_update(ctx: &ReducerContext) -> Result<(), String> {
    let strategies: Vec<MutationStrategy> = ctx.db.mutation_strategy().iter().collect();

    if strategies.is_empty() {
        return Ok(());
    }

    // Compute raw UCB1 scores
    let mut scores: Vec<(String, f64)> = Vec::new();
    for s in &strategies {
        let rate: f64 = s.success_rate.parse().unwrap_or(0.0);
        let score = rate + EXPLORATION_BONUS / (1.0 + s.total_tried as f64);
        scores.push((s.id.clone(), score));
    }

    // Normalize
    let total: f64 = scores.iter().map(|(_, s)| s).sum();
    if total <= 0.0 {
        return Ok(());
    }

    for (id, score) in &scores {
        let weight = score / total;
        if let Some(strategy) = ctx.db.mutation_strategy().id().find(id) {
            ctx.db.mutation_strategy().id().update(MutationStrategy {
                selection_weight: format!("{:.6}", weight),
                ..strategy
            });
        }
    }

    log::info!(
        "Mutation strategy weights updated ({} strategies)",
        scores.len()
    );
    Ok(())
}

// ============================================================
//  Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_strategies_count() {
        assert_eq!(DEFAULT_STRATEGIES.len(), 6);
    }

    #[test]
    fn test_ucb1_formula() {
        // success_rate=0.5, total_tried=10 -> 0.5 + 1.0/11 = 0.5909...
        let rate = 0.5_f64;
        let tried = 10_u32;
        let score = rate + EXPLORATION_BONUS / (1.0 + tried as f64);
        assert!((score - 0.590909).abs() < 0.001);
    }

    #[test]
    fn test_ucb1_exploration_dominates_for_new() {
        // success_rate=0.0, total_tried=0 -> 0.0 + 1.0/1 = 1.0
        let rate = 0.0_f64;
        let tried = 0_u32;
        let score = rate + EXPLORATION_BONUS / (1.0 + tried as f64);
        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_ucb1_normalization() {
        // 3 strategies with scores 1.0, 0.5, 0.5 -> weights 0.5, 0.25, 0.25
        let scores = vec![1.0_f64, 0.5, 0.5];
        let total: f64 = scores.iter().sum();
        let weights: Vec<f64> = scores.iter().map(|s| s / total).collect();
        assert!((weights[0] - 0.5).abs() < f64::EPSILON);
        assert!((weights[1] - 0.25).abs() < f64::EPSILON);
        assert!((weights[2] - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn test_improvement_calculation() {
        let val_bpb = 1.05_f64;
        let baseline_bpb = 1.10_f64;
        let improvement = val_bpb - baseline_bpb;
        assert!((improvement - (-0.05)).abs() < 0.001);
        // Negative improvement means the new config is better
        assert!(improvement < 0.0);
    }
}
