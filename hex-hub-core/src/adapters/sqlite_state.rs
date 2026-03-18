use async_trait::async_trait;
use rusqlite::Connection;
use std::sync::Mutex;
use tokio::sync::broadcast;

use crate::ports::state::*;
use crate::rl::q_learning::QLearningEngine;
use crate::rl::patterns::PatternStore;

/// SQLite-backed implementation of IStatePort.
///
/// Wraps the existing RL engine, orchestration, and fleet SQLite code
/// behind the unified state port abstraction. This is the default backend.
pub struct SqliteStateAdapter {
    db: Mutex<Connection>,
    rl: Mutex<QLearningEngine>,
    event_tx: broadcast::Sender<StateEvent>,
}

impl SqliteStateAdapter {
    pub fn new(db_path: &str) -> Result<Self, StateError> {
        let conn = Connection::open(db_path)
            .map_err(|e| StateError::Storage(e.to_string()))?;

        // Run migrations
        crate::rl::schema::migrate_rl(&conn)
            .map_err(|e| StateError::Storage(e.to_string()))?;

        Self::migrate_state_tables(&conn)?;

        let (event_tx, _) = broadcast::channel(256);

        Ok(Self {
            db: Mutex::new(conn),
            rl: Mutex::new(QLearningEngine::new()),
            event_tx,
        })
    }

