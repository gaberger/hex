//! Shared test helpers.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use jobqueue_clean::{Config, FsyncMode};

static DIR_CTR: AtomicU64 = AtomicU64::new(0);

/// A unique temp dir under the system temp, cleaned up on drop.
pub struct TempDir {
    pub path: PathBuf,
}

impl TempDir {
    pub fn new() -> Self {
        let n = DIR_CTR.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let mut path = std::env::temp_dir();
        path.push(format!("jobqueue-clean-test-{pid}-{n}"));
        std::fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A deterministic test config: no jitter, small timeouts.
pub fn test_cfg() -> Config {
    Config {
        visibility_timeout: Duration::from_millis(1000),
        base_backoff: Duration::from_millis(1000),
        max_backoff: Duration::from_millis(60_000),
        default_max_attempts: 5,
        jitter: false,
        fsync_mode: FsyncMode::PerOp,
        compact_threshold_records: 0,
    }
}
