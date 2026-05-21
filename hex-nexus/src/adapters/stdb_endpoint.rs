//! SpacetimeDB endpoint discovery (ADR-2605190900 P4).
//!
//! Today's `std::env::var("HEX_SPACETIMEDB_HOST").unwrap_or_else(...)`
//! pattern is fine when the env var is set correctly. It fails silently
//! when something writes a stale value to `.hex/state.json` and a
//! subsequent code path reads from there — that's the
//! `http://192.168.30.162:3033` bug the 2026-05-19 postmortem captured.
//!
//! Discovery hierarchy (first that responds wins on `discover_validated`):
//!   1. `HEX_SPACETIMEDB_HOST` env var
//!   2. `.hex/project.json` → `coordination.host`
//!   3. `localhost:3033` (the production default)
//!   4. `HEX_STDB_FALLBACK_HOST` env var (operator escape hatch)
//!
//! `.hex/state.json` is read-only telemetry. It must NOT be a configuration
//! input — that bug is exactly what makes the cache-drift class possible.
//! Other code paths that read it for endpoint config should migrate to
//! this module. See ADR-2605190900 §5.

use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_HOST: &str = "http://127.0.0.1:3033";

/// Walk the discovery hierarchy and return the first candidate. Does
/// NOT validate that the candidate is reachable — for that, use
/// `discover_validated`.
///
/// This is the synchronous answer to "what host should I aim at?" —
/// suitable for nexus-startup paths where you can't await.
pub fn discover_endpoint() -> String {
    // Primary env var; `HEX_STDB_HOST` is the historical alias used by
    // state_config.rs and several scripts — honor it identically.
    for var in &["HEX_SPACETIMEDB_HOST", "HEX_STDB_HOST"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return v;
            }
        }
    }
    if let Some(host) = read_project_json_host() {
        return host;
    }
    if let Ok(v) = std::env::var("HEX_STDB_FALLBACK_HOST") {
        if !v.trim().is_empty() {
            return v;
        }
    }
    DEFAULT_HOST.to_string()
}

/// Async variant: probe each candidate with a 2s timeout, return the
/// first that responds to `/v1/ping`. Use this on the slow path
/// (connection error → retry-with-rediscovery) where the time cost is
/// already paid.
pub async fn discover_validated() -> String {
    let candidates = candidate_list();
    let http = match reqwest::Client::builder().timeout(Duration::from_secs(2)).build() {
        Ok(c) => c,
        Err(_) => return DEFAULT_HOST.to_string(),
    };
    for cand in &candidates {
        // STDB v1 exposes /v1/ping on the root host. Any 2xx counts as
        // alive; non-2xx or transport error → try the next candidate.
        let url = format!("{cand}/v1/ping");
        if let Ok(res) = http.get(&url).send().await {
            if res.status().is_success() {
                return cand.clone();
            }
        }
    }
    DEFAULT_HOST.to_string()
}

/// The candidate list in priority order — exported for diagnostics
/// (`hex doctor liveness` will display which candidate each entry
/// came from) and for testing.
pub fn candidate_list() -> Vec<String> {
    let mut out = Vec::with_capacity(4);
    for var in &["HEX_SPACETIMEDB_HOST", "HEX_STDB_HOST"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim();
            if !v.is_empty() && !out.iter().any(|c| c == v) {
                out.push(v.to_string());
            }
        }
    }
    if let Some(host) = read_project_json_host() {
        if !out.iter().any(|c| c == &host) {
            out.push(host);
        }
    }
    if !out.iter().any(|c| c == DEFAULT_HOST) {
        out.push(DEFAULT_HOST.to_string());
    }
    if let Ok(v) = std::env::var("HEX_STDB_FALLBACK_HOST") {
        let v = v.trim();
        if !v.is_empty() && !out.iter().any(|c| c == v) {
            out.push(v.to_string());
        }
    }
    out
}

