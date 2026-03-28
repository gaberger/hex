use crate::ports::cli::CliCommand;

pub struct Cli {
    command: CliCommand,
}

impl Cli {
    pub fn new(command: CliCommand) -> Self {
        Self { command }
    }

    pub fn run(&self) -> anyhow::Result<()> {
        self.command.execute()
    }
}