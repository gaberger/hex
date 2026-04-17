use spacetimedb::{reducer, table, ReducerContext, Table};

#[table(name = rl_experience, public)]
#[derive(Clone, Debug)]
pub struct RlExperience {
    #[unique]
    pub id: String,
    pub state_key: String,
    pub action: String,
    pub reward: f64,
    pub next_state_key: String,
    pub task_type: String,
    pub timestamp: String,
}

#[table(name = rl_q_entry, public)]
#[derive(Clone, Debug)]
pub struct RlQEntry {
    #[unique]
    pub composite_id: String, // "{state_key}::{action}"
    pub state_key: String,
    pub action: String,
    pub q_value: f64,
    pub visit_count: u32,
    pub last_updated: String,
}

#[table(name = rl_pattern, public)]
#[derive(Clone, Debug)]
pub struct RlPattern {
    #[unique]
    pub id: String,
    pub category: String,
    pub content: String,
    pub confidence: f64,
    pub created_at: String,
    pub last_accessed: String,
    pub access_count: u32,
    pub decay_rate: f64,
}

#[table(name = rl_last_action, public)]
#[derive(Clone, Debug)]
pub struct RlLastAction {
    #[unique]
    pub id: String,
    pub state_key: String,
    pub action: String,
}

const EPSILON: f64 = 0.1;
const LEARNING_RATE: f64 = 0.1;
const DISCOUNT_FACTOR: f64 = 0.95;

/// Known model actions with progressive tiers (ADR-2604102200).
/// Local models tried first, escalates to cloud on failure.
const MODEL_ACTIONS: &[(&str, f64)] = &[
    // Tier 1: Local fast models (try first)
    ("model:nemotron-mini", 0.3),
    ("model:qwen3:4b", 0.25),
    // Tier 2: Local medium models
    ("model:qwen3:8b", 0.35),
    ("model:qwen3.5:9b", 0.4),
    // Tier 3: Local coding models
    ("model:qwen2.5-coder:32b", 0.5),
    ("model:devstral-small-2:24b", 0.45),
    // Tier 4: Cloud fallback
    ("model:sonnet", 0.5),
    ("model:haiku", 0.3),
    ("model:opus", 0.4),
    ("model:minimax", 0.35),
    ("model:minimax_fast", 0.3),
];

/// Reward increment for successful local model execution.
/// Encourages self-improvement by using local over cloud.
const LOCAL_SUCCESS_BONUS: f64 = 0.1;

/// Penalty for rate-limiting (encourages fallback to alternate models).
const RATE_LIMIT_PENALTY: f64 = -0.5;

/// Maximum number of distinct OpenRouter model entries per state_key.
const MAX_OPENROUTER_ENTRIES_PER_STATE: usize = 50;

/// Exploration bonus added to OpenRouter models with fewer than this many visits.
const OPENROUTER_EXPLORATION_THRESHOLD: u32 = 5;

/// Exploration bonus magnitude for under-observed OpenRouter models.
const OPENROUTER_EXPLORATION_BONUS: f64 = 0.15;

/// Number of days after which unused OpenRouter Q-entries can be pruned.
/// Referenced by callers of `prune_stale_openrouter` to compute the cutoff timestamp.
#[allow(dead_code)]
const OPENROUTER_STALE_DAYS: u64 = 30;

/// Small boost given to alternative models when one is rate-limited.
const RATE_LIMIT_ALT_BOOST: f64 = 0.1;

/// Returns true if `action` is a model selection action.
fn is_model_action(action: &str) -> bool {
    action.starts_with("model:")
}

/// Returns true if `action` is an OpenRouter model action.
fn is_openrouter_action(action: &str) -> bool {
    action.starts_with("model:openrouter:")
}

/// Returns true if `action` is a local model (Ollama) action.
/// These get bonus reward to encourage self-improvement (ADR-2604102200).
fn is_local_model_action(action: &str) -> bool {
    // Local models: no slash, no openrouter prefix
    if !action.starts_with("model:") {
        return false;
    }
    let model = action.trim_start_matches("model:");
    // Contains slash = cloud model, or contains openrouter = cloud
    !model.contains('/') && !model.starts_with("openrouter")
}

