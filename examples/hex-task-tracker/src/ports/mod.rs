use crate::domain::{DomainError, Priority, Task, TaskId};

pub trait TaskStore {
    fn save(&mut self, task: Task) -> Result<(), DomainError>;
    fn find(&self, id: &TaskId) -> Result<&Task, DomainError>;
    fn find_mut(&mut self, id: &TaskId) -> Result<&mut Task, DomainError>;
    fn list(&self) -> Vec<&Task>;
    fn remove(&mut self, id: &TaskId) -> Result<Task, DomainError>;
}

pub enum Command {
    Add { title: String, priority: Priority },
    List,
    Start { id: String },
    Done { id: String },
    Cancel { id: String },
    Remove { id: String },
}

pub fn parse_args(args: &[String]) -> Result<Command, String> {
    if args.len() < 2 { return Err(usage()); }
    match args[1].as_str() {
        "add" => {
            let title = args.get(2).ok_or("Usage: task add <title>")?.clone();
            let pri = args.iter().position(|a| a == "--priority")
                .and_then(|i| args.get(i + 1))
                .map(|p| match p.as_str() {
                    "low" => Priority::Low, "high" => Priority::High,
                    "critical" => Priority::Critical, _ => Priority::Medium,
                }).unwrap_or(Priority::Medium);
            Ok(Command::Add { title, priority: pri })
        }
        "list" => Ok(Command::List),
        "start" => Ok(Command::Start { id: args.get(2).ok_or("Usage: task start <id>")?.clone() }),
        "done" => Ok(Command::Done { id: args.get(2).ok_or("Usage: task done <id>")?.clone() }),
        "cancel" => Ok(Command::Cancel { id: args.get(2).ok_or("Usage: task cancel <id>")?.clone() }),
        "remove" => Ok(Command::Remove { id: args.get(2).ok_or("Usage: task remove <id>")?.clone() }),
        _ => Err(usage()),
    }
}

fn usage() -> String {
    "Usage: task <add|list|start|done|cancel|remove> [args]\n  add <title> [--priority low|medium|high|critical]\n  list\n  start|done|cancel|remove <id>".into()
}
