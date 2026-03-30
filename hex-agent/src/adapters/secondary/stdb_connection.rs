//! StdbConnection — direct SpacetimeDB WebSocket client for hex-agent.
//!
//! Subscribes to the `code_gen_task` table in the `remote-agent-registry`
//! SpacetimeDB module. Maintains an in-memory cache of pending tasks and
//! exposes reducer calls (`assign_task`, `complete_task`) so Docker workers
//! can claim and complete tasks without REST polling.
//!
//! Falls back gracefully: if the connection fails or the feature is disabled,
//! `is_connected()` returns false and the caller switches to REST.

// ── Feature-gated implementation (real SpacetimeDB SDK) ──────────────────────

#[cfg(feature = "spacetimedb")]
mod real {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use tokio::sync::{Notify, RwLock};

    use hex_nexus::spacetime_bindings::remote_agent_registry::{
        CodeGenTask, CodeGenTaskTableAccess, DbConnection,
    };
    use spacetimedb_sdk::{DbContext, Table};

    /// Live WebSocket connection to the `remote-agent-registry` SpacetimeDB module.
    ///
    /// Created once at daemon startup. After `connect()` succeeds, the internal
    /// `run_threaded()` loop processes incoming WebSocket frames in a background OS
    /// thread, populating `task_cache` via `on_insert` callbacks. Async code waits
    /// on `task_notify` for efficient push-based wakeup.
    pub struct StdbConnection {
        /// All code_gen_task rows visible to this agent, keyed by task_id.
        task_cache: Arc<dashmap::DashMap<String, CodeGenTask>>,
        /// Fired when a new task enters the cache (on_insert callback → notify).
        task_notify: Arc<Notify>,
        /// True once the initial subscription rows have been applied.
        subscribed: Arc<AtomicBool>,
        /// The underlying connection handle, None until connect() succeeds.
        connection: Arc<RwLock<Option<DbConnection>>>,
        /// This agent's unique ID.
        agent_id: String,
    }

    impl StdbConnection {
        pub fn new(agent_id: String) -> Self {
            Self {
                task_cache: Arc::new(dashmap::DashMap::new()),
                task_notify: Arc::new(Notify::new()),
                subscribed: Arc::new(AtomicBool::new(false)),
                connection: Arc::new(RwLock::new(None)),
                agent_id,
            }
        }

        /// Attempt to connect to SpacetimeDB at `ws_url` for `database`.
        ///
        /// On success, starts the background WebSocket thread and waits up to 5s
        /// for the initial subscription to apply. On failure, logs a warning and
        /// returns `Ok(())` — callers check `is_connected()` to pick a path.
        pub async fn connect(
            &self,
            ws_url: &str,
            database: &str,
            token: Option<&str>,
        ) -> Result<(), String> {
            if ws_url.is_empty() || database.is_empty() {
                tracing::info!(
                    "StdbConnection: no URL/database configured, REST fallback active"
                );
                return Ok(());
            }

            let cache_insert = self.task_cache.clone();
            let cache_delete = self.task_cache.clone();
            let notify_insert = self.task_notify.clone();
            let subscribed_applied = self.subscribed.clone();
            let subscribed_disconnect = self.subscribed.clone();
            let my_agent_id = self.agent_id.clone();

            // TODO: token-based auth — the spacetimedb_sdk Credentials type differs
            // from &str. For now, auth is handled at the nexus layer. Pass token
            // via env SPACETIMEDB_TOKEN for future use.
            let _ = token;

            let build_result = DbConnection::builder()
                .with_uri(ws_url)
                .with_database_name(database)
                .on_connect(move |conn, _identity, _token| {
                    // Cache all task rows: pending ones + anything assigned to this agent
                    conn.db().code_gen_task().on_insert(move |_ctx, row| {
                        if row.status == "pending" || row.assigned_agent_id == my_agent_id {
                            tracing::debug!(
                                task_id = %row.task_id,
                                status = %row.status,
                                "StdbConnection: code_gen_task inserted"
                            );
                            cache_insert.insert(row.task_id.clone(), row.clone());
                            notify_insert.notify_one();
                        }
                    });

                    conn.db().code_gen_task().on_delete(move |_ctx, row| {
                        tracing::debug!(task_id = %row.task_id, "StdbConnection: code_gen_task deleted");
                        cache_delete.remove(&row.task_id);
                    });

                    conn.subscription_builder()
                        .on_applied(move |_ctx| {
                            tracing::info!("StdbConnection: subscription applied — task cache live");
                            subscribed_applied.store(true, Ordering::Release);
                        })
                        .on_error(|_ctx, err| {
                            tracing::error!(?err, "StdbConnection: subscription error");
                        })
                        .subscribe(["SELECT * FROM code_gen_task"]);
                })
                .on_connect_error(|_ctx, err| {
                    tracing::warn!(?err, "StdbConnection: connect error, REST fallback active");
                })
                .on_disconnect(move |_ctx, err| {
                    if let Some(e) = err {
                        tracing::warn!(?e, "StdbConnection: disconnected with error");
                    } else {
                        tracing::info!("StdbConnection: disconnected cleanly");
                    }
                    // Reset subscribed so is_connected() returns false on a dead connection,
                    // preventing the task poller from retrying the StDB path indefinitely.
                    subscribed_disconnect.store(false, Ordering::Release);
                })
                .build();

            match build_result {
                Ok(conn) => {
                    // Start the background WebSocket processing thread
                    conn.run_threaded();

                    // Wait up to 5s for initial rows
                    let deadline =
                        tokio::time::Instant::now() + tokio::time::Duration::from_secs(5);
                    while tokio::time::Instant::now() < deadline {
                        if self.subscribed.load(Ordering::Acquire) {
                            break;
                        }
                        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                    }

                    let ready = self.subscribed.load(Ordering::Acquire);
                    tracing::info!(subscription_ready = ready, "StdbConnection: connect complete");

                    *self.connection.write().await = Some(conn);
                    Ok(())
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "StdbConnection: build failed, REST fallback active"
                    );
                    // Not a fatal error — callers fall back to REST
                    Ok(())
                }
            }
        }