    fn migrate_state_tables(conn: &Connection) -> Result<(), StateError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS state_agents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                project_dir TEXT NOT NULL,
                model TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'spawning',
                started_at TEXT NOT NULL,
                ended_at TEXT,
                metrics_json TEXT
            );
            CREATE TABLE IF NOT EXISTS state_workplan_tasks (
                task_id TEXT PRIMARY KEY,
                workplan_id TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                agent_id TEXT,
                result TEXT
            );
            CREATE TABLE IF NOT EXISTS state_chat_messages (
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                timestamp TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS state_fleet_nodes (
                id TEXT PRIMARY KEY,
                host TEXT NOT NULL,
                port INTEGER NOT NULL DEFAULT 22,
                status TEXT NOT NULL DEFAULT 'registered',
                active_agents INTEGER NOT NULL DEFAULT 0,
                max_agents INTEGER NOT NULL DEFAULT 4,
                last_health_check TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_chat_conv ON state_chat_messages(conversation_id, timestamp);
            CREATE INDEX IF NOT EXISTS idx_tasks_wp ON state_workplan_tasks(workplan_id);

            CREATE TABLE IF NOT EXISTS state_skills (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                description TEXT NOT NULL DEFAULT '',
                triggers_json TEXT NOT NULL DEFAULT '[]',
                body TEXT NOT NULL DEFAULT '',
                source TEXT NOT NULL DEFAULT 'filesystem',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS state_skill_triggers (
                skill_id TEXT NOT NULL,
                trigger_type TEXT NOT NULL,
                trigger_value TEXT NOT NULL,
                FOREIGN KEY (skill_id) REFERENCES state_skills(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_skill_triggers ON state_skill_triggers(trigger_type, trigger_value);

            CREATE TABLE IF NOT EXISTS state_hooks (
                id TEXT PRIMARY KEY,
                event_type TEXT NOT NULL,
                handler_type TEXT NOT NULL DEFAULT 'shell',
                handler_config_json TEXT NOT NULL DEFAULT '{}',
                timeout_secs INTEGER NOT NULL DEFAULT 30,
                blocking INTEGER NOT NULL DEFAULT 0,
                tool_pattern TEXT NOT NULL DEFAULT '',
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_hooks_event ON state_hooks(event_type);
            CREATE TABLE IF NOT EXISTS state_hook_execution_log (
                hook_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                exit_code INTEGER NOT NULL,
                stdout TEXT NOT NULL DEFAULT '',
                stderr TEXT NOT NULL DEFAULT '',
                duration_ms INTEGER NOT NULL DEFAULT 0,
                timed_out INTEGER NOT NULL DEFAULT 0,
                timestamp TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS state_agent_definitions (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                description TEXT NOT NULL DEFAULT '',
                role_prompt TEXT NOT NULL DEFAULT '',
                allowed_tools_json TEXT NOT NULL DEFAULT '[]',
                constraints_json TEXT NOT NULL DEFAULT '{}',
                model TEXT NOT NULL DEFAULT '',
                max_turns INTEGER NOT NULL DEFAULT 50,
                metadata_json TEXT NOT NULL DEFAULT '{}',
                version INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS state_agent_definition_versions (
                definition_id TEXT NOT NULL,
                version INTEGER NOT NULL,
                snapshot_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                PRIMARY KEY (definition_id, version)
            );",
        )
        .map_err(|e| StateError::Storage(e.to_string()))?;
        Ok(())
    }
}

#[async_trait]
impl IStatePort for SqliteStateAdapter {
    // ── RL ───────────────────────────────────────────

    async fn rl_select_action(&self, state: &RlState) -> Result<String, StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let rl = self.rl.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let state_key = QLearningEngine::discretize_state(
            &state.task_type,
            state.codebase_size,
            state.agent_count,
            state.token_usage,
        );
        Ok(rl.select_action(&conn, &state_key))
    }

    async fn rl_record_reward(
        &self,
        state_key: &str,
        action: &str,
        reward: f64,
        next_state_key: &str,
    ) -> Result<(), StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let mut rl = self.rl.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        rl.update(&conn, state_key, action, reward, next_state_key);
        QLearningEngine::record_experience(&conn, state_key, action, reward, next_state_key, "unknown");
        Ok(())
    }

    async fn rl_get_stats(&self) -> Result<RlStats, StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let rl = self.rl.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let stats = QLearningEngine::get_stats(&conn, rl.epsilon);
        Ok(RlStats {
            q_table_size: stats.table_size as usize,
            avg_q_value: stats.avg_q_value,
            epsilon: stats.epsilon,
            total_experiences: stats.total_experiences as usize,
        })
    }

    // ── Patterns ────────────────────────────────────

    async fn pattern_store(
        &self,
        category: &str,
        content: &str,
        confidence: f64,
    ) -> Result<String, StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        Ok(PatternStore::store(&conn, category, content, confidence))
    }

    async fn pattern_search(
        &self,
        category: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<PatternEntry>, StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let patterns = PatternStore::search(&conn, category, query, limit);
        Ok(patterns
            .into_iter()
            .map(|p| PatternEntry {
                id: p.id,
                category: p.category,
                content: p.content,
                confidence: p.confidence,
                access_count: p.access_count.try_into().unwrap_or(0),
            })
            .collect())
    }

    async fn pattern_reinforce(&self, id: &str, delta: f64) -> Result<(), StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        PatternStore::reinforce(&conn, id, delta);
        Ok(())
    }

    async fn pattern_decay_all(&self) -> Result<u32, StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        PatternStore::decay_all(&conn);
        Ok(0)
    }

    // ── Agent Registry ──────────────────────────────

    async fn agent_register(&self, info: AgentInfo) -> Result<String, StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        conn.execute(
            "INSERT OR REPLACE INTO state_agents (id, name, project_dir, model, status, started_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![info.id, info.name, info.project_dir, info.model, serde_json::to_string(&info.status).unwrap_or_default().trim_matches('"'), info.started_at],
        ).map_err(|e| StateError::Storage(e.to_string()))?;

        let _ = self.event_tx.send(StateEvent::AgentChanged { agent: info.clone() });
        Ok(info.id)
    }

    async fn agent_update_status(
        &self,
        id: &str,
        status: AgentStatus,
        metrics: Option<AgentMetricsData>,
    ) -> Result<(), StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let status_str = serde_json::to_string(&status).unwrap_or_default();
        let status_str = status_str.trim_matches('"');
        let metrics_json = metrics.map(|m| serde_json::to_string(&m).unwrap_or_default());

        conn.execute(
            "UPDATE state_agents SET status = ?1, metrics_json = ?2 WHERE id = ?3",
            rusqlite::params![status_str, metrics_json, id],
        ).map_err(|e| StateError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn agent_list(&self) -> Result<Vec<AgentInfo>, StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let mut stmt = conn
            .prepare("SELECT id, name, project_dir, model, status, started_at FROM state_agents ORDER BY started_at DESC")
            .map_err(|e| StateError::Storage(e.to_string()))?;

        let agents = stmt
            .query_map([], |row| {
                let status_str: String = row.get(4)?;
                let status = match status_str.as_str() {
                    "running" => AgentStatus::Running,
                    "completed" => AgentStatus::Completed,
                    "failed" => AgentStatus::Failed,
                    _ => AgentStatus::Spawning,
                };
                Ok(AgentInfo {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    project_dir: row.get(2)?,
                    model: row.get(3)?,
                    status,
                    started_at: row.get(5)?,
                })
            })
            .map_err(|e| StateError::Storage(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(agents)
    }

    async fn agent_get(&self, id: &str) -> Result<Option<AgentInfo>, StateError> {
        let all = self.agent_list().await?;
        Ok(all.into_iter().find(|a| a.id == id))
    }

    async fn agent_remove(&self, id: &str) -> Result<(), StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        conn.execute("DELETE FROM state_agents WHERE id = ?1", [id])
            .map_err(|e| StateError::Storage(e.to_string()))?;
        Ok(())
    }

    // ── Workplan ────────────────────────────────────

    async fn workplan_update_task(&self, update: WorkplanTaskUpdate) -> Result<(), StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        conn.execute(
            "INSERT OR REPLACE INTO state_workplan_tasks (task_id, workplan_id, status, agent_id, result) VALUES (?1, '', ?2, ?3, ?4)",
            rusqlite::params![update.task_id, update.status, update.agent_id, update.result],
        ).map_err(|e| StateError::Storage(e.to_string()))?;

        let _ = self.event_tx.send(StateEvent::TaskChanged { update });
        Ok(())
    }

    async fn workplan_get_tasks(
        &self,
        _workplan_id: &str,
    ) -> Result<Vec<WorkplanTaskUpdate>, StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let mut stmt = conn
            .prepare("SELECT task_id, status, agent_id, result FROM state_workplan_tasks")
            .map_err(|e| StateError::Storage(e.to_string()))?;

        let tasks = stmt
            .query_map([], |row| {
                Ok(WorkplanTaskUpdate {
                    task_id: row.get(0)?,
                    status: row.get(1)?,
                    agent_id: row.get(2)?,
                    result: row.get(3)?,
                })
            })
            .map_err(|e| StateError::Storage(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(tasks)
    }

    // ── Chat ────────────────────────────────────────

    async fn chat_send(&self, message: ChatMessage) -> Result<(), StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        conn.execute(
            "INSERT INTO state_chat_messages (id, conversation_id, role, content, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![message.id, message.conversation_id, message.role, message.content, message.timestamp],
        ).map_err(|e| StateError::Storage(e.to_string()))?;

        let _ = self.event_tx.send(StateEvent::ChatMessage { message });
        Ok(())
    }

    async fn chat_history(
        &self,
        conversation_id: &str,
        limit: u32,
    ) -> Result<Vec<ChatMessage>, StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let mut stmt = conn
            .prepare("SELECT id, conversation_id, role, content, timestamp FROM state_chat_messages WHERE conversation_id = ?1 ORDER BY timestamp DESC LIMIT ?2")
            .map_err(|e| StateError::Storage(e.to_string()))?;

        let msgs = stmt
            .query_map(rusqlite::params![conversation_id, limit], |row| {
                Ok(ChatMessage {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    timestamp: row.get(4)?,
                })
            })
            .map_err(|e| StateError::Storage(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(msgs)
    }

    // ── Fleet ───────────────────────────────────────

    async fn fleet_register(&self, node: FleetNode) -> Result<(), StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        conn.execute(
            "INSERT OR REPLACE INTO state_fleet_nodes (id, host, port, status, active_agents, max_agents, last_health_check) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![node.id, node.host, node.port, node.status, node.active_agents, node.max_agents, node.last_health_check],
        ).map_err(|e| StateError::Storage(e.to_string()))?;

        let _ = self.event_tx.send(StateEvent::FleetChanged { node });
        Ok(())
    }

    async fn fleet_update_status(&self, id: &str, status: &str) -> Result<(), StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        conn.execute(
            "UPDATE state_fleet_nodes SET status = ?1 WHERE id = ?2",
            rusqlite::params![status, id],
        ).map_err(|e| StateError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn fleet_list(&self) -> Result<Vec<FleetNode>, StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let mut stmt = conn
            .prepare("SELECT id, host, port, status, active_agents, max_agents, last_health_check FROM state_fleet_nodes")
            .map_err(|e| StateError::Storage(e.to_string()))?;

        let nodes = stmt
            .query_map([], |row| {
                Ok(FleetNode {
                    id: row.get(0)?,
                    host: row.get(1)?,
                    port: row.get::<_, u32>(2)? as u16,
                    status: row.get(3)?,
                    active_agents: row.get(4)?,
                    max_agents: row.get(5)?,
                    last_health_check: row.get(6)?,
                })
            })
            .map_err(|e| StateError::Storage(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(nodes)
    }

    async fn fleet_remove(&self, id: &str) -> Result<(), StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        conn.execute("DELETE FROM state_fleet_nodes WHERE id = ?1", [id])
            .map_err(|e| StateError::Storage(e.to_string()))?;
        Ok(())
    }

    // ── Skill Registry (SpacetimeDB-primary, stubs for SQLite) ──

    async fn skill_register(&self, skill: SkillEntry) -> Result<String, StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        conn.execute(
            "INSERT OR REPLACE INTO state_skills (id, name, description, triggers_json, body, source, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![skill.id, skill.name, skill.description, skill.triggers_json, skill.body, skill.source, skill.created_at, skill.updated_at],
        ).map_err(|e| StateError::Storage(e.to_string()))?;
        let _ = self.event_tx.send(StateEvent::SkillChanged { skill: skill.clone() });
        Ok(skill.id)
    }

    async fn skill_update(&self, id: &str, description: &str, triggers_json: &str, body: &str) -> Result<(), StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE state_skills SET description = ?1, triggers_json = ?2, body = ?3, updated_at = ?4 WHERE id = ?5",
            rusqlite::params![description, triggers_json, body, now, id],
        ).map_err(|e| StateError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn skill_remove(&self, id: &str) -> Result<(), StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        conn.execute("DELETE FROM state_skills WHERE id = ?1", [id])
            .map_err(|e| StateError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn skill_list(&self) -> Result<Vec<SkillEntry>, StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let mut stmt = conn.prepare("SELECT id, name, description, triggers_json, body, source, created_at, updated_at FROM state_skills")
            .map_err(|e| StateError::Storage(e.to_string()))?;
        let skills = stmt.query_map([], |row| {
            Ok(SkillEntry {
                id: row.get(0)?, name: row.get(1)?, description: row.get(2)?,
                triggers_json: row.get(3)?, body: row.get(4)?, source: row.get(5)?,
                created_at: row.get(6)?, updated_at: row.get(7)?,
            })
        }).map_err(|e| StateError::Storage(e.to_string()))?.filter_map(|r| r.ok()).collect();
        Ok(skills)
    }

    async fn skill_get(&self, id: &str) -> Result<Option<SkillEntry>, StateError> {
        let all = self.skill_list().await?;
        Ok(all.into_iter().find(|s| s.id == id))
    }

    async fn skill_search(&self, trigger_type: &str, query: &str) -> Result<Vec<SkillEntry>, StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT s.id, s.name, s.description, s.triggers_json, s.body, s.source, s.created_at, s.updated_at
             FROM state_skills s JOIN state_skill_triggers t ON s.id = t.skill_id
             WHERE t.trigger_type = ?1 AND t.trigger_value LIKE '%' || ?2 || '%'"
        ).map_err(|e| StateError::Storage(e.to_string()))?;
        let skills = stmt.query_map(rusqlite::params![trigger_type, query], |row| {
            Ok(SkillEntry {
                id: row.get(0)?, name: row.get(1)?, description: row.get(2)?,
                triggers_json: row.get(3)?, body: row.get(4)?, source: row.get(5)?,
                created_at: row.get(6)?, updated_at: row.get(7)?,
            })
        }).map_err(|e| StateError::Storage(e.to_string()))?.filter_map(|r| r.ok()).collect();
        Ok(skills)
    }

    // ── Hook Registry ──────────────────────────────────

    async fn hook_register(&self, hook: HookEntry) -> Result<String, StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        conn.execute(
            "INSERT OR REPLACE INTO state_hooks (id, event_type, handler_type, handler_config_json, timeout_secs, blocking, tool_pattern, enabled, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![hook.id, hook.event_type, hook.handler_type, hook.handler_config_json, hook.timeout_secs, hook.blocking as i32, hook.tool_pattern, hook.enabled as i32, hook.created_at, hook.updated_at],
        ).map_err(|e| StateError::Storage(e.to_string()))?;
        let _ = self.event_tx.send(StateEvent::HookChanged { hook: hook.clone() });
        Ok(hook.id)
    }

    async fn hook_update(&self, id: &str, handler_config_json: &str, timeout_secs: u32, blocking: bool, tool_pattern: &str) -> Result<(), StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE state_hooks SET handler_config_json = ?1, timeout_secs = ?2, blocking = ?3, tool_pattern = ?4, updated_at = ?5 WHERE id = ?6",
            rusqlite::params![handler_config_json, timeout_secs, blocking as i32, tool_pattern, now, id],
        ).map_err(|e| StateError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn hook_remove(&self, id: &str) -> Result<(), StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        conn.execute("DELETE FROM state_hooks WHERE id = ?1", [id])
            .map_err(|e| StateError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn hook_toggle(&self, id: &str, enabled: bool) -> Result<(), StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE state_hooks SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![enabled as i32, now, id],
        ).map_err(|e| StateError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn hook_list(&self) -> Result<Vec<HookEntry>, StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let mut stmt = conn.prepare("SELECT id, event_type, handler_type, handler_config_json, timeout_secs, blocking, tool_pattern, enabled, created_at, updated_at FROM state_hooks")
            .map_err(|e| StateError::Storage(e.to_string()))?;
        let hooks = stmt.query_map([], |row| {
            Ok(HookEntry {
                id: row.get(0)?, event_type: row.get(1)?, handler_type: row.get(2)?,
                handler_config_json: row.get(3)?, timeout_secs: row.get(4)?,
                blocking: row.get::<_, i32>(5)? != 0, tool_pattern: row.get(6)?,
                enabled: row.get::<_, i32>(7)? != 0, created_at: row.get(8)?, updated_at: row.get(9)?,
            })
        }).map_err(|e| StateError::Storage(e.to_string()))?.filter_map(|r| r.ok()).collect();
        Ok(hooks)
    }

    async fn hook_list_by_event(&self, event_type: &str) -> Result<Vec<HookEntry>, StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let mut stmt = conn.prepare("SELECT id, event_type, handler_type, handler_config_json, timeout_secs, blocking, tool_pattern, enabled, created_at, updated_at FROM state_hooks WHERE event_type = ?1 AND enabled = 1")
            .map_err(|e| StateError::Storage(e.to_string()))?;
        let hooks = stmt.query_map(rusqlite::params![event_type], |row| {
            Ok(HookEntry {
                id: row.get(0)?, event_type: row.get(1)?, handler_type: row.get(2)?,
                handler_config_json: row.get(3)?, timeout_secs: row.get(4)?,
                blocking: row.get::<_, i32>(5)? != 0, tool_pattern: row.get(6)?,
                enabled: row.get::<_, i32>(7)? != 0, created_at: row.get(8)?, updated_at: row.get(9)?,
            })
        }).map_err(|e| StateError::Storage(e.to_string()))?.filter_map(|r| r.ok()).collect();
        Ok(hooks)
    }

    async fn hook_log_execution(&self, entry: HookExecutionEntry) -> Result<(), StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        conn.execute(
            "INSERT INTO state_hook_execution_log (hook_id, agent_id, event_type, exit_code, stdout, stderr, duration_ms, timed_out, timestamp) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![entry.hook_id, entry.agent_id, entry.event_type, entry.exit_code, entry.stdout, entry.stderr, entry.duration_ms, entry.timed_out as i32, entry.timestamp],
        ).map_err(|e| StateError::Storage(e.to_string()))?;
        Ok(())
    }

    // ── Agent Definition Registry ──────────────────────

    async fn agent_def_register(&self, def: AgentDefinitionEntry) -> Result<String, StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        conn.execute(
            "INSERT OR REPLACE INTO state_agent_definitions (id, name, description, role_prompt, allowed_tools_json, constraints_json, model, max_turns, metadata_json, version, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![def.id, def.name, def.description, def.role_prompt, def.allowed_tools_json, def.constraints_json, def.model, def.max_turns, def.metadata_json, def.version, def.created_at, def.updated_at],
        ).map_err(|e| StateError::Storage(e.to_string()))?;
        // Store version snapshot
        let snapshot = serde_json::to_string(&def).unwrap_or_default();
        conn.execute(
            "INSERT OR REPLACE INTO state_agent_definition_versions (definition_id, version, snapshot_json, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![def.id, def.version, snapshot, def.created_at],
        ).map_err(|e| StateError::Storage(e.to_string()))?;
        let _ = self.event_tx.send(StateEvent::AgentDefinitionChanged { definition: def.clone() });
        Ok(def.id)
    }

    async fn agent_def_update(
        &self, id: &str, description: &str, role_prompt: &str,
        allowed_tools_json: &str, constraints_json: &str, model: &str,
        max_turns: u32, metadata_json: &str,
    ) -> Result<(), StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE state_agent_definitions SET description = ?1, role_prompt = ?2, allowed_tools_json = ?3, constraints_json = ?4, model = ?5, max_turns = ?6, metadata_json = ?7, version = version + 1, updated_at = ?8 WHERE id = ?9",
            rusqlite::params![description, role_prompt, allowed_tools_json, constraints_json, model, max_turns, metadata_json, now, id],
        ).map_err(|e| StateError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn agent_def_remove(&self, id: &str) -> Result<(), StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        conn.execute("DELETE FROM state_agent_definitions WHERE id = ?1", [id])
            .map_err(|e| StateError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn agent_def_list(&self) -> Result<Vec<AgentDefinitionEntry>, StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let mut stmt = conn.prepare("SELECT id, name, description, role_prompt, allowed_tools_json, constraints_json, model, max_turns, metadata_json, version, created_at, updated_at FROM state_agent_definitions")
            .map_err(|e| StateError::Storage(e.to_string()))?;
        let defs = stmt.query_map([], |row| {
            Ok(AgentDefinitionEntry {
                id: row.get(0)?, name: row.get(1)?, description: row.get(2)?,
                role_prompt: row.get(3)?, allowed_tools_json: row.get(4)?,
                constraints_json: row.get(5)?, model: row.get(6)?, max_turns: row.get(7)?,
                metadata_json: row.get(8)?, version: row.get(9)?,
                created_at: row.get(10)?, updated_at: row.get(11)?,
            })
        }).map_err(|e| StateError::Storage(e.to_string()))?.filter_map(|r| r.ok()).collect();
        Ok(defs)
    }

    async fn agent_def_get_by_name(&self, name: &str) -> Result<Option<AgentDefinitionEntry>, StateError> {
        let all = self.agent_def_list().await?;
        Ok(all.into_iter().find(|d| d.name == name))
    }

    async fn agent_def_versions(&self, definition_id: &str) -> Result<Vec<AgentDefinitionVersionEntry>, StateError> {
        let conn = self.db.lock().map_err(|e| StateError::Storage(e.to_string()))?;
        let mut stmt = conn.prepare("SELECT definition_id, version, snapshot_json, created_at FROM state_agent_definition_versions WHERE definition_id = ?1 ORDER BY version DESC")
            .map_err(|e| StateError::Storage(e.to_string()))?;
        let versions = stmt.query_map(rusqlite::params![definition_id], |row| {
            Ok(AgentDefinitionVersionEntry {
                definition_id: row.get(0)?, version: row.get(1)?,
                snapshot_json: row.get(2)?, created_at: row.get(3)?,
            })
        }).map_err(|e| StateError::Storage(e.to_string()))?.filter_map(|r| r.ok()).collect();
        Ok(versions)
    }

    // ── Subscriptions ───────────────────────────────

    fn subscribe(&self) -> broadcast::Receiver<StateEvent> {
        self.event_tx.subscribe()
    }
}
