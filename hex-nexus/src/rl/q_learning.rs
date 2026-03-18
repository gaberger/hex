use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Available actions the RL engine can select.
const ACTIONS: &[&str] = &[
    "agent:hex-coder",
    "agent:planner",
    "context:aggressive",
    "context:balanced",
    "context:conservative",
    "parallel:1",
    "parallel:2",
    "parallel:4",
    "parallel:8",
];

/// Tabular Q-learning engine for agent/context/parallelism decisions.
/// Stateless beyond hyperparameters — all Q-values live in SQLite.
pub struct QLearningEngine {
    pub learning_rate: f64,
    pub discount_factor: f64,
    pub epsilon: f64,
    pub epsilon_decay: f64,
    pub min_epsilon: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QStats {
    pub table_size: i64,
    pub avg_q_value: f64,
    pub total_experiences: i64,
    pub epsilon: f64,
}

impl Default for QLearningEngine {
    fn default() -> Self {
        Self {
            learning_rate: 0.1,
            discount_factor: 0.95,
            epsilon: 0.1,
            epsilon_decay: 0.995,
            min_epsilon: 0.01,
        }
    }
}

impl QLearningEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Discretize continuous state into a string key for the Q-table.
    /// Buckets: codebase_size (0-4), token_usage (0-4).
    pub fn discretize_state(
        task_type: &str,
        codebase_size: u64,
        agent_count: u8,
        token_usage: u64,
    ) -> String {
        let size_bucket = match codebase_size {
            0..=100 => 0u8,
            101..=1000 => 1,
            1001..=10000 => 2,
            10001..=100000 => 3,
            _ => 4,
        };
        let token_bucket = match token_usage {
            0..=1000 => 0u8,
            1001..=10000 => 1,
            10001..=50000 => 2,
            50001..=200000 => 3,
            _ => 4,
        };
        format!(
            "{}:sz{}:ag{}:tk{}",
            task_type, size_bucket, agent_count, token_bucket
        )
    }

    /// Epsilon-greedy action selection from the Q-table.
    /// If the state has no entries yet, returns a random action.
    pub fn select_action(&self, conn: &Connection, state_key: &str) -> String {
        // Epsilon-greedy: explore with probability epsilon
        let r: f64 = rand_f64();
        if r < self.epsilon {
            // Random exploration
            let idx = rand_usize(ACTIONS.len());
            return ACTIONS[idx].to_string();
        }

        // Greedy: pick the action with the highest Q-value for this state
        let result: Option<String> = conn
            .query_row(
                "SELECT action FROM rl_q_table
                 WHERE state_key = ?1
                 ORDER BY q_value DESC
                 LIMIT 1",
                params![state_key],
                |row| row.get(0),
            )
            .optional()
            .unwrap_or(None);

        result.unwrap_or_else(|| {
            // No entries for this state — pick randomly
            let idx = rand_usize(ACTIONS.len());
            ACTIONS[idx].to_string()
        })
    }

    /// Q-learning update: Q(s,a) += lr * (reward + gamma * max_Q(s') - Q(s,a))
    pub fn update(
        &mut self,
        conn: &Connection,
        state_key: &str,
        action: &str,
        reward: f64,
        next_state_key: &str,
    ) {
        // Get current Q(s, a)
        let current_q: f64 = conn
            .query_row(
                "SELECT q_value FROM rl_q_table WHERE state_key = ?1 AND action = ?2",
                params![state_key, action],
                |row| row.get(0),
            )
            .optional()
            .unwrap_or(None)
            .unwrap_or(0.0);

        // Get max Q(s', a') for next state
        let max_next_q: f64 = conn
            .query_row(
                "SELECT MAX(q_value) FROM rl_q_table WHERE state_key = ?1",
                params![next_state_key],
                |row| row.get::<_, Option<f64>>(0).map(|v| v.unwrap_or(0.0)),
            )
            .unwrap_or(0.0);

        // Bellman equation
        let new_q =
            current_q + self.learning_rate * (reward + self.discount_factor * max_next_q - current_q);
        let now = chrono::Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO rl_q_table (state_key, action, q_value, visit_count, last_updated)
             VALUES (?1, ?2, ?3, 1, ?4)
             ON CONFLICT(state_key, action) DO UPDATE SET
                q_value = ?3,
                visit_count = visit_count + 1,
                last_updated = ?4",
            params![state_key, action, new_q, now],
        )
        .expect("Failed to update Q-table");