/// Returns true if `action` is a context strategy action.
fn is_context_action(action: &str) -> bool {
    action.starts_with("context:")
}

/// Find the best action (highest Q-value) among entries matching a predicate.
/// Returns the action string or None if no entries match.
fn best_action_matching(entries: &[RlQEntry], predicate: fn(&str) -> bool) -> Option<String> {
    entries
        .iter()
        .filter(|e| predicate(&e.action))
        .max_by(|a, b| {
            let a_effective = effective_q_value(a);
            let b_effective = effective_q_value(b);
            a_effective
                .partial_cmp(&b_effective)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|e| e.action.clone())
}

/// Compute effective Q-value with exploration bonus for under-observed OpenRouter models.
fn effective_q_value(entry: &RlQEntry) -> f64 {
    let mut q = entry.q_value;
    if is_openrouter_action(&entry.action) && entry.visit_count < OPENROUTER_EXPLORATION_THRESHOLD {
        q += OPENROUTER_EXPLORATION_BONUS;
    }
    q
}

/// Select a compound action (model + context strategy) for the given state.
///
/// Returns a pipe-separated compound action via log, e.g. "model:sonnet|context:balanced".
/// If no Q-values exist for model actions, defaults to "model:sonnet".
/// If no Q-values exist for context actions, logs exploration needed.
#[reducer]
pub fn select_action(ctx: &ReducerContext, state_key: String) -> Result<(), String> {
    let entries: Vec<RlQEntry> = ctx
        .db
        .rl_q_entry()
        .iter()
        .filter(|e| e.state_key == state_key)
        .collect();

    // Select best model action (default to model:sonnet if none found)
    let model_action = best_action_matching(&entries, is_model_action)
        .unwrap_or_else(|| "model:sonnet".to_string());

    // Select best context action
    let context_action = best_action_matching(&entries, is_context_action);

    let result = match context_action {
        Some(ctx_action) => {
            format!("{}|{}", model_action, ctx_action)
        }
        None => model_action,
    };

    log::info!(
        "Selected action '{}' for state '{}' (epsilon={:.2})",
        result,
        state_key,
        EPSILON
    );

    // Write to singleton table for caller to read (timestamp not needed)
    ctx.db.rl_last_action().insert(RlLastAction {
        id: "last".to_string(),
        state_key: state_key,
        action: result,
    });

    Ok(())
}

/// Record a reward for an action (simple or compound).
///
/// Supports compound actions in the format "model:X|context:Y".
/// When a compound action is provided, Q-values are updated for both
/// the individual model action and the individual context action.
///
/// If `rate_limited` is true, an additional -0.5 penalty is applied to
/// the model component of the action.
///
/// If `openrouter_cost_usd` > 0.0 and the model is an OpenRouter model,
/// the actual cost is used to compute a cost-efficiency reward adjustment
/// instead of the default token-based estimate.
#[reducer]
pub fn record_reward(
    ctx: &ReducerContext,
    state_key: String,
    action: String,
    reward: f64,
    next_state_key: String,
    rate_limited: bool,
    openrouter_cost_usd: f64,
) -> Result<(), String> {
    // Parse compound action into components
    let components: Vec<&str> = action.split('|').collect();

    for component in &components {
        let component = component.trim();
        let mut effective_reward = reward;

        // Apply rate-limit penalty to model actions
        if rate_limited && is_model_action(component) {
            effective_reward += RATE_LIMIT_PENALTY;
        }

        // Reward local models to encourage self-improvement (ADR-2604102200)
        if is_local_model_action(component) && reward > 0.0 {
            effective_reward += LOCAL_SUCCESS_BONUS;
            log::info!(
                "Local model bonus for '{}': +{:.2} reward",
                component,
                LOCAL_SUCCESS_BONUS
            );
        }

        // For OpenRouter models with actual cost data, adjust reward based on
        // real cost-efficiency instead of estimated token cost.
        // Cost < $0.01 is very cheap → bonus; > $0.10 is expensive → penalty.
        if openrouter_cost_usd > 0.0 && is_openrouter_action(component) {
            let cost_adjustment = if openrouter_cost_usd < 0.01 {
                0.2 // very cost-efficient
            } else if openrouter_cost_usd < 0.05 {
                0.0 // neutral
            } else if openrouter_cost_usd < 0.10 {
                -0.1 // moderately expensive
            } else {
                -0.3 // expensive
            };
            effective_reward += cost_adjustment;
            log::info!(
                "OpenRouter cost adjustment for '{}': ${:.4} -> reward delta {:.2}",
                component,
                openrouter_cost_usd,
                cost_adjustment
            );
        }

        update_q_value(
            ctx,
            &state_key,
            component,
            effective_reward,
            &next_state_key,
        );
    }

    // Enforce cap on OpenRouter entries per state to prevent unbounded growth
    enforce_openrouter_cap(ctx, &state_key);

    // Store the full experience with the original compound action.
    let exp_id = format!("exp-{}-{}-{}", state_key, action, reward);
    ctx.db.rl_experience().insert(RlExperience {
        id: exp_id,
        state_key,
        action,
        reward,
        next_state_key,
        task_type: String::new(),
        timestamp: String::new(),
    });

    Ok(())
}

/// Enforce a cap of MAX_OPENROUTER_ENTRIES_PER_STATE OpenRouter model entries per state.
/// When the cap is exceeded, remove the entry with the lowest visit_count (least observed).
fn enforce_openrouter_cap(ctx: &ReducerContext, state_key: &str) {
    let mut or_entries: Vec<RlQEntry> = ctx
        .db
        .rl_q_entry()
        .iter()
        .filter(|e| e.state_key == state_key && is_openrouter_action(&e.action))
        .collect();

    if or_entries.len() <= MAX_OPENROUTER_ENTRIES_PER_STATE {
        return;
    }

    // Sort by visit_count ascending — prune least-observed first
    or_entries.sort_by_key(|e| e.visit_count);

    let to_remove = or_entries.len() - MAX_OPENROUTER_ENTRIES_PER_STATE;
    for entry in or_entries.iter().take(to_remove) {
        ctx.db
            .rl_q_entry()
            .composite_id()
            .delete(&entry.composite_id);
        log::info!(
            "Pruned low-visit OpenRouter Q-entry '{}' (visits: {})",
            entry.action,
            entry.visit_count
        );
    }
}

/// Internal helper: update or insert a Q-value for a single (state, action) pair.
fn update_q_value(
    ctx: &ReducerContext,
    state_key: &str,
    action: &str,
    reward: f64,
    next_state_key: &str,
) {
    let composite_id = format!("{}::{}", state_key, action);

    // Find max Q-value for next state
    let max_next_q: f64 = ctx
        .db
        .rl_q_entry()
        .iter()
        .filter(|e| e.state_key == next_state_key)
        .map(|e| e.q_value)
        .fold(0.0_f64, f64::max);

    let existing = ctx.db.rl_q_entry().composite_id().find(&composite_id);

    match existing {
        Some(old) => {
            let new_q =
                old.q_value + LEARNING_RATE * (reward + DISCOUNT_FACTOR * max_next_q - old.q_value);
            let updated = RlQEntry {
                composite_id: composite_id.clone(),
                state_key: state_key.to_string(),
                action: action.to_string(),
                q_value: new_q,
                visit_count: old.visit_count + 1,
                last_updated: String::new(),
            };
            ctx.db.rl_q_entry().composite_id().update(updated);
        }
        None => {
            ctx.db.rl_q_entry().insert(RlQEntry {
                composite_id,
                state_key: state_key.to_string(),
                action: action.to_string(),
                q_value: reward,
                visit_count: 1,
                last_updated: String::new(),
            });
        }
    }
}

/// Prune OpenRouter Q-entries that have not been updated in OPENROUTER_STALE_DAYS days.
///
/// Call this periodically (e.g. daily) to prevent unbounded Q-table growth from
/// dynamic OpenRouter model IDs.
#[reducer]
pub fn prune_stale_openrouter(
    ctx: &ReducerContext,
    cutoff_timestamp: String,
) -> Result<(), String> {
    let mut pruned = 0u32;
    let entries: Vec<RlQEntry> = ctx
        .db
        .rl_q_entry()
        .iter()
        .filter(|e| is_openrouter_action(&e.action))
        .collect();

    for entry in entries {
        // If last_updated is empty or older than cutoff, prune it
        if entry.last_updated.is_empty() || entry.last_updated < cutoff_timestamp {
            ctx.db
                .rl_q_entry()
                .composite_id()
                .delete(&entry.composite_id);
            pruned += 1;
        }
    }

    log::info!(
        "Pruned {} stale OpenRouter Q-entries (cutoff: {})",
        pruned,
        cutoff_timestamp
    );
    Ok(())
}

/// Record a rate-limit event for a specific model.
///
/// Applies a -0.5 penalty to the rate-limited model's Q-value for the given state,
/// and gives a +0.1 boost to all other model actions to encourage exploration of
/// alternatives.
#[reducer]
pub fn record_rate_limit(
    ctx: &ReducerContext,
    state_key: String,
    model: String,
) -> Result<(), String> {
    // Penalize the rate-limited model
    let penalty_id = format!("{}::{}", state_key, model);
    let existing = ctx.db.rl_q_entry().composite_id().find(&penalty_id);

    match existing {
        Some(old) => {
            let updated = RlQEntry {
                q_value: old.q_value + RATE_LIMIT_PENALTY,
                visit_count: old.visit_count + 1,
                ..old
            };
            ctx.db.rl_q_entry().composite_id().update(updated);
            log::info!(
                "Rate-limit penalty applied to '{}' at state '{}': {:.4} -> {:.4}",
                model,
                state_key,
                old.q_value,
                old.q_value + RATE_LIMIT_PENALTY
            );
        }
        None => {
            // Seed with penalty so we remember this model was rate-limited
            let default_q = MODEL_ACTIONS
                .iter()
                .find(|(name, _)| *name == model)
                .map(|(_, q)| *q)
                .unwrap_or(0.3);
            ctx.db.rl_q_entry().insert(RlQEntry {
                composite_id: penalty_id,
                state_key: state_key.clone(),
                action: model.clone(),
                q_value: default_q + RATE_LIMIT_PENALTY,
                visit_count: 1,
                last_updated: String::new(),
            });
            log::info!(
                "Rate-limit penalty applied to new '{}' at state '{}': {:.4}",
                model,
                state_key,
                default_q + RATE_LIMIT_PENALTY
            );
        }
    }

    // Boost alternative model actions (both fixed and known OpenRouter entries)
    for (alt_model, default_q) in MODEL_ACTIONS {
        if *alt_model == model {
            continue;
        }
        let alt_id = format!("{}::{}", state_key, alt_model);
        let alt_existing = ctx.db.rl_q_entry().composite_id().find(&alt_id);

        match alt_existing {
            Some(old) => {
                let updated = RlQEntry {
                    q_value: old.q_value + RATE_LIMIT_ALT_BOOST,
                    ..old
                };
                ctx.db.rl_q_entry().composite_id().update(updated);
            }
            None => {
                ctx.db.rl_q_entry().insert(RlQEntry {
                    composite_id: alt_id,
                    state_key: state_key.clone(),
                    action: alt_model.to_string(),
                    q_value: default_q + RATE_LIMIT_ALT_BOOST,
                    visit_count: 0,
                    last_updated: String::new(),
                });
            }
        }
    }

    // Also boost existing OpenRouter entries for this state (they're dynamic, so
    // we can't seed them, but we can boost ones we've already seen)
    let or_entries: Vec<RlQEntry> = ctx
        .db
        .rl_q_entry()
        .iter()
        .filter(|e| {
            e.state_key == state_key && is_openrouter_action(&e.action) && e.action != model
        })
        .collect();
    for entry in or_entries {
        let updated = RlQEntry {
            q_value: entry.q_value + RATE_LIMIT_ALT_BOOST,
            ..entry
        };
        ctx.db.rl_q_entry().composite_id().update(updated);
    }

    Ok(())
}

/// Seed initial Q-values for all model actions at a given state.
///
/// Only inserts entries that do not already exist, preserving any
/// learned Q-values.
#[reducer]
pub fn seed_model_q_values(ctx: &ReducerContext, state_key: String) -> Result<(), String> {
    let mut seeded = 0u32;
    for (model, default_q) in MODEL_ACTIONS {
        let composite_id = format!("{}::{}", state_key, model);
        if ctx
            .db
            .rl_q_entry()
            .composite_id()
            .find(&composite_id)
            .is_none()
        {
            ctx.db.rl_q_entry().insert(RlQEntry {
                composite_id,
                state_key: state_key.clone(),
                action: model.to_string(),
                q_value: *default_q,
                visit_count: 0,
                last_updated: String::new(),
            });
            seeded += 1;
        }
    }
    log::info!("Seeded {} model Q-values for state '{}'", seeded, state_key);
    Ok(())
}

#[reducer]
pub fn store_pattern(
    ctx: &ReducerContext,
    category: String,
    content: String,
    confidence: f64,
) -> Result<(), String> {
    let id = format!("pat-{}-{}", category, ctx.db.rl_pattern().count());
    ctx.db.rl_pattern().insert(RlPattern {
        id,
        category,
        content,
        confidence,
        created_at: String::new(),
        last_accessed: String::new(),
        access_count: 0,
        decay_rate: 0.01,
    });
    Ok(())
}

#[reducer]
pub fn decay_patterns(ctx: &ReducerContext) -> Result<(), String> {
    let patterns: Vec<RlPattern> = ctx.db.rl_pattern().iter().collect();
    for pattern in patterns {
        let new_confidence = pattern.confidence * (1.0 - pattern.decay_rate);
        if new_confidence < 0.01 {
            ctx.db.rl_pattern().id().delete(&pattern.id);
            log::info!("Removed decayed pattern '{}'", pattern.id);
        } else {
            let updated = RlPattern {
                confidence: new_confidence,
                ..pattern
            };
            ctx.db.rl_pattern().id().update(updated);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_model_action() {
        assert!(is_model_action("model:sonnet"));
        assert!(is_model_action("model:opus"));
        assert!(is_model_action("model:haiku"));
        assert!(is_model_action("model:local"));
        assert!(is_model_action(
            "model:openrouter:meta-llama/llama-4-maverick"
        ));
        assert!(!is_model_action("context:balanced"));
        assert!(!is_model_action("something_else"));
    }

    #[test]
    fn test_is_context_action() {
        assert!(is_context_action("context:aggressive"));
        assert!(is_context_action("context:balanced"));
        assert!(is_context_action("context:conservative"));
        assert!(!is_context_action("model:sonnet"));
        assert!(!is_context_action("something_else"));
    }

    #[test]
    fn test_is_openrouter_action() {
        assert!(is_openrouter_action(
            "model:openrouter:meta-llama/llama-4-maverick"
        ));
        assert!(is_openrouter_action(
            "model:openrouter:deepseek/deepseek-r1"
        ));
        assert!(!is_openrouter_action("model:sonnet"));
        assert!(!is_openrouter_action("model:opus"));
        assert!(!is_openrouter_action("context:balanced"));
    }

    #[test]
    fn test_best_action_matching_empty() {
        let entries: Vec<RlQEntry> = vec![];
        assert_eq!(best_action_matching(&entries, is_model_action), None);
    }

    #[test]
    fn test_best_action_matching_picks_highest_q() {
        let entries = vec![
            RlQEntry {
                composite_id: "s1::model:sonnet".to_string(),
                state_key: "s1".to_string(),
                action: "model:sonnet".to_string(),
                q_value: 0.5,
                visit_count: 1,
                last_updated: String::new(),
            },
            RlQEntry {
                composite_id: "s1::model:opus".to_string(),
                state_key: "s1".to_string(),
                action: "model:opus".to_string(),
                q_value: 0.9,
                visit_count: 1,
                last_updated: String::new(),
            },
            RlQEntry {
                composite_id: "s1::context:balanced".to_string(),
                state_key: "s1".to_string(),
                action: "context:balanced".to_string(),
                q_value: 0.99,
                visit_count: 1,
                last_updated: String::new(),
            },
        ];
        // Should pick opus (0.9) among model actions, ignoring context (0.99)
        assert_eq!(
            best_action_matching(&entries, is_model_action),
            Some("model:opus".to_string())
        );
        // Should pick context:balanced (0.99) among context actions
        assert_eq!(
            best_action_matching(&entries, is_context_action),
            Some("context:balanced".to_string())
        );
    }

    #[test]
    fn test_model_actions_seed_values() {
        // Verify the seed defaults match the MODEL_ACTIONS constant
        let map: std::collections::HashMap<&str, f64> = MODEL_ACTIONS.iter().copied().collect();
        // Cloud models
        assert_eq!(map["model:sonnet"], 0.5);
        assert_eq!(map["model:haiku"], 0.3);
        assert_eq!(map["model:opus"], 0.4);
        assert_eq!(map["model:minimax"], 0.35);
        assert_eq!(map["model:minimax_fast"], 0.3);
        // Local models (tiered by capability)
        assert_eq!(map["model:nemotron-mini"], 0.3);
        assert_eq!(map["model:qwen3:4b"], 0.25);
        assert_eq!(map["model:qwen3:8b"], 0.35);
        assert_eq!(map["model:qwen3.5:9b"], 0.4);
        assert_eq!(map["model:qwen2.5-coder:32b"], 0.5);
        assert_eq!(map["model:devstral-small-2:24b"], 0.45);
    }

    #[test]
    fn test_exploration_bonus_for_new_openrouter() {
        let entry_new = RlQEntry {
            composite_id: "s1::model:openrouter:test/model".to_string(),
            state_key: "s1".to_string(),
            action: "model:openrouter:test/model".to_string(),
            q_value: 0.3,
            visit_count: 2, // below threshold of 5
            last_updated: String::new(),
        };
        let entry_mature = RlQEntry {
            composite_id: "s1::model:openrouter:test/model2".to_string(),
            state_key: "s1".to_string(),
            action: "model:openrouter:test/model2".to_string(),
            q_value: 0.3,
            visit_count: 10, // above threshold
            last_updated: String::new(),
        };
        // New OpenRouter model gets exploration bonus
        assert!((effective_q_value(&entry_new) - 0.45).abs() < 0.001);
        // Mature OpenRouter model does not
        assert!((effective_q_value(&entry_mature) - 0.3).abs() < 0.001);
    }

    #[test]
    fn test_no_exploration_bonus_for_non_openrouter() {
        let entry = RlQEntry {
            composite_id: "s1::model:sonnet".to_string(),
            state_key: "s1".to_string(),
            action: "model:sonnet".to_string(),
            q_value: 0.5,
            visit_count: 1, // low visits, but not OpenRouter
            last_updated: String::new(),
        };
        // No bonus for non-OpenRouter models
        assert!((effective_q_value(&entry) - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_compound_action_parsing() {
        let action = "model:sonnet|context:balanced";
        let components: Vec<&str> = action.split('|').collect();
        assert_eq!(components.len(), 2);
        assert_eq!(components[0], "model:sonnet");
        assert_eq!(components[1], "context:balanced");
        assert!(is_model_action(components[0]));
        assert!(is_context_action(components[1]));
    }

    #[test]
    fn test_single_action_parsing() {
        let action = "context:aggressive";
        let components: Vec<&str> = action.split('|').collect();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0], "context:aggressive");
    }

    #[test]
    fn test_rate_limit_constants() {
        assert_eq!(RATE_LIMIT_PENALTY, -0.5);
        assert_eq!(RATE_LIMIT_ALT_BOOST, 0.1);
    }
}