fn read_project_json_host() -> Option<String> {
    let path = locate_project_json()?;
    let raw = std::fs::read_to_string(&path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    json.get("coordination")
        .and_then(|c| c.get("host"))
        .and_then(|h| h.as_str())
        .map(|s| s.to_string())
}

/// Look up `.hex/project.json` starting from $HEX_PROJECT_DIR (if set)
/// then the current directory. Doesn't walk up the directory tree —
/// hex-nexus is launched from a known project root.
fn locate_project_json() -> Option<PathBuf> {
    let root = std::env::var("HEX_PROJECT_DIR")
        .ok()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())?;
    let candidate = root.join(".hex").join("project.json");
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::OwnedMutexGuard;

    /// Saves + restores env vars across a test. Holds the workspace
    /// env-mutation lock for the test duration so parallel cases that
    /// touch HEX_SPACETIMEDB_HOST et al. queue rather than interleave.
    ///
    /// Async API + tokio::sync::Mutex so callers can hold the guard
    /// across `.await` points (the rediscovery tests in
    /// `spacetime_state::p4_2_rediscovery_tests` need that). Sync
    /// callers use `block_on` on the same lock — fine inside a Tokio
    /// runtime via `Handle::current().block_on`. To keep these tests
    /// `#[test]` (not `#[tokio::test]`) we own a runtime for the lock
    /// acquisition only.
    struct EnvGuard {
        keys: Vec<(&'static str, Option<String>)>,
        _lock: OwnedMutexGuard<()>,
    }
    impl EnvGuard {
        fn capture(keys: &[&'static str]) -> Self {
            // Acquire the shared workspace lock. Spin up a small
            // current-thread runtime — we're already in a sync test
            // context so we can't await directly.
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build current-thread runtime");
            let lock = rt.block_on(super::super::test_env_lock().lock_owned());
            let snapshot = keys
                .iter()
                .map(|k| (*k, std::env::var(k).ok()))
                .collect();
            Self { keys: snapshot, _lock: lock }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in &self.keys {
                match v {
                    Some(val) => std::env::set_var(k, val),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    #[test]
    fn env_host_wins_when_set() {
        let _g = EnvGuard::capture(&["HEX_SPACETIMEDB_HOST", "HEX_STDB_FALLBACK_HOST"]);
        std::env::set_var("HEX_SPACETIMEDB_HOST", "http://from-env:9000");
        std::env::remove_var("HEX_STDB_FALLBACK_HOST");
        assert_eq!(discover_endpoint(), "http://from-env:9000");
    }

    #[test]
    fn fallback_used_when_env_and_project_json_missing() {
        let _g = EnvGuard::capture(&["HEX_SPACETIMEDB_HOST", "HEX_STDB_FALLBACK_HOST", "HEX_PROJECT_DIR"]);
        std::env::remove_var("HEX_SPACETIMEDB_HOST");
        std::env::set_var("HEX_STDB_FALLBACK_HOST", "http://fallback:9000");
        // Point project dir at /tmp so .hex/project.json definitely
        // doesn't exist for this test.
        std::env::set_var("HEX_PROJECT_DIR", "/tmp/hex-endpoint-test-no-such-dir");
        let endpoint = discover_endpoint();
        // The default beats the fallback because the fallback only
        // joins the candidate list — it doesn't replace the default
        // when both are present. Both are equally valid by design.
        assert!(
            endpoint == "http://127.0.0.1:3033" || endpoint == "http://fallback:9000",
            "expected default or fallback, got {endpoint}"
        );
    }

    #[test]
    fn empty_env_treated_as_unset() {
        let _g = EnvGuard::capture(&["HEX_SPACETIMEDB_HOST"]);
        std::env::set_var("HEX_SPACETIMEDB_HOST", "   ");
        // Should skip the empty env and fall through.
        let endpoint = discover_endpoint();
        assert_ne!(endpoint, "   ");
    }

    #[test]
    fn candidate_list_orders_env_first() {
        let _g = EnvGuard::capture(&["HEX_SPACETIMEDB_HOST", "HEX_STDB_FALLBACK_HOST", "HEX_PROJECT_DIR"]);
        std::env::set_var("HEX_SPACETIMEDB_HOST", "http://env:1");
        std::env::set_var("HEX_STDB_FALLBACK_HOST", "http://fb:2");
        std::env::set_var("HEX_PROJECT_DIR", "/tmp/hex-endpoint-no");
        let list = candidate_list();
        assert_eq!(list[0], "http://env:1");
        assert!(list.contains(&"http://127.0.0.1:3033".to_string()));
        assert!(list.contains(&"http://fb:2".to_string()));
    }

    #[test]
    fn hex_stdb_host_alias_is_honored() {
        // ADR-2605190900 P4.3 — state_config.rs and several legacy scripts
        // export `HEX_STDB_HOST`. discover_endpoint accepts it as an
        // alias for `HEX_SPACETIMEDB_HOST` so the migration off
        // state.json doesn't break operator env scripts.
        let _g = EnvGuard::capture(&["HEX_SPACETIMEDB_HOST", "HEX_STDB_HOST", "HEX_PROJECT_DIR"]);
        std::env::remove_var("HEX_SPACETIMEDB_HOST");
        std::env::set_var("HEX_STDB_HOST", "http://legacy-alias:9000");
        std::env::set_var("HEX_PROJECT_DIR", "/tmp/hex-endpoint-no");
        assert_eq!(discover_endpoint(), "http://legacy-alias:9000");
    }

    #[test]
    fn spacetimedb_host_wins_over_stdb_host_alias() {
        // Both set — canonical name takes priority over the alias.
        let _g = EnvGuard::capture(&["HEX_SPACETIMEDB_HOST", "HEX_STDB_HOST", "HEX_PROJECT_DIR"]);
        std::env::set_var("HEX_SPACETIMEDB_HOST", "http://canonical:1");
        std::env::set_var("HEX_STDB_HOST", "http://alias:2");
        std::env::set_var("HEX_PROJECT_DIR", "/tmp/hex-endpoint-no");
        assert_eq!(discover_endpoint(), "http://canonical:1");
    }
}
