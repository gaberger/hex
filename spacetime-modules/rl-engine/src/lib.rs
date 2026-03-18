use spacetimedb::{table, reducer, ReducerContext, Table, SpacetimeType};

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

#[reducer]
pub fn select_action(ctx: &ReducerContext, state_key: String) -> Result<(), String> {
    let entries: Vec<RlQEntry> = ctx.db.rl_q_entry().iter()
        .filter(|e| e.state_key == state_key)
        .collect();

    if entries.is_empty() {
        log::info!("No Q-entries for state '{}', exploration needed", state_key);
        return Ok(());
    }

    // Epsilon-greedy: pick best action (exploitation)
    let best = entries.iter()
        .max_by(|a, b| a.q_value.partial_cmp(&b.q_value).unwrap_or(std::cmp::Ordering::Equal));

    if let Some(entry) = best {
        log::info!(
            "Selected action '{}' for state '{}' (q={:.4}, epsilon={:.2})",
            entry.action, state_key, entry.q_value, EPSILON
        );
    }

    Ok(())
}

#[reducer]
pub fn record_reward(
    ctx: &ReducerContext,
    state_key: String,
    action: String,
    reward: f64,
    next_state_key: String,
) -> Result<(), String> {
    let composite_id = format!("{}::{}", state_key, action);

    // Find max Q-value for next state
    let max_next_q: f64 = ctx.db.rl_q_entry().iter()
        .filter(|e| e.state_key == next_state_key)
        .map(|e| e.q_value)
        .fold(0.0_f64, f64::max);

    // Get or create Q-entry
    let existing = ctx.db.rl_q_entry().composite_id().find(&composite_id);

    match existing {
        Some(old) => {
            let new_q = old.q_value + LEARNING_RATE * (reward + DISCOUNT_FACTOR * max_next_q - old.q_value);
            let updated = RlQEntry {
                composite_id: composite_id.clone(),
                state_key: state_key.clone(),
                action: action.clone(),
                q_value: new_q,
                visit_count: old.visit_count + 1,
                last_updated: String::new(), // caller should set via timestamp
            };
            ctx.db.rl_q_entry().composite_id().update(updated);
        }
        None => {
            ctx.db.rl_q_entry().insert(RlQEntry {
                composite_id,
                state_key: state_key.clone(),
                action: action.clone(),
                q_value: reward,
                visit_count: 1,
                last_updated: String::new(),
            });
        }
    }

    // Store experience
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
