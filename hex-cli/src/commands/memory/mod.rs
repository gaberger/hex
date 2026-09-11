//! `hex memory` — the lessons the agent loop reads before it starts work.
//!
//! Backed by `~/.hex/memory.jsonl` through `hex_exec::local_store`
//! (ADR-2608241500). It was the `hexflo_memory` SpacetimeDB table, reached
//! through the daemon over HTTP — so writing down a lesson needed a database
//! and a control plane to be up.
//!
//! Key prefixes are conventional, not enforced: `lesson:` (don't repeat
//! this), `gap:` (known issue), `project:` (in-flight context), `decision:`
//! (a recorded choice). `hex-exec`'s context assembly reads the same feed, so
//! a lesson stored here reaches the next `hex do`.
//!
//! # What was removed
//!
//! `sync-check` and `validate` both existed to prove that two agents on two
//! hosts saw the same row through SpacetimeDB. There is one agent and one
//! machine now, so the property they asserted is not a property any more.

use clap::Subcommand;
use colored::Colorize;

use hex_exec::local_store;

#[derive(Subcommand)]
pub enum MemoryAction {
    /// Store a key-value pair
    Store {
        /// Key name, e.g. `lesson:trace-consumers`
        key: String,
        /// Value to store
        value: String,
    },
    /// Retrieve a value by key
    Get {
        /// Key name
        key: String,
    },
    /// Search stored memory by substring, over keys and values
    Search {
        /// Search query
        query: String,
    },
    /// List every stored entry
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Delete an entry by key
    Delete {
        /// Key name
        key: String,
    },
}

/// Entries listed by `hex memory list`.
const LIST_LIMIT: usize = 500;

pub async fn run(action: MemoryAction) -> anyhow::Result<()> {
    match action {
        MemoryAction::Store { key, value } => {
            local_store::memory_put(&key, &value).map_err(|e| anyhow::anyhow!(e))?;
            println!("{} Memory stored", "\u{2b21}".green());
            println!("  Key:   {}", key.bold());
            println!("  Value: {} bytes", value.len());
        }
        MemoryAction::Get { key } => match local_store::memory_get(&key) {
            Some(value) => {
                println!("{} Memory lookup", "\u{2b21}".cyan());
                println!("  Key:   {}", key.bold());
                println!("  Value: {}", value);
            }
            None => println!("{} Key '{}' not found", "\u{2b21}".yellow(), key),
        },
        MemoryAction::Search { query } => {
            let results = local_store::memory_search(&query);
            if results.is_empty() {
                println!("{} No results for '{}'", "\u{2b21}".dimmed(), query);
                return Ok(());
            }
            println!(
                "{} Memory search: '{}' ({} results)",
                "\u{2b21}".cyan(),
                query.bold(),
                results.len()
            );
            println!();
            for (key, value) in &results {
                println!("  {} {}", key.bold(), preview(value).dimmed());
            }
        }
        MemoryAction::List { json } => {
            let all = local_store::memory_entries(LIST_LIMIT);
            if json {
                let rows: Vec<_> = all
                    .iter()
                    .map(|(k, v)| serde_json::json!({ "key": k, "value": v }))
                    .collect();
                println!("{}", serde_json::to_string_pretty(&rows)?);
                return Ok(());
            }
            if all.is_empty() {
                println!("{} No memory entries yet", "\u{2b21}".dimmed());
                return Ok(());
            }
            println!("{} Memory ({} entries)", "\u{2b21}".cyan(), all.len());
            println!();
            for (key, value) in &all {
                println!("  {} {}", key.bold(), preview(value).dimmed());
            }
        }
        MemoryAction::Delete { key } => {
            if local_store::memory_delete(&key).map_err(|e| anyhow::anyhow!(e))? {
                println!("{} Deleted '{}'", "\u{2b21}".green(), key.bold());
            } else {
                println!("{} Key '{}' not found", "\u{2b21}".yellow(), key);
            }
        }
    }
    Ok(())
}

/// First line of a value, clipped, for list and search output.
fn preview(value: &str) -> String {
    let first = value.lines().next().unwrap_or("");
    if first.chars().count() > 60 {
        // Clip on a character boundary; values are arbitrary UTF-8.
        let clipped: String = first.chars().take(57).collect();
        format!("{clipped}...")
    } else {
        first.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_takes_the_first_line_only() {
        assert_eq!(preview("one\ntwo\nthree"), "one");
        assert_eq!(preview(""), "");
    }

    #[test]
    fn preview_clips_on_a_character_boundary() {
        // Slicing by byte index here would panic.
        let wide = "\u{e9}".repeat(100);
        let got = preview(&wide);
        assert!(got.ends_with("..."));
        assert_eq!(got.chars().count(), 60);
    }
}
