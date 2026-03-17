use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

// ── Types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Swarm {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub topology: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwarmTask {
    pub id: String,
    pub swarm_id: String,
    pub title: String,
    pub status: String,
    pub agent_id: Option<String>,
    pub result: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwarmAgent {
    pub id: String,
    pub swarm_id: String,
    pub name: String,
    pub role: String,
    pub status: String,
    pub worktree_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwarmDetail {
    #[serde(flatten)]
    pub swarm: Swarm,
    pub tasks: Vec<SwarmTask>,
    pub agents: Vec<SwarmAgent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IncompleteWorkItem {
    pub task_id: String,
    pub task_title: String,
    pub task_status: String,
    pub swarm_id: String,
    pub swarm_name: String,
    pub project_id: String,
    pub agent_id: Option<String>,
}

// ── Request types ──────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSwarmRequest {
    pub project_id: String,
    pub name: String,
    pub topology: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskRequest {
    pub status: Option<String>,
    pub result: Option<String>,
    pub agent_id: Option<String>,
}

// ── SwarmDb ────────────────────────────────────────────

pub struct SwarmDb {
    conn: Arc<Mutex<Connection>>,
}

impl SwarmDb {
    /// Open (or create) the SQLite database at ~/.hex/hub.db.
    /// Runs migrations on first use.
    pub fn open() -> Result<Self, rusqlite::Error> {
        let db_path = Self::db_path();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(&db_path)?;
        Self::migrate(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Open an in-memory database (for tests).
    #[cfg(test)]
    pub fn open_memory() -> Result<Self, rusqlite::Error> {
        let conn = Connection::open_in_memory()?;
        Self::migrate(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn db_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(home).join(".hex").join("hub.db")
    }

    fn migrate(conn: &Connection) -> Result<(), rusqlite::Error> {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS swarms (
                id          TEXT PRIMARY KEY,
                project_id  TEXT NOT NULL,
                name        TEXT NOT NULL,
                topology    TEXT NOT NULL DEFAULT 'hierarchical',
                status      TEXT NOT NULL DEFAULT 'active',
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS swarm_tasks (
                id           TEXT PRIMARY KEY,
                swarm_id     TEXT NOT NULL REFERENCES swarms(id),
                title        TEXT NOT NULL,
                status       TEXT NOT NULL DEFAULT 'pending',
                agent_id     TEXT,
                result       TEXT,
                created_at   TEXT NOT NULL,
                completed_at TEXT
            );

            CREATE TABLE IF NOT EXISTS swarm_agents (
                id            TEXT PRIMARY KEY,
                swarm_id      TEXT NOT NULL REFERENCES swarms(id),
                name          TEXT NOT NULL,
                role          TEXT NOT NULL,
                status        TEXT NOT NULL DEFAULT 'idle',
                worktree_path TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_swarm_tasks_swarm ON swarm_tasks(swarm_id);
            CREATE INDEX IF NOT EXISTS idx_swarm_agents_swarm ON swarm_agents(swarm_id);
            CREATE INDEX IF NOT EXISTS idx_swarms_status ON swarms(status);
            ",
        )
    }

    // ── Swarm CRUD ─────────────────────────────────────

    pub async fn create_swarm(
        &self,
        req: &CreateSwarmRequest,
    ) -> Result<Swarm, rusqlite::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let topology = req.topology.clone().unwrap_or_else(|| "hierarchical".to_string());
        let project_id = req.project_id.clone();
        let name = req.name.clone();

        let swarm = Swarm {
            id: id.clone(),
            project_id: project_id.clone(),
            name: name.clone(),
            topology: topology.clone(),
            status: "active".to_string(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };

        let conn = self.conn.clone();
        let s = swarm.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO swarms (id, project_id, name, topology, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![s.id, s.project_id, s.name, s.topology, s.status, s.created_at, s.updated_at],
            )
        })
        .await
        .expect("spawn_blocking join")?;

        Ok(swarm)
    }

    pub async fn get_swarm(&self, id: &str) -> Result<Option<SwarmDetail>, rusqlite::Error> {
        let conn = self.conn.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();

            let mut stmt = conn.prepare(
                "SELECT id, project_id, name, topology, status, created_at, updated_at
                 FROM swarms WHERE id = ?1",
            )?;
            let swarm = stmt
                .query_row(params![id], |row| {
                    Ok(Swarm {
                        id: row.get(0)?,
                        project_id: row.get(1)?,
                        name: row.get(2)?,
                        topology: row.get(3)?,
                        status: row.get(4)?,
                        created_at: row.get(5)?,
                        updated_at: row.get(6)?,
                    })
                })
                .optional()?;

            let Some(swarm) = swarm else {
                return Ok(None);
            };

            let tasks = Self::query_tasks_sync(&conn, &id)?;
            let agents = Self::query_agents_sync(&conn, &id)?;

            Ok(Some(SwarmDetail {
                swarm,
                tasks,
                agents,
            }))
        })
        .await
        .expect("spawn_blocking join")
    }

    pub async fn list_active_swarms(&self) -> Result<Vec<Swarm>, rusqlite::Error> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT id, project_id, name, topology, status, created_at, updated_at
                 FROM swarms WHERE status != 'completed'
                 ORDER BY created_at DESC",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(Swarm {
                        id: row.get(0)?,
                        project_id: row.get(1)?,
                        name: row.get(2)?,
                        topology: row.get(3)?,
                        status: row.get(4)?,
                        created_at: row.get(5)?,
                        updated_at: row.get(6)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .expect("spawn_blocking join")
    }

    // ── Task operations ────────────────────────────────

    pub async fn update_task(
        &self,
        task_id: &str,
        req: &UpdateTaskRequest,
    ) -> Result<bool, rusqlite::Error> {
        let conn = self.conn.clone();
        let task_id = task_id.to_string();
        let status = req.status.clone();
        let result = req.result.clone();
        let agent_id = req.agent_id.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut sets = Vec::new();
            let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(ref s) = status {
                sets.push("status = ?");
                values.push(Box::new(s.clone()));
                if s == "completed" || s == "failed" {
                    sets.push("completed_at = ?");
                    values.push(Box::new(chrono::Utc::now().to_rfc3339()));
                }
            }
            if let Some(ref r) = result {
                sets.push("result = ?");
                values.push(Box::new(r.clone()));
            }
            if let Some(ref a) = agent_id {
                sets.push("agent_id = ?");
                values.push(Box::new(a.clone()));
            }

            if sets.is_empty() {
                return Ok(false);
            }

            values.push(Box::new(task_id.clone()));

            let sql = format!(
                "UPDATE swarm_tasks SET {} WHERE id = ?",
                sets.join(", ")
            );
            let params: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
            let changed = conn.execute(&sql, params.as_slice())?;

            // If task completed/failed, check if all tasks in swarm are done
            if let Some(ref s) = status {
                if s == "completed" || s == "failed" {
                    Self::maybe_complete_swarm_sync(&conn, &task_id)?;
                }
            }

            Ok(changed > 0)
        })
        .await
        .expect("spawn_blocking join")
    }

    pub async fn complete_task(
        &self,
        task_id: &str,
        result: Option<String>,
    ) -> Result<bool, rusqlite::Error> {
        self.update_task(
            task_id,
            &UpdateTaskRequest {
                status: Some("completed".to_string()),
                result,
                agent_id: None,
            },
        )
        .await
    }

    // ── Incomplete work query ──────────────────────────

    pub async fn get_incomplete_work(&self) -> Result<Vec<IncompleteWorkItem>, rusqlite::Error> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT t.id, t.title, t.status, t.agent_id,
                        s.id, s.name, s.project_id
                 FROM swarm_tasks t
                 JOIN swarms s ON t.swarm_id = s.id
                 WHERE t.status NOT IN ('completed', 'failed')
                   AND s.status != 'completed'
                 ORDER BY t.created_at ASC",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(IncompleteWorkItem {
                        task_id: row.get(0)?,
                        task_title: row.get(1)?,
                        task_status: row.get(2)?,
                        agent_id: row.get(3)?,
                        swarm_id: row.get(4)?,
                        swarm_name: row.get(5)?,
                        project_id: row.get(6)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .expect("spawn_blocking join")
    }

    // ── Sync helpers (called inside spawn_blocking) ────

    fn query_tasks_sync(
        conn: &Connection,
        swarm_id: &str,
    ) -> Result<Vec<SwarmTask>, rusqlite::Error> {
        let mut stmt = conn.prepare(
            "SELECT id, swarm_id, title, status, agent_id, result, created_at, completed_at
             FROM swarm_tasks WHERE swarm_id = ?1
             ORDER BY created_at ASC",
        )?;
        let result = stmt.query_map(params![swarm_id], |row| {
            Ok(SwarmTask {
                id: row.get(0)?,
                swarm_id: row.get(1)?,
                title: row.get(2)?,
                status: row.get(3)?,
                agent_id: row.get(4)?,
                result: row.get(5)?,
                created_at: row.get(6)?,
                completed_at: row.get(7)?,
            })
        })?
        .collect();
        result
    }

    fn query_agents_sync(
        conn: &Connection,
        swarm_id: &str,
    ) -> Result<Vec<SwarmAgent>, rusqlite::Error> {
        let mut stmt = conn.prepare(
            "SELECT id, swarm_id, name, role, status, worktree_path
             FROM swarm_agents WHERE swarm_id = ?1
             ORDER BY name ASC",
        )?;
        let result = stmt.query_map(params![swarm_id], |row| {
            Ok(SwarmAgent {
                id: row.get(0)?,
                swarm_id: row.get(1)?,
                name: row.get(2)?,
                role: row.get(3)?,
                status: row.get(4)?,
                worktree_path: row.get(5)?,
            })
        })?
        .collect();
        result
    }

    fn maybe_complete_swarm_sync(
        conn: &Connection,
        task_id: &str,
    ) -> Result<(), rusqlite::Error> {
        // Find swarm_id for this task
        let swarm_id: Option<String> = conn
            .query_row(
                "SELECT swarm_id FROM swarm_tasks WHERE id = ?1",
                params![task_id],
                |row| row.get(0),
            )
            .optional()?;

        let Some(swarm_id) = swarm_id else {
            return Ok(());
        };

        // Check if any tasks are still pending/running
        let remaining: i64 = conn.query_row(
            "SELECT COUNT(*) FROM swarm_tasks
             WHERE swarm_id = ?1 AND status NOT IN ('completed', 'failed')",
            params![swarm_id],
            |row| row.get(0),
        )?;

        if remaining == 0 {
            let now = chrono::Utc::now().to_rfc3339();
            conn.execute(
                "UPDATE swarms SET status = 'completed', updated_at = ?1 WHERE id = ?2",
                params![now, swarm_id],
            )?;
        }

        Ok(())
    }
}

