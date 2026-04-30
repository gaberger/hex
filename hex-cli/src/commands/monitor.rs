use clap::Parser;

/// Monitor command for hex-cli
#[derive(Parser, Debug)]
pub struct MonitorCommand {
    /// Query memory directly with a specified pattern
    #[clap(short, long)]
    query_memory: Option<String>,
}

impl MonitorCommand {
    pub fn run(&self) {
        if let Some(query) = &self.query_memory {
            println!("Querying memory with pattern: {}", query);
        } else {
            println!("No memory query specified.");
        }
    }
}