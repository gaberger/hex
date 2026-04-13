// src/ports/mod.rs

use crate::domain::{Task, TaskId}; // Ensure this path matches your project structure

pub trait TaskStore {
    fn save(&mut self, task: Task);
    fn find(&self, id: &TaskId) -> Option<&Task>;
    fn find_mut(&mut self, id: &TaskId) -> Option<&mut Task>;
    fn list(&self) -> Vec<&Task>;
    fn remove(&mut self, id: &TaskId) -> bool;
}

pub enum Command {
    Add(Task),
    List,
    Start(TaskId),
    Done(TaskId),
    Cancel(TaskId),
    Remove(TaskId),
}

pub fn parse_args(args: &[String]) -> Option<Command> {
    if args.is_empty() {
        return None;
    }

    match args[0].as_str() {
        "add" => {
            // Assuming the task description follows the command
            let description = args.get(1)?.clone();
            Some(Command::Add(Task::new(description)))
        }
        "list" => Some(Command::List),
        "start" | "done" | "cancel" | "remove" => {
            // Assuming the TaskId follows the command
            let task_id_str = args.get(1)?;
            let task_id: TaskId = task_id_str.parse().ok()?;
            match args[0].as_str() {
                "start" => Some(Command::Start(task_id)),
                "done" => Some(Command::Done(task_id)),
                "cancel" => Some(Command::Cancel(task_id)),
                "remove" => Some(Command::Remove(task_id)),
                _ => None,
            }
        }
        _ => None,
    }
}
