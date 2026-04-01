//! `hex chat` TUI — full-screen streaming chat (stub).
//!
//! This is a placeholder for P3.2. The real ratatui implementation
//! will replace this stub once the TUI layer is built out.

use anyhow::Result;

use crate::commands::chat::ChatArgs;

/// Launch the full-screen chat TUI.
///
/// TODO(P3.2): Implement ratatui streaming chat UI.
/// For now, callers should use `--no-tui` for functional operation.
pub async fn run(_args: ChatArgs) -> Result<()> {
    anyhow::bail!("TUI not yet implemented — use --no-tui flag")
}