        /// True once the initial subscription has been applied and the cache is live.
        pub fn is_connected(&self) -> bool {
            self.subscribed.load(Ordering::Acquire)
        }

        /// Return all `pending` tasks from the cache (not yet claimed by any agent).
        pub fn pending_tasks(&self) -> Vec<CodeGenTask> {
            self.task_cache
                .iter()
                .filter(|e| e.value().status == "pending")
                .map(|e| e.value().clone())
                .collect()
        }

        /// Wait until a new task notification arrives (non-blocking if tasks already pending).
        pub async fn wait_for_task(&self) {
            self.task_notify.notified().await;
        }

        /// Remove a task from the local cache (called after claiming it).
        pub fn evict(&self, task_id: &str) {
            self.task_cache.remove(task_id);
        }

        /// Call the `assign_task` reducer to claim `task_id` for this agent.
        pub async fn assign_task(&self, task_id: &str) -> Result<(), String> {
            let conn = self.connection.read().await;
            match conn.as_ref() {
                Some(c) => {
                    use hex_nexus::spacetime_bindings::remote_agent_registry::assign_task;
                    c.reducers
                        .assign_task(task_id.to_string(), self.agent_id.clone())
                        .map_err(|e| e.to_string())
                }
                None => Err("StdbConnection: not connected".to_string()),
            }
        }

        /// Call the `complete_task` reducer to mark `task_id` done with JSON result.
        pub async fn complete_task(&self, task_id: &str, result_json: &str) -> Result<(), String> {
            let conn = self.connection.read().await;
            match conn.as_ref() {
                Some(c) => {
                    use hex_nexus::spacetime_bindings::remote_agent_registry::complete_task;
                    c.reducers
                        .complete_task(task_id.to_string(), result_json.to_string())
                        .map_err(|e| e.to_string())
                }
                None => Err("StdbConnection: not connected".to_string()),
            }
        }
    }
}

// ── Stub implementation (spacetimedb feature disabled) ────────────────────────

#[cfg(not(feature = "spacetimedb"))]
mod stub {
    /// No-op stub — used when the `spacetimedb` feature is not compiled in.
    /// All operations return immediately; `is_connected()` always returns false
    /// so callers use REST polling.
    pub struct StdbConnection;

    impl StdbConnection {
        pub fn new(_agent_id: String) -> Self {
            Self
        }

        pub async fn connect(
            &self,
            _ws_url: &str,
            _database: &str,
            _token: Option<&str>,
        ) -> Result<(), String> {
            Ok(())
        }

        pub fn is_connected(&self) -> bool {
            false
        }
    }
}

#[cfg(feature = "spacetimedb")]
pub use real::StdbConnection;
#[cfg(not(feature = "spacetimedb"))]
pub use stub::StdbConnection;
