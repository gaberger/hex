use spacetimedb::{table, reducer, ReducerContext, Table};

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

const EPSILON: f64 = 0.1;
const LEARNING_RATE: f64 = 0.1;
const DISCOUNT_FACTOR: f64 = 0.95;

/// Known model actions with their default seed Q-values.
const MODEL_ACTIONS: &[(&str, f64)] = &[
    ("model:sonnet", 0.5),
    ("model:haiku", 0.3),
    ("model:opus", 0.4),
    ("model:local", 0.2),
];

/// Rate-limit penalty applied to the offending model action.
const RATE_LIMIT_PENALTY: f64 = -0.5;

/// Small boost given to alternative models when one is rate-limited.
const RATE_LIMIT_ALT_BOOST: f64 = 0.1;

/// Returns true if `action` is a model selection action.
fn is_model_action(action: &str) -> bool {
    action.starts_with("model:")
}

/// Returns true if `action` is a context strategy action.
fn is_context_action(action: &str) -> bool {
    action.starts_with("context:")
}

/// Find the best action (highest Q-value) among entries matching a predicate.
/// Returns the action string or None if no entries match.
fn best_action_matching(entries: &[RlQEntry], predicate: fn(&str) -> bool) -> Option<String> {
    entries.iter()
        .filter(|e| predicate(&e.action))
        .max_by(|a, b| a.q_value.partial_cmp(&b.q_value).unwrap_or(std::cmp::Ordering::Equal))
        .map(|e| e.action.clone())
}

/// Select a compound action (model + context strategy) for the given state.
///
/// Returns a pipe-separated compound action via log, e.g. "model:sonnet|context:balanced".
/// If no Q-values exist for model actions, defaults to "model:sonnet".
/// If no Q-values exist for context actions, logs exploration needed.
#[reducer]
pub fn select_action(ctx: &ReducerContext, state_key: String) -> Result<(), String> {
    let entries: Vec<RlQEntry> = ctx.db.rl_q_entry().iter()
        .filter(|e| e.state_key == state_key)
        .collect();

    // Select best model action (default to model:sonnet if none found)
    let model_action = best_action_matching(&entries, is_model_action)
        .unwrap_or_else(|| "model:sonnet".to_string());

    // Select best context action
    let context_action = best_action_matching(&entries, is_context_action);

    match context_action {
        Some(ctx_action) => {
            let compound = format!("{}|{}", model_action, ctx_action);
            log::info!(
                "Selected compound action '{}' for state '{}' (epsilon={:.2})",
                compound, state_key, EPSILON
            );
        }
        None => {
            log::info!(
                "Selected model '{}' for state '{}', context exploration needed (epsilon={:.2})",
                model_action, state_key, EPSILON
            );
        }
    }

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
#[reducer]
pub fn record_reward(
    ctx: &ReducerContext,
    state_key: String,
    action: String,
    reward: f64,
    next_state_key: String,
    rate_limited: bool,
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

        update_q_value(ctx, &state_key, component, effective_reward, &next_state_key);
    }

    // If the action was not compound (single action), it was already handled above.
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
    let max_next_q: f64 = ctx.db.rl_q_entry().iter()
        .filter(|e| e.state_key == next_state_key)
        .map(|e| e.q_value)
        .fold(0.0_f64, f64::max);

    let existing = ctx.db.rl_q_entry().composite_id().find(&composite_id);

    match existing {
        Some(old) => {
            let new_q = old.q_value + LEARNING_RATE * (reward + DISCOUNT_FACTOR * max_next_q - old.q_value);
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

/// Record a rate-limit event for a specific model.
///
/// Applies a -0.5 penalty to the rate-limited model's Q-value for the given state,
/// and gives a +0.1 boost to all other model actions to encourage exploration of
/// alternatives.
#[reducer]
pub fn record_rate_limit(ctx: &ReducerContext, state_key: String, model: String) -> Result<(), String> {
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
                model, state_key, old.q_value, old.q_value + RATE_LIMIT_PENALTY
            );
        }
        None => {
            // Seed with penalty so we remember this model was rate-limited
            let default_q = MODEL_ACTIONS.iter()
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
                model, state_key, default_q + RATE_LIMIT_PENALTY
            );
        }
    }

    // Boost alternative model actions
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
        if ctx.db.rl_q_entry().composite_id().find(&composite_id).is_none() {
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
        // Verify the seed defaults match spec
        let map: std::collections::HashMap<&str, f64> = MODEL_ACTIONS.iter().copied().collect();
        assert_eq!(map["model:sonnet"], 0.5);
        assert_eq!(map["model:haiku"], 0.3);
        assert_eq!(map["model:opus"], 0.4);
        assert_eq!(map["model:local"], 0.2);
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
