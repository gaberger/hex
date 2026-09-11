//! `hex memory` — the lessons the agent loop reads before it starts work.
//!
//! Per ADR-2608241500 P3.3. Entries are Markdown files, one per key, under
//! `.hex/memory/` (tracked by git, so a lesson is reviewable in a diff and
//! reaches another machine with `git pull`) and `~/.hex/memory/` (this
//! machine, every project). They were rows in a SpacetimeDB table reached
//! through the daemon over HTTP.
//!
//! Key prefixes are conventional: `lesson:` (don't repeat this), `gap:`
//! (known issue), `project:` (in-flight context), `decision:` (a recorded
//! choice). `hex-exec`'s `memory_search` tool and its context assembly read
//! the same files.
//!
//! # What was removed
//!
//! `sync-check` and `validate` both existed to prove that two agents on two
//! hosts saw the same row through SpacetimeDB. There is one agent and one
//! machine now, so the property they asserted is not a property any more.

use clap::Subcommand;
use colored::Colorize;

use hex_core::ports::local_store::{ILocalStore, MemoryScope};
use hex_exec::store::FileStore;

#[derive(Subcommand)]
pub enum MemoryAction {
    /// Store a key-value pair
    Store {
        /// Key name, e.g. `lesson:trace-consumers`
        key: String,
        /// Value to store
        value: String,
        /// Store for every project on this machine instead of just this one
        #[arg(long)]
        global: bool,
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

pub async fn run(action: MemoryAction) -> anyhow::Result<()> {
    let store = FileStore::current();
    match action {
        MemoryAction::Store { key, value, global } => {
            let scope = if global { MemoryScope::Global } else { MemoryScope::Project };
            store.put_memory(&key, &value, scope)?;
            println!("{} Memory stored", "\u{2b21}".green());
            println!("  Key:   {}", key.bold());
            println!("  Scope: {}", scope.as_str());
            println!("  Value: {} bytes", value.len());
            Ok(())
        }
        MemoryAction::Get { key } => {
            match store.get_memory(&key)? {
                Some(entry) => {
                    println!("{} Memory lookup", "\u{2b21}".cyan());
                    println!("  Key:     {}", entry.key.bold());
                    println!("  Scope:   {}", entry.scope.as_str());
                    println!("  Updated: {}", entry.updated_at);
                    println!("  Value:   {}", entry.value);
                }
                None => println!("{} Key '{}' not found", "\u{2b21}".yellow(), key),
            }
            Ok(())
        }
        MemoryAction::Search { query } => {
            let results = store.search_memory(&query)?;
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
            for entry in &results {
                println!("  {} {}", entry.key.bold(), preview(&entry.value).dimmed());
            }
            Ok(())
        }
        MemoryAction::List { json } => {
            let all = store.list_memory()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&all)?);
                return Ok(());
            }
            if all.is_empty() {
                println!("{} No memory entries yet", "\u{2b21}".dimmed());
                return Ok(());
            }
            println!("{} Memory ({} entries)", "\u{2b21}".cyan(), all.len());
            println!();
            for entry in &all {
                println!(
                    "  {} [{}] {}",
                    entry.key.bold(),
                    entry.scope.as_str().dimmed(),
                    preview(&entry.value).dimmed()
                );
            }
            Ok(())
        }
        MemoryAction::Delete { key } => {
            if store.delete_memory(&key)? {
                println!("{} Deleted '{}'", "\u{2b21}".green(), key.bold());
            } else {
                println!("{} Key '{}' not found", "\u{2b21}".yellow(), key);
            }
            Ok(())
        }
    }
}

/// First line of a value, clipped, for list and search output.
fn preview(value: &str) -> String {
    let first = value.lines().next().unwrap_or("");
    if first.chars().count() > 60 {
        // Clip on a character boundary; keys and values are arbitrary UTF-8.
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
        // The old implementation sliced by byte index and would panic here.
        let wide = "\u{e9}".repeat(100);
        let got = preview(&wide);
        assert!(got.ends_with("..."));
        assert_eq!(got.chars().count(), 60);
    }
}
