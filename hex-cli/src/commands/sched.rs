use clap::ArgMatches;
use crate::tasks::{Task, execute_task};

pub fn execute_brain_task(matches: &ArgMatches) {
    let task_type = matches.value_of("task").unwrap_or("default");

    match task_type {
        "arch-analysis" => execute_task(Task::ArchAnalysis),
        _ => println!("Unknown task type"),
    }
}