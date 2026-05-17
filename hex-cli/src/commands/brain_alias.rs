//! Hidden `brain` alias for the renamed `sched` subcommand (ADR-2026-04-15-0000).
//!
//! `hex brain ...` forwards to `hex sched ...` silently. The subcommand is
//! hidden from `--help` output (see `#[command(hide = true)]` on the
//! top-level `Commands::Brain` variant). The `BrainAction` enum is shared
//! with `sched` so the subcommand trees stay in lockstep automatically.

use crate::commands::sched::{self, BrainAction};

pub async fn run(action: BrainAction) -> anyhow::Result<()> {
    sched::run(action).await
}
