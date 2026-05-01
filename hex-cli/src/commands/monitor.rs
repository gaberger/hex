use clap::Parser;

/// Monitor command for hex-cli
#[derive(Parser, Debug)]
pub struct MonitorArgs {
    /// Query memory directly with a specified pattern
    #[clap(short, long)]
    pub query_memory: Option<String>,
}

impl MonitorArgs {
    pub fn run(&self) {
        if let Some(pattern) = &self.query_memory {
            println!("Querying memory with pattern: {}", pattern);
            // Memory search logic here
        } else {
            println!("Monitoring without specific memory query.");
        }
    }
}