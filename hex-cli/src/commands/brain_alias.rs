//! Hidden `brain` alias for the renamed `sched` subcommand (ADR-2604150000).
//!
//! `hex brain ...` forwards to `hex sched ...` for backward compatibility
//! with scripts, muscle memory, and pre-rename tooling. The subcommand is
//! hidden from `--help` output (see `#[command(hide = true)]` on the
//! top-level `Commands::Brain` variant) and emits a one-shot deprecation
//! warning on the first invocation per process.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::commands::sched::{self, BrainAction};

/// Tracks whether the deprecation warning has been emitted this process.
/// One-shot so piped invocations (`hex brain ...` in shell loops) don't
/// spam stderr.
static WARNED: AtomicBool = AtomicBool::new(false);

/// Forward a `hex brain <action>` invocation to `hex sched <action>`,
/// emitting a one-shot deprecation warning on the first call per process.
///
/// The `BrainAction` enum is shared with `sched` so the subcommand trees
/// stay in lockstep automatically — new `sched` actions are picked up by
/// the alias without code changes here.
pub async fn run(action: BrainAction) -> anyhow::Result<()> {
    if !WARNED.swap(true, Ordering::Relaxed) {
        eprintln!("warning: `hex brain` is deprecated; use `hex sched` instead");
    }
    sched::run(action).await
}
