//! Clock injection seam. Live lease deadlines compare against the MONOTONIC
//! clock (`mono_ms`); persisted backoff/lease wall values use `now_ms`. [FIX-CLOCKSTEP]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub trait Clock: Send + Sync {
    /// Wall-clock ms since UNIX_EPOCH (persisted, for backoff/lease math).
    fn now_ms(&self) -> u64;
    /// Monotonic ms (for LIVE lease deadline comparisons).
    fn mono_ms(&self) -> u64;
}

/// Production clock backed by the OS.
pub struct SystemClock {
    origin: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl SystemClock {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn mono_ms(&self) -> u64 {
        self.origin.elapsed().as_millis() as u64
    }
}

/// Deterministic clock for tests. Wall and monotonic advance independently so
/// clock-step scenarios can be exercised.
pub struct ManualClock {
    wall: AtomicU64,
    mono: AtomicU64,
}

impl ManualClock {
    pub fn new(start_ms: u64) -> Arc<Self> {
        Arc::new(Self {
            wall: AtomicU64::new(start_ms),
            mono: AtomicU64::new(start_ms),
        })
    }

    /// Advance both wall and monotonic clocks by `ms`.
    pub fn advance(&self, ms: u64) {
        self.wall.fetch_add(ms, Ordering::SeqCst);
        self.mono.fetch_add(ms, Ordering::SeqCst);
    }

    /// Advance only the wall clock (simulate NTP jump).
    pub fn advance_wall(&self, ms: u64) {
        self.wall.fetch_add(ms, Ordering::SeqCst);
    }

    /// Advance only the monotonic clock.
    pub fn advance_mono(&self, ms: u64) {
        self.mono.fetch_add(ms, Ordering::SeqCst);
    }

    pub fn set_wall(&self, ms: u64) {
        self.wall.store(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.wall.load(Ordering::SeqCst)
    }
    fn mono_ms(&self) -> u64 {
        self.mono.load(Ordering::SeqCst)
    }
}