        // Decay epsilon
        self.epsilon = (self.epsilon * self.epsilon_decay).max(self.min_epsilon);
    }

    /// Record a raw experience tuple for replay / analysis.
    pub fn record_experience(
        conn: &Connection,
        state: &str,
        action: &str,
        reward: f64,
        next_state: &str,
        task_type: &str,
    ) {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO rl_experiences (id, state, action, reward, next_state, timestamp, task_type)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, state, action, reward, next_state, now, task_type],
        )
        .expect("Failed to record experience");
    }

    /// Return aggregate stats about the Q-table and experience buffer.
    pub fn get_stats(conn: &Connection, epsilon: f64) -> QStats {
        let table_size: i64 = conn
            .query_row("SELECT COUNT(*) FROM rl_q_table", [], |r| r.get(0))
            .unwrap_or(0);

        let avg_q_value: f64 = conn
            .query_row(
                "SELECT COALESCE(AVG(q_value), 0.0) FROM rl_q_table",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0.0);

        let total_experiences: i64 = conn
            .query_row("SELECT COUNT(*) FROM rl_experiences", [], |r| r.get(0))
            .unwrap_or(0);

        QStats {
            table_size,
            avg_q_value,
            total_experiences,
            epsilon,
        }
    }
}

// ── Simple RNG (no external crate needed) ──────────────

/// Quick pseudo-random f64 in [0, 1) using thread-local state seeded from time.
fn rand_f64() -> f64 {
    use std::cell::Cell;
    use std::time::SystemTime;

    thread_local! {
        static STATE: Cell<u64> = Cell::new(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64
        );
    }

    STATE.with(|s| {
        // xorshift64
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        (x as f64) / (u64::MAX as f64)
    })
}

fn rand_usize(upper: usize) -> usize {
    ((rand_f64() * upper as f64) as usize).min(upper - 1)
}

// ── Tests ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rl::schema::migrate_rl;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate_rl(&conn).unwrap();
        conn
    }

    #[test]
    fn discretize_state_produces_consistent_keys() {
        let k1 = QLearningEngine::discretize_state("build", 500, 2, 5000);
        let k2 = QLearningEngine::discretize_state("build", 500, 2, 5000);
        assert_eq!(k1, k2);
        assert_eq!(k1, "build:sz1:ag2:tk1");
    }

    #[test]
    fn discretize_state_buckets_correctly() {
        assert_eq!(
            QLearningEngine::discretize_state("test", 50, 1, 500),
            "test:sz0:ag1:tk0"
        );
        assert_eq!(
            QLearningEngine::discretize_state("test", 999999, 8, 999999),
            "test:sz4:ag8:tk4"
        );
    }

    #[test]
    fn select_action_returns_valid_action() {
        let conn = setup();
        let engine = QLearningEngine::new();
        let action = engine.select_action(&conn, "test:sz0:ag1:tk0");
        assert!(ACTIONS.contains(&action.as_str()));
    }

    #[test]
    fn update_creates_and_updates_q_values() {
        let conn = setup();
        let mut engine = QLearningEngine::new();

        engine.update(&conn, "s1", "agent:planner", 1.0, "s2");

        let q: f64 = conn
            .query_row(
                "SELECT q_value FROM rl_q_table WHERE state_key = 's1' AND action = 'agent:planner'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // Q = 0 + 0.1 * (1.0 + 0.95*0.0 - 0.0) = 0.1
        assert!((q - 0.1).abs() < 1e-9);

        // Second update
        engine.update(&conn, "s1", "agent:planner", 2.0, "s2");
        let q2: f64 = conn
            .query_row(
                "SELECT q_value FROM rl_q_table WHERE state_key = 's1' AND action = 'agent:planner'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // Q = 0.1 + 0.1 * (2.0 + 0.95*0.0 - 0.1) = 0.1 + 0.19 = 0.29
        assert!((q2 - 0.29).abs() < 1e-9);
    }

    #[test]
    fn record_experience_stores_rows() {
        let conn = setup();
        QLearningEngine::record_experience(&conn, "{}", "agent:planner", 0.5, "{}", "build");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM rl_experiences", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn get_stats_returns_correct_values() {
        let conn = setup();
        let mut engine = QLearningEngine::new();

        let stats = QLearningEngine::get_stats(&conn, engine.epsilon);
        assert_eq!(stats.table_size, 0);
        assert_eq!(stats.total_experiences, 0);

        engine.update(&conn, "s1", "agent:planner", 1.0, "s2");
        QLearningEngine::record_experience(&conn, "{}", "agent:planner", 1.0, "{}", "build");

        let stats = QLearningEngine::get_stats(&conn, engine.epsilon);
        assert_eq!(stats.table_size, 1);
        assert_eq!(stats.total_experiences, 1);
    }

    #[test]
    fn epsilon_decays_after_update() {
        let conn = setup();
        let mut engine = QLearningEngine::new();
        let initial = engine.epsilon;

        engine.update(&conn, "s1", "agent:planner", 1.0, "s2");
        assert!(engine.epsilon < initial);
        assert!((engine.epsilon - initial * engine.epsilon_decay).abs() < 1e-12);
    }

    #[test]
    fn epsilon_does_not_go_below_min() {
        let conn = setup();
        let mut engine = QLearningEngine::new();
        engine.epsilon = 0.011;
        engine.epsilon_decay = 0.5;
        engine.min_epsilon = 0.01;

        engine.update(&conn, "s1", "agent:planner", 1.0, "s2");
        assert!((engine.epsilon - 0.01).abs() < 1e-12);
    }
}