// We need the optional() extension
use rusqlite::OptionalExtension;

// ── Tests ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_and_get_swarm() {
        let db = SwarmDb::open_memory().unwrap();

        let swarm = db
            .create_swarm(&CreateSwarmRequest {
                project_id: "test-proj".to_string(),
                name: "my-swarm".to_string(),
                topology: Some("mesh".to_string()),
            })
            .await
            .unwrap();

        assert_eq!(swarm.name, "my-swarm");
        assert_eq!(swarm.topology, "mesh");
        assert_eq!(swarm.status, "active");

        let detail = db.get_swarm(&swarm.id).await.unwrap().unwrap();
        assert_eq!(detail.swarm.id, swarm.id);
        assert!(detail.tasks.is_empty());
        assert!(detail.agents.is_empty());
    }

    #[tokio::test]
    async fn test_list_active_swarms() {
        let db = SwarmDb::open_memory().unwrap();

        db.create_swarm(&CreateSwarmRequest {
            project_id: "p1".to_string(),
            name: "s1".to_string(),
            topology: None,
        })
        .await
        .unwrap();

        db.create_swarm(&CreateSwarmRequest {
            project_id: "p2".to_string(),
            name: "s2".to_string(),
            topology: None,
        })
        .await
        .unwrap();

        let active = db.list_active_swarms().await.unwrap();
        assert_eq!(active.len(), 2);
    }

    #[tokio::test]
    async fn test_create_task_and_complete() {
        let db = SwarmDb::open_memory().unwrap();

        let swarm = db
            .create_swarm(&CreateSwarmRequest {
                project_id: "proj".to_string(),
                name: "test".to_string(),
                topology: None,
            })
            .await
            .unwrap();

        // Insert a task directly for testing
        {
            let conn = db.conn.lock().await;
            conn.execute(
                "INSERT INTO swarm_tasks (id, swarm_id, title, status, created_at)
                 VALUES ('t1', ?1, 'Build widget', 'pending', ?2)",
                params![swarm.id, chrono::Utc::now().to_rfc3339()],
            )
            .unwrap();
        }

        // Task shows up as incomplete
        let incomplete = db.get_incomplete_work().await.unwrap();
        assert_eq!(incomplete.len(), 1);
        assert_eq!(incomplete[0].task_title, "Build widget");

        // Complete it
        let ok = db.complete_task("t1", Some("done".to_string())).await.unwrap();
        assert!(ok);

        // No more incomplete work
        let incomplete = db.get_incomplete_work().await.unwrap();
        assert!(incomplete.is_empty());

        // Swarm auto-completed
        let detail = db.get_swarm(&swarm.id).await.unwrap().unwrap();
        assert_eq!(detail.swarm.status, "completed");
    }

    #[tokio::test]
    async fn test_update_task_partial() {
        let db = SwarmDb::open_memory().unwrap();

        let swarm = db
            .create_swarm(&CreateSwarmRequest {
                project_id: "proj".to_string(),
                name: "test".to_string(),
                topology: None,
            })
            .await
            .unwrap();

        {
            let conn = db.conn.lock().await;
            conn.execute(
                "INSERT INTO swarm_tasks (id, swarm_id, title, status, created_at)
                 VALUES ('t2', ?1, 'Task 2', 'pending', ?2)",
                params![swarm.id, chrono::Utc::now().to_rfc3339()],
            )
            .unwrap();
        }

        // Update just the agent_id
        let ok = db
            .update_task(
                "t2",
                &UpdateTaskRequest {
                    status: Some("running".to_string()),
                    result: None,
                    agent_id: Some("agent-1".to_string()),
                },
            )
            .await
            .unwrap();
        assert!(ok);

        let detail = db.get_swarm(&swarm.id).await.unwrap().unwrap();
        let task = &detail.tasks[0];
        assert_eq!(task.status, "running");
        assert_eq!(task.agent_id.as_deref(), Some("agent-1"));
    }

    #[tokio::test]
    async fn test_nonexistent_swarm_returns_none() {
        let db = SwarmDb::open_memory().unwrap();
        let result = db.get_swarm("nope").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_empty_update_returns_false() {
        let db = SwarmDb::open_memory().unwrap();
        let ok = db
            .update_task(
                "nonexistent",
                &UpdateTaskRequest {
                    status: None,
                    result: None,
                    agent_id: None,
                },
            )
            .await
            .unwrap();
        assert!(!ok);
    }
}
